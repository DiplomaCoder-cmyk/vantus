use std::fmt;
use std::fmt::Write as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use async_trait::async_trait;
use dashmap::DashMap;
use serde::Serialize;
use tokio::sync::RwLock;

use crate::app::application::HostContext;
use crate::app::module::{Module, RuntimeModule};
use crate::core::errors::{FrameworkError, HttpError};
use crate::core::http::{Method, Response};
use crate::logging::{LogLevel, RequestLogEvent, sanitize_path_for_logs};
use crate::middleware::{Middleware, MiddlewareStage, Next};
use crate::routing::{RequestContext, RouteDefinition, RouteRegistrar};

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RequestId(String);

impl RequestId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RequestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ReadinessCheck {
    pub name: String,
    pub healthy: bool,
    pub detail: String,
}

impl ReadinessCheck {
    pub fn healthy(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            healthy: true,
            detail: detail.into(),
        }
    }

    pub fn unhealthy(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            healthy: false,
            detail: detail.into(),
        }
    }
}

#[async_trait]
pub trait ReadinessContributor: Send + Sync {
    async fn check(&self) -> ReadinessCheck;
}

#[derive(Default)]
pub struct ReadinessRegistry {
    contributors: RwLock<Vec<Arc<dyn ReadinessContributor>>>,
}

impl ReadinessRegistry {
    pub async fn register(&self, contributor: Arc<dyn ReadinessContributor>) {
        self.contributors.write().await.push(contributor);
    }

    pub async fn run_checks(&self) -> Vec<ReadinessCheck> {
        let contributors = self.contributors.read().await.clone();
        let mut checks = Vec::with_capacity(contributors.len());
        for contributor in contributors {
            checks.push(contributor.check().await);
        }
        checks
    }

    pub async fn contributor_count(&self) -> usize {
        self.contributors.read().await.len()
    }
}

#[derive(Default)]
struct ObservabilityMetrics {
    in_flight_requests: AtomicUsize,
    requests_total: AtomicU64,
    errors_4xx_total: AtomicU64,
    errors_5xx_total: AtomicU64,
    route_metrics: DashMap<RequestMetricKey, RequestMetricValue>,
}

impl ObservabilityMetrics {
    fn begin_request(&self) {
        self.in_flight_requests.fetch_add(1, Ordering::SeqCst);
    }

    fn finish_request(&self, route: &str, method: &Method, status_code: u16, duration_ms: u64) {
        self.in_flight_requests.fetch_sub(1, Ordering::SeqCst);
        self.requests_total.fetch_add(1, Ordering::SeqCst);
        if (400..500).contains(&status_code) {
            self.errors_4xx_total.fetch_add(1, Ordering::SeqCst);
        }
        if status_code >= 500 {
            self.errors_5xx_total.fetch_add(1, Ordering::SeqCst);
        }

        let key = RequestMetricKey {
            method: method.to_string(),
            route: route.to_string(),
            status: status_code.to_string(),
        };
        let entry = self.route_metrics.entry(key).or_default();
        entry.count.fetch_add(1, Ordering::SeqCst);
        entry
            .duration_ms_total
            .fetch_add(duration_ms, Ordering::SeqCst);
    }

