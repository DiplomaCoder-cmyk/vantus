use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use futures_util::FutureExt;
use serde::Serialize;

use crate::app::application::HostContext;
use crate::app::module::{Module, RuntimeModule};
use crate::core::errors::{FrameworkError, HttpError};
use crate::core::http::{Method, Response};
use crate::logging::{LogLevel, RequestLogEvent, redact_headers, sanitize_path_for_logs};
use crate::middleware::{Middleware, MiddlewareStage, Next};
use crate::routing::{RequestContext, RouteDefinition, RouteRegistrar};

pub struct RequestLogger;

impl Middleware for RequestLogger {
    fn stage(&self) -> MiddlewareStage {
        MiddlewareStage::Logging
    }

    fn handle(&self, ctx: RequestContext, next: Next) -> crate::routing::HandlerFuture {
        Box::pin(async move {
            let started = std::time::Instant::now();
            let event = RequestLogEvent {
                request_id: None,
                method: ctx.request().method.to_string(),
                path: sanitize_path_for_logs(&ctx.request().path),
                status_code: 0,
                duration_ms: 0,
                client_ip: ctx
                    .request()
                    .client_ip(&ctx.app_config().server.trusted_proxies)
                    .map(|ip| ip.to_string()),
                headers: redact_headers(&ctx.request().headers),
            };

            let result = next.run(ctx.clone()).await;
            let mut event = event;
            event.status_code = status_code_for_result(&result);
            event.duration_ms = started.elapsed().as_millis();
            ctx.log_sink().log_request("vantus.web", &event);
            result
        })
    }
}

pub struct SecurityHeaders;

impl Middleware for SecurityHeaders {
    fn stage(&self) -> MiddlewareStage {
        MiddlewareStage::Response
    }

    fn handle(&self, ctx: RequestContext, next: Next) -> crate::routing::HandlerFuture {
        Box::pin(async move {
            let response = next.run(ctx).await?;
            Ok(response
                .with_header("X-Content-Type-Options", "nosniff")
                .with_header("X-Frame-Options", "DENY")
                .with_header("Referrer-Policy", "no-referrer")
                .with_header("Cache-Control", "no-store"))
        })
    }
}

pub struct PanicRecovery;

impl Middleware for PanicRecovery {
    fn stage(&self) -> MiddlewareStage {
        MiddlewareStage::Recovery
    }

    fn handle(&self, ctx: RequestContext, next: Next) -> crate::routing::HandlerFuture {
        Box::pin(async move {
            match std::panic::AssertUnwindSafe(next.run(ctx.clone()))
                .catch_unwind()
                .await
            {
                Ok(result) => result,
                Err(_) => {
                    ctx.log_sink().log_text(
                        LogLevel::Error,
                        "vantus.web",
                        "request handling panicked",
                    );
                    Ok(Response::internal_server_error())
                }
            }
        })
    }
}

#[derive(Clone)]
pub struct HealthModule {
    ready: Arc<AtomicBool>,
}

impl Default for HealthModule {
    fn default() -> Self {
        Self {
            ready: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl Module for HealthModule {
    fn configure_routes(&self, routes: &mut dyn RouteRegistrar) -> Result<(), FrameworkError> {
        let ready = Arc::clone(&self.ready);
        routes.add_route(RouteDefinition::new(
            Method::Get,
            "/health",
            crate::routing::Handler::new(move |_| {
                let ready = Arc::clone(&ready);
                async move {
                    #[derive(Serialize)]
                    struct HealthPayload<'a> {
                        status: &'a str,
                    }

                    let status = if ready.load(Ordering::SeqCst) {
                        "ok"
                    } else {
                        "starting"
                    };
                    Response::json_serialized(&HealthPayload { status })
                        .map_err(|_| FrameworkError::internal("response serialization failed"))
                }
            }),
        ))
    }
}

#[async_trait]
impl RuntimeModule for HealthModule {
    async fn on_start(&self, host: &HostContext) -> Result<(), FrameworkError> {
        self.ready
            .store(host.app_config().readiness, Ordering::SeqCst);
        Ok(())
    }

    async fn on_stop(&self, _host: &HostContext) -> Result<(), FrameworkError> {
        self.ready.store(false, Ordering::SeqCst);
        Ok(())
    }
}

#[derive(Clone)]
pub struct InfoModule {
    enabled: Arc<AtomicBool>,
}

impl Default for InfoModule {
    fn default() -> Self {
        Self {
            enabled: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl Module for InfoModule {
    fn configure_routes(&self, routes: &mut dyn RouteRegistrar) -> Result<(), FrameworkError> {
        let enabled = Arc::clone(&self.enabled);
        routes.add_route(RouteDefinition::new(
            Method::Get,
            "/info",
            crate::routing::Handler::new(move |ctx| {
                let enabled = Arc::clone(&enabled);
                async move {
                    #[derive(Serialize)]
                    struct InfoPayload {
                        service: String,
                        path: String,
                        time: u64,
                    }

                    if !enabled.load(Ordering::SeqCst) {
                        return Err(HttpError::not_found("404 Not Found").into());
                    }

                    let payload = InfoPayload {
                        service: ctx.app_config().service_name.clone(),
                        path: ctx.request().path.clone(),
                        time: SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .map(|duration| duration.as_secs())
                            .unwrap_or(0),
                    };
                    Response::json_serialized(&payload)
                        .map_err(|_| FrameworkError::internal("response serialization failed"))
                }
            }),
        ))
    }
}

#[async_trait]
impl RuntimeModule for InfoModule {
    async fn on_start(&self, host: &HostContext) -> Result<(), FrameworkError> {
        self.enabled
            .store(host.app_config().enable_info, Ordering::SeqCst);
        Ok(())
    }

    async fn on_stop(&self, _host: &HostContext) -> Result<(), FrameworkError> {
        self.enabled.store(false, Ordering::SeqCst);
        Ok(())
    }
}

pub struct WebPlatformModule {
    health: HealthModule,
    info: InfoModule,
}

impl WebPlatformModule {
    pub fn new() -> Self {
        Self {
            health: HealthModule::default(),
            info: InfoModule::default(),
        }
    }
}

impl Default for WebPlatformModule {
    fn default() -> Self {
        Self::new()
    }
}

impl Module for WebPlatformModule {
    fn configure_middleware(&self, middleware: &mut crate::middleware::MiddlewareStack) {
        middleware.add(RequestLogger);
        middleware.add(PanicRecovery);
        middleware.add(SecurityHeaders);
    }

    fn configure_routes(&self, routes: &mut dyn RouteRegistrar) -> Result<(), FrameworkError> {
        self.health.configure_routes(routes)?;
        self.info.configure_routes(routes)?;
        Ok(())
    }
}

#[async_trait]
impl RuntimeModule for WebPlatformModule {
    async fn on_start(&self, host: &HostContext) -> Result<(), FrameworkError> {
        self.health.on_start(host).await?;
        self.info.on_start(host).await
    }

    async fn on_stop(&self, host: &HostContext) -> Result<(), FrameworkError> {
        self.info.on_stop(host).await?;
        self.health.on_stop(host).await
    }
}

fn status_code_for_result(result: &crate::routing::HandlerResult) -> u16 {
    match result {
        Ok(response) => response.status_code,
        Err(FrameworkError::Http(error)) => error.status_code,
        Err(_) => 500,
    }
}