    fn render_prometheus(&self, snapshot: &crate::runtime::RuntimeSnapshot) -> String {
        let mut output = String::new();
        let _ = writeln!(output, "# TYPE vantus_requests_total counter");
        let _ = writeln!(
            output,
            "vantus_requests_total {}",
            self.requests_total.load(Ordering::SeqCst)
        );
        let _ = writeln!(output, "# TYPE vantus_request_errors_4xx_total counter");
        let _ = writeln!(
            output,
            "vantus_request_errors_4xx_total {}",
            self.errors_4xx_total.load(Ordering::SeqCst)
        );
        let _ = writeln!(output, "# TYPE vantus_request_errors_5xx_total counter");
        let _ = writeln!(
            output,
            "vantus_request_errors_5xx_total {}",
            self.errors_5xx_total.load(Ordering::SeqCst)
        );
        let _ = writeln!(output, "# TYPE vantus_in_flight_requests gauge");
        let _ = writeln!(
            output,
            "vantus_in_flight_requests {}",
            self.in_flight_requests.load(Ordering::SeqCst)
        );
        let _ = writeln!(output, "# TYPE vantus_route_requests_total counter");
        let _ = writeln!(
            output,
            "# TYPE vantus_route_request_duration_ms_total counter"
        );

        for entry in self.route_metrics.iter() {
            let key = entry.key();
            let value = entry.value();
            let labels = format!(
                "method=\"{}\",route=\"{}\",status=\"{}\"",
                escape_metric_label(&key.method),
                escape_metric_label(&key.route),
                escape_metric_label(&key.status),
            );
            let _ = writeln!(
                output,
                "vantus_route_requests_total{{{labels}}} {}",
                value.count.load(Ordering::SeqCst)
            );
            let _ = writeln!(
                output,
                "vantus_route_request_duration_ms_total{{{labels}}} {}",
                value.duration_ms_total.load(Ordering::SeqCst)
            );
        }

        let _ = writeln!(
            output,
            "vantus_runtime_total_requests {}",
            snapshot.total_requests
        );
        let _ = writeln!(
            output,
            "vantus_runtime_active_requests {}",
            snapshot.active_requests
        );
        let _ = writeln!(
            output,
            "vantus_runtime_active_connections {}",
            snapshot.active_connections
        );
        let _ = writeln!(
            output,
            "vantus_runtime_request_timeout_total {}",
            snapshot.request_timeout_total
        );
        let _ = writeln!(
            output,
            "vantus_runtime_read_timeout_total {}",
            snapshot.read_timeout_total
        );
        let _ = writeln!(
            output,
            "vantus_runtime_handler_timeout_total {}",
            snapshot.handler_timeout_total
        );
        let _ = writeln!(
            output,
            "vantus_runtime_rate_limit_rejections_total {}",
            snapshot.rate_limit_rejections_total
        );
        let _ = writeln!(
            output,
            "vantus_runtime_method_not_allowed_total {}",
            snapshot.method_not_allowed_total
        );
        let _ = writeln!(
            output,
            "vantus_runtime_content_type_rejections_total {}",
            snapshot.content_type_rejections_total
        );
        let _ = writeln!(
            output,
            "vantus_runtime_body_rejections_total {}",
            snapshot.body_rejections_total
        );
        output
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct RequestMetricKey {
    method: String,
    route: String,
    status: String,
}

#[derive(Default)]
struct RequestMetricValue {
    count: AtomicU64,
    duration_ms_total: AtomicU64,
}

#[derive(Clone)]
struct RequestIdMiddleware;

impl Middleware for RequestIdMiddleware {
    fn stage(&self) -> MiddlewareStage {
        MiddlewareStage::Logging
    }

    fn handle(&self, ctx: RequestContext, next: Next) -> crate::routing::HandlerFuture {
        Box::pin(async move {
            let request_id = RequestId(ctx.id_generator().next_id());
            ctx.insert_state(request_id.clone());
            let response = next.run(ctx).await?;
            Ok(response.with_header("X-Request-Id", request_id.to_string()))
        })
    }
}

#[derive(Clone)]
struct StructuredLoggingMiddleware;

impl Middleware for StructuredLoggingMiddleware {
    fn stage(&self) -> MiddlewareStage {
        MiddlewareStage::Logging
    }

    fn handle(&self, ctx: RequestContext, next: Next) -> crate::routing::HandlerFuture {
        Box::pin(async move {
            let started = std::time::Instant::now();
            let result = next.run(ctx.clone()).await;

            let request_id = match &result {
                Ok(response) => response
                    .headers
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("X-Request-Id"))
                    .map(|(_, value)| value.clone())
                    .or_else(|| ctx.state::<RequestId>().map(|id| id.to_string())),
                Err(_) => ctx.state::<RequestId>().map(|id| id.to_string()),
            };

            let event = RequestLogEvent {
                request_id,
                method: ctx.request().method.to_string(),
                path: sanitize_path_for_logs(&ctx.request().path),
                status_code: status_code_for_result(&result),
                duration_ms: started.elapsed().as_millis(),
                client_ip: ctx
                    .request()
                    .client_ip(&ctx.app_config().server.trusted_proxies)
                    .map(|ip| ip.to_string()),
                headers: Vec::new(),
            };

            ctx.log_sink().log_request("vantus.observability", &event);
            result
        })
    }
}

#[derive(Clone)]
struct MetricsMiddleware {
    metrics: Arc<ObservabilityMetrics>,
}

impl MetricsMiddleware {
    fn new(metrics: Arc<ObservabilityMetrics>) -> Self {
        Self { metrics }
    }
}

impl Middleware for MetricsMiddleware {
    fn stage(&self) -> MiddlewareStage {
        MiddlewareStage::Logging
    }

    fn handle(&self, ctx: RequestContext, next: Next) -> crate::routing::HandlerFuture {
        let metrics = Arc::clone(&self.metrics);
        Box::pin(async move {
            metrics.begin_request();
            let started = std::time::Instant::now();
            let result = next.run(ctx.clone()).await;
            metrics.finish_request(
                ctx.route_pattern(),
                &ctx.request().method,
                status_code_for_result(&result),
                started.elapsed().as_millis() as u64,
            );
            result
        })
    }
}

#[derive(Clone)]
struct ObservabilityState {
    live: Arc<AtomicBool>,
    readiness: Arc<ReadinessRegistry>,
    metrics: Arc<ObservabilityMetrics>,
}

impl Default for ObservabilityState {
    fn default() -> Self {
        Self {
            live: Arc::new(AtomicBool::new(false)),
            readiness: Arc::new(ReadinessRegistry::default()),
            metrics: Arc::new(ObservabilityMetrics::default()),
        }
    }
}

#[derive(Clone, Default)]
pub struct ObservabilityModule {
    state: ObservabilityState,
}

impl ObservabilityModule {
    pub fn readiness_registry(&self) -> Arc<ReadinessRegistry> {
        Arc::clone(&self.state.readiness)
    }
}

impl Module for ObservabilityModule {
    fn configure_middleware(&self, middleware: &mut crate::middleware::MiddlewareStack) {
        middleware.add(MetricsMiddleware::new(Arc::clone(&self.state.metrics)));
        middleware.add(RequestIdMiddleware);
        middleware.add(StructuredLoggingMiddleware);
    }

    fn configure_routes(&self, routes: &mut dyn RouteRegistrar) -> Result<(), FrameworkError> {
        let live = Arc::clone(&self.state.live);
        routes.add_route(RouteDefinition::new(
            Method::Get,
            "/live",
            crate::routing::Handler::new(move |_| {
                let live = Arc::clone(&live);
                async move {
                    let status = if live.load(Ordering::SeqCst) {
                        "ok"
                    } else {
                        "stopped"
                    };
                    Ok(Response::json_value(
                        serde_json::json!({ "status": status }),
                    ))
                }
            }),
        ))?;

        let live = Arc::clone(&self.state.live);
        let readiness = Arc::clone(&self.state.readiness);
        routes.add_route(RouteDefinition::new(
            Method::Get,
            "/ready",
            crate::routing::Handler::new(move |_| {
                let live = Arc::clone(&live);
                let readiness = Arc::clone(&readiness);
                async move {
                    if !live.load(Ordering::SeqCst) {
                        return Err(HttpError::new(
                            503,
                            "Service Unavailable",
                            "service is stopping",
                        )
                        .into());
                    }

                    let checks = readiness.run_checks().await;
                    let healthy = checks.iter().all(|check| check.healthy);
                    let response = Response::json_value(serde_json::json!({
                        "status": if healthy { "ok" } else { "degraded" },
                        "checks": checks,
                    }));
                    if healthy {
                        Ok(response)
                    } else {
                        Ok(response.with_header("X-Readiness-State", "degraded"))
                    }
                }
            }),
        ))?;

        let readiness = Arc::clone(&self.state.readiness);
        routes.add_route(RouteDefinition::new(
            Method::Get,
            "/diag",
            crate::routing::Handler::new(move |ctx| {
                let readiness = Arc::clone(&readiness);
                async move {
                    let contributor_count = readiness.contributor_count().await;
                    Ok(Response::json_value(serde_json::json!({
                        "service": ctx.app_config().service_name,
                        "environment": ctx.app_config().environment,
                        "profile": ctx.app_config().profile,
                        "runtime": ctx.runtime_state().snapshot(),
                        "readiness_contributors": contributor_count,
                    })))
                }
            }),
        ))?;

        let metrics = Arc::clone(&self.state.metrics);
        routes.add_route(RouteDefinition::new(
            Method::Get,
            "/metrics",
            crate::routing::Handler::new(move |ctx| {
                let metrics = Arc::clone(&metrics);
                async move {
                    let body = metrics.render_prometheus(&ctx.runtime_state().snapshot());
                    Ok(Response::text(body)
                        .with_header("Content-Type", "text/plain; version=0.0.4; charset=utf-8"))
                }
            }),
        ))?;

        Ok(())
    }
}

#[async_trait]
impl RuntimeModule for ObservabilityModule {
    async fn on_start(&self, host: &HostContext) -> Result<(), FrameworkError> {
        self.state.live.store(true, Ordering::SeqCst);
        host.log_sink().log_text(
            LogLevel::Info,
            "vantus.observability",
            "observability module started",
        );
        Ok(())
    }

    async fn on_stop(&self, host: &HostContext) -> Result<(), FrameworkError> {
        self.state.live.store(false, Ordering::SeqCst);
        host.log_sink().log_text(
            LogLevel::Info,
            "vantus.observability",
            "observability module stopped",
        );
        Ok(())
    }
}

fn status_code_for_result(result: &crate::routing::HandlerResult) -> u16 {
    match result {
        Ok(response) => response.status_code,
        Err(FrameworkError::Http(error)) => error.status_code,
        Err(_) => 500,
    }
}

fn escape_metric_label(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}
