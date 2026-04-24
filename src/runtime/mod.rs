use std::convert::Infallible;
use std::future::Future;
use std::net::IpAddr;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use dashmap::DashMap;
use http::HeaderMap;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::{Request as HyperRequest, Response as HyperResponse, Version};
use hyper_util::rt::{TokioExecutor, TokioIo};
use serde::Serialize;
use tokio::net::TcpListener;
use tokio::sync::{Semaphore, TryAcquireError};
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::timeout;
use tracing::{Instrument, Span, info_span};

use crate::app::{HostContext, HostState, RuntimeModule};
use crate::config::{AppConfig, ServerOptions, ServerProtocol};
use crate::core::{FrameworkError, HttpError, Method, Request, Response};
use crate::middleware::MiddlewareStack;
use crate::routing::{
    RequestBodyKind, RequestContext, RouteContract, RouteResolution, Router, normalize_request_path,
};
use crate::{HostBuildError, HostError};

#[derive(Clone, Default)]
struct ConnectionTracker {
    tasks: Arc<tokio::sync::Mutex<JoinSet<()>>>,
}

impl ConnectionTracker {
    async fn spawn<F>(&self, future: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let mut tasks = self.tasks.lock().await;
        tasks.spawn(future);
        while tasks.try_join_next().is_some() {}
    }

    async fn shutdown(&self, grace_period: std::time::Duration) {
        let mut tasks = self.tasks.lock().await;
        if tasks.is_empty() {
            return;
        }

        let waiter = async { while tasks.join_next().await.is_some() {} };

        if timeout(grace_period, waiter).await.is_err() {
            tasks.abort_all();
            while tasks.join_next().await.is_some() {
                // Drain aborted tasks.
            }
        }
    }
}

pub struct RuntimeState {
    started_at_unix: AtomicU64,
    concurrency_limit: AtomicUsize,
    active_connections: AtomicUsize,
    active_requests: AtomicUsize,
    total_requests: AtomicU64,
    limiter_saturated_total: AtomicU64,
    rate_limit_rejections_total: AtomicU64,
    request_timeout_total: AtomicU64,
    read_timeout_total: AtomicU64,
    handler_timeout_total: AtomicU64,
    method_not_allowed_total: AtomicU64,
    content_type_rejections_total: AtomicU64,
    body_rejections_total: AtomicU64,
}

impl RuntimeState {
    pub fn snapshot(&self) -> RuntimeSnapshot {
        RuntimeSnapshot {
            started_at_unix: self.started_at_unix.load(Ordering::SeqCst),
            concurrency_limit: self.concurrency_limit.load(Ordering::SeqCst),
            active_connections: self.active_connections.load(Ordering::SeqCst),
            active_requests: self.active_requests.load(Ordering::SeqCst),
            total_requests: self.total_requests.load(Ordering::SeqCst),
            limiter_saturated_total: self.limiter_saturated_total.load(Ordering::SeqCst),
            rate_limit_rejections_total: self.rate_limit_rejections_total.load(Ordering::SeqCst),
            request_timeout_total: self.request_timeout_total.load(Ordering::SeqCst),
            read_timeout_total: self.read_timeout_total.load(Ordering::SeqCst),
            handler_timeout_total: self.handler_timeout_total.load(Ordering::SeqCst),
            method_not_allowed_total: self.method_not_allowed_total.load(Ordering::SeqCst),
            content_type_rejections_total: self
                .content_type_rejections_total
                .load(Ordering::SeqCst),
            body_rejections_total: self.body_rejections_total.load(Ordering::SeqCst),
        }
    }

    fn mark_started(&self, concurrency_limit: usize) {
        let started_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0);
        self.started_at_unix.store(started_at, Ordering::SeqCst);
        self.concurrency_limit
            .store(concurrency_limit, Ordering::SeqCst);
    }
}

impl Default for RuntimeState {
    fn default() -> Self {
        Self {
            started_at_unix: AtomicU64::new(0),
            concurrency_limit: AtomicUsize::new(0),
            active_connections: AtomicUsize::new(0),
            active_requests: AtomicUsize::new(0),
            total_requests: AtomicU64::new(0),
            limiter_saturated_total: AtomicU64::new(0),
            rate_limit_rejections_total: AtomicU64::new(0),
            request_timeout_total: AtomicU64::new(0),
            read_timeout_total: AtomicU64::new(0),
            handler_timeout_total: AtomicU64::new(0),
            method_not_allowed_total: AtomicU64::new(0),
            content_type_rejections_total: AtomicU64::new(0),
            body_rejections_total: AtomicU64::new(0),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct RuntimeSnapshot {
    pub started_at_unix: u64,
    pub concurrency_limit: usize,
    pub active_connections: usize,
    pub active_requests: usize,
    pub total_requests: u64,
    pub limiter_saturated_total: u64,
    pub rate_limit_rejections_total: u64,
    pub request_timeout_total: u64,
    pub read_timeout_total: u64,
    pub handler_timeout_total: u64,
    pub method_not_allowed_total: u64,
    pub content_type_rejections_total: u64,
    pub body_rejections_total: u64,
}

#[derive(Clone)]
struct DispatchPipeline {
    router: Arc<Router>,
    middleware: Arc<MiddlewareStack>,
    host: Arc<HostState>,
    runtime_state: Arc<RuntimeState>,
    semaphore: Arc<Semaphore>,
    settings: RuntimeSettings,
    security: RuntimeSecuritySettings,
    remote_addr: SocketAddr,
}

type ServerConnectionFuture =
    Pin<Box<dyn Future<Output = Result<(), Box<dyn std::error::Error + Send + Sync>>> + Send>>;

#[derive(Clone, Debug)]
pub struct RuntimeSettings {
    pub server: ServerOptions,
}

#[derive(Clone)]
pub struct GlobalRateLimiter {
    inner: Arc<GlobalRateLimiterInner>,
}

struct GlobalRateLimiterInner {
    capacity: f64,
    refill_tokens: f64,
    refill_interval: Duration,
    buckets: DashMap<IpAddr, BucketState>,
    last_cleanup_at_unix: AtomicU64,
}

struct BucketState {
    tokens: f64,
    last_refill: Instant,
}

impl GlobalRateLimiter {
    pub fn new(capacity: usize, refill_tokens: usize, refill_interval: Duration) -> Self {
        assert!(capacity > 0, "capacity must be positive");
        assert!(refill_tokens > 0, "refill_tokens must be positive");
        assert!(
            !refill_interval.is_zero(),
            "refill_interval must be positive"
        );
        Self {
            inner: Arc::new(GlobalRateLimiterInner {
                capacity: capacity as f64,
                refill_tokens: refill_tokens as f64,
                refill_interval,
                buckets: DashMap::new(),
                last_cleanup_at_unix: AtomicU64::new(0),
            }),
        }
    }

    pub fn check(&self, ip: IpAddr) -> bool {
        let now = Instant::now();
        maybe_prune_idle_rate_limit_buckets(&self.inner, now);
        let mut bucket = self.inner.buckets.entry(ip).or_insert_with(|| BucketState {
            tokens: self.inner.capacity,
            last_refill: now,
        });
        refill_bucket(&self.inner, &mut bucket, now);
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

fn refill_bucket(inner: &GlobalRateLimiterInner, bucket: &mut BucketState, now: Instant) {
    let elapsed = now.saturating_duration_since(bucket.last_refill);
    if elapsed.is_zero() {
        return;
    }

    let refill_units = elapsed.as_secs_f64() / inner.refill_interval.as_secs_f64();
    if refill_units <= 0.0 {
        return;
    }

    bucket.tokens = (bucket.tokens + refill_units * inner.refill_tokens).min(inner.capacity);
    bucket.last_refill = now;
}

fn maybe_prune_idle_rate_limit_buckets(inner: &GlobalRateLimiterInner, now: Instant) {
    const CLEANUP_ENTRY_THRESHOLD: usize = 4_096;

    if inner.buckets.len() < CLEANUP_ENTRY_THRESHOLD {
        return;
    }

    let now_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let cleanup_every = cleanup_interval_secs(inner.refill_interval);
    let last_cleanup = inner.last_cleanup_at_unix.load(Ordering::SeqCst);
    if now_unix.saturating_sub(last_cleanup) < cleanup_every {
        return;
    }
    if inner
        .last_cleanup_at_unix
        .compare_exchange(last_cleanup, now_unix, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }

    let idle_ttl = idle_bucket_ttl(inner.refill_interval);
    // Buckets that have fully refilled and stayed idle well past the
    // configured rate-limit window are safe to evict.
    inner.buckets.retain(|_, bucket| {
        let idle_for = now.saturating_duration_since(bucket.last_refill);
        idle_for < idle_ttl || bucket.tokens < inner.capacity
    });
}

fn cleanup_interval_secs(refill_interval: Duration) -> u64 {
    refill_interval.as_secs().max(1).saturating_mul(4).max(30)
}

fn idle_bucket_ttl(refill_interval: Duration) -> Duration {
    let secs = refill_interval.as_secs().max(1).saturating_mul(16).max(60);
    Duration::from_secs(secs)
}

#[derive(Clone)]
pub struct RuntimeSecuritySettings {
    pub(crate) max_body_size: usize,
    pub(crate) request_timeout: Duration,
    pub(crate) trusted_proxies: Vec<IpAddr>,
    pub(crate) rate_limiter: Option<GlobalRateLimiter>,
}

impl RuntimeSecuritySettings {
    pub(crate) fn new(
        max_body_size: usize,
        request_timeout: Duration,
        trusted_proxies: Vec<IpAddr>,
        rate_limiter: Option<GlobalRateLimiter>,
    ) -> Self {
        Self {
            max_body_size,
            request_timeout,
            trusted_proxies,
            rate_limiter,
        }
    }
}

pub(crate) fn enforce_pre_middleware_limits(
    request: &Request,
    security: &RuntimeSecuritySettings,
    runtime_state: Option<&RuntimeState>,
) -> Result<(), FrameworkError> {
    if request.body.len() > security.max_body_size {
        if let Some(runtime_state) = runtime_state {
            runtime_state
                .body_rejections_total
                .fetch_add(1, Ordering::SeqCst);
        }
        return Err(HttpError::payload_too_large("request exceeds maximum allowed size").into());
    }

    if let Some(rate_limiter) = &security.rate_limiter {
        if let Some(ip) = request.client_ip(&security.trusted_proxies) {
            if !rate_limiter.check(ip) {
                if let Some(runtime_state) = runtime_state {
                    runtime_state
                        .rate_limit_rejections_total
                        .fetch_add(1, Ordering::SeqCst);
                }
                return Err(HttpError::too_many_requests("too many requests").into());
            }
        }
    }

    Ok(())
}

impl RuntimeSettings {
    pub fn from_config(config: &AppConfig) -> Result<Self, HostBuildError> {
        Ok(Self {
            server: ServerOptions::try_from(&config.server).map_err(HostBuildError::Config)?,
        })
    }
}

pub struct ServerHandle {
    shutdown: tokio_util::sync::CancellationToken,
    join: JoinHandle<Result<(), HostError>>,
    local_addr: SocketAddr,
}

impl ServerHandle {
    pub fn shutdown(&self) {
        self.shutdown.cancel();
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub async fn wait(self) -> Result<(), HostError> {
        match self.join.await {
            Ok(result) => result,
            Err(error) => Err(HostError::Io(std::io::Error::other(error.to_string()))),
        }
    }
}

pub fn spawn_task<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    tokio::spawn(future)
}

pub fn block_on<F>(future: F) -> Result<F::Output, HostError>
where
    F: Future,
{
    Ok(tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(HostError::Io)?
        .block_on(future))
}

#[allow(clippy::too_many_arguments)]
pub async fn serve(
    router: Arc<Router>,
    middleware: Arc<MiddlewareStack>,
    host: Arc<HostState>,
    modules: Vec<Arc<dyn RuntimeModule>>,
    settings: RuntimeSettings,
    security: RuntimeSecuritySettings,
    context: HostContext,
) -> Result<ServerHandle, HostError> {
    let listener = TcpListener::bind(settings.server.bind_address)
        .await
        .map_err(HostError::Io)?;
    let local_addr = listener.local_addr().map_err(HostError::Io)?;
    let shutdown = context.background_tasks().cancellation_token();
    let semaphore = Arc::new(Semaphore::new(settings.server.concurrency_limit));
    let connections = ConnectionTracker::default();
    let runtime_state = host.runtime_state();
    runtime_state.mark_started(settings.server.concurrency_limit);

    let shutdown_for_join = shutdown.clone();
    let join = spawn_task(async move {
        loop {
            tokio::select! {
                _ = shutdown_for_join.cancelled() => break,
                accept = listener.accept() => {
                    let (stream, remote_addr) = accept.map_err(HostError::Io)?;
                    stream.set_nodelay(settings.server.tcp_nodelay).map_err(HostError::Io)?;
                    let io = TokioIo::new(stream);
                    let router = Arc::clone(&router);
                    let middleware = Arc::clone(&middleware);
                    let host = Arc::clone(&host);
                    let semaphore = Arc::clone(&semaphore);
                    let settings = settings.clone();
                    let security = security.clone();
                    let connection_options = settings.server.clone();
                    let grace_period = settings.server.graceful_shutdown;
                    let connection_shutdown = shutdown_for_join.clone();
                    let connections = connections.clone();
                    let runtime_state = Arc::clone(&runtime_state);
                    let runtime_state_for_connection = Arc::clone(&runtime_state);

                    connections.spawn(async move {
                        runtime_state
                            .active_connections
                            .fetch_add(1, Ordering::SeqCst);
                        let service = DispatchPipeline {
                            router,
                            middleware,
                            host,
                            runtime_state: Arc::clone(&runtime_state),
                            semaphore: Arc::clone(&semaphore),
                            settings: settings.clone(),
                            security: security.clone(),
                            remote_addr,
                        };

                        let connection = serve_connection(io, service, &connection_options);
                        tokio::pin!(connection);

                        tokio::select! {
                            result = &mut connection => {
                                let _ = result;
                            }
                            _ = connection_shutdown.cancelled() => {
                                let _ = timeout(grace_period, &mut connection).await;
                            }
                        }

                        runtime_state_for_connection
                            .active_connections
                            .fetch_sub(1, Ordering::SeqCst);
                    }).await;
                }
            }
        }

        connections
            .shutdown(settings.server.graceful_shutdown)
            .await;
        context.background_tasks().shutdown().await;
        for module in modules.iter().rev() {
            module
                .on_stop(&context)
                .await
                .map_err(HostError::Framework)?;
        }
        Ok(())
    });

    Ok(ServerHandle {
        shutdown,
        join,
        local_addr,
    })
}

async fn process_request(request: HyperRequest<Incoming>, pipeline: DispatchPipeline) -> Response {
    pipeline
        .runtime_state
        .total_requests
        .fetch_add(1, Ordering::SeqCst);
    let runtime_state = Arc::clone(&pipeline.runtime_state);
    match timeout(
        pipeline.settings.server.read_timeout,
        normalize_request(
            request,
            pipeline.remote_addr,
            &pipeline.settings.server,
            pipeline.security.max_body_size,
        ),
    )
    .await
    {
        Ok(Ok(request)) => {
            let span = info_span!(
                "vantus.request",
                method = %request.method,
                path = %request.path,
                route = tracing::field::Empty,
                status = tracing::field::Empty,
                remote_addr = %pipeline.remote_addr
            );

            if let Err(error) = enforce_pre_middleware_limits(
                &request,
                &pipeline.security,
                Some(runtime_state.as_ref()),
            ) {
                span.record(
                    "status",
                    tracing::field::display(error.to_response().status_code),
                );
                return error.to_response();
            }

            match timeout(
                pipeline.settings.server.handler_timeout,
                dispatch_request(
                    request,
                    pipeline.router,
                    pipeline.middleware,
                    pipeline.host,
                    Arc::clone(&runtime_state),
                )
                .instrument(span),
            )
            .await
            {
                Ok(response) => response,
                Err(_) => {
                    runtime_state
                        .handler_timeout_total
                        .fetch_add(1, Ordering::SeqCst);
                    Response::from_error(408, "Request Timeout", "408 Request Timeout")
                }
            }
        }
        Ok(Err(error)) => error.to_response(),
        Err(_) => {
            runtime_state
                .read_timeout_total
                .fetch_add(1, Ordering::SeqCst);
            Response::from_error(408, "Request Timeout", "408 Request Timeout")
        }
    }
}

fn serve_connection<S>(
    io: TokioIo<tokio::net::TcpStream>,
    service: S,
    options: &ServerOptions,
) -> ServerConnectionFuture
where
    S: hyper::service::Service<
            HyperRequest<Incoming>,
            Response = HyperResponse<Full<Bytes>>,
            Error = Infallible,
        > + Send
        + 'static,
    S::Future: Send + 'static,
{
    let protocol = options.protocol;
    let keep_alive = options.keep_alive;
    Box::pin(async move {
        match protocol {
            ServerProtocol::Http1 => hyper::server::conn::http1::Builder::new()
                .keep_alive(keep_alive)
                .serve_connection(io, service)
                .await
                .map_err(|error| Box::new(error) as Box<dyn std::error::Error + Send + Sync>),
            ServerProtocol::Http2 => hyper::server::conn::http2::Builder::new(TokioExecutor::new())
                .serve_connection(io, service)
                .await
                .map_err(|error| Box::new(error) as Box<dyn std::error::Error + Send + Sync>),
            ServerProtocol::Auto => {
                let mut builder =
                    hyper_util::server::conn::auto::Builder::new(TokioExecutor::new());
                builder.http1().keep_alive(keep_alive);
                builder.serve_connection(io, service).await
            }
        }
    })
}

async fn normalize_request(
    request: HyperRequest<Incoming>,
    remote_addr: SocketAddr,
    options: &ServerOptions,
    max_body_size: usize,
) -> Result<Request, FrameworkError> {
    let (parts, body) = request.into_parts();
    let method = Method::from_http_str(parts.method.as_str());
    if matches!(method, Method::Other(_)) {
        return Err(HttpError::method_not_allowed("unsupported http method").into());
    }

    let path = normalize_request_path(parts.uri.path())
        .map_err(|error| HttpError::bad_request(error.to_string()))?;

    let query_params = parts
        .uri
        .query()
        .map(Request::parse_query)
        .transpose()
        .map_err(|error| HttpError::bad_request(error.to_string()))?
        .unwrap_or_default();

    if parts
        .headers
        .get_all(hyper::header::CONTENT_LENGTH)
        .iter()
        .count()
        > 1
    {
        return Err(
            HttpError::bad_request("duplicate content-length headers are not allowed").into(),
        );
    }

    let version = parts.version;
    let headers = normalize_headers(parts.headers.iter(), options)?;
    validate_host_header(version, &headers)?;
    let body_bytes = read_body_limited(body, max_body_size).await?;

    if let Some(content_length) = headers
        .get("content-length")
        .and_then(|value| value.to_str().ok())
    {
        if content_length.contains(',') {
            return Err(
                HttpError::bad_request("duplicate content-length headers are not allowed").into(),
            );
        }
        let expected = content_length
            .parse::<usize>()
            .map_err(|_| HttpError::bad_request("content-length header is invalid"))?;
        if expected != body_bytes.len() {
            return Err(HttpError::bad_request(
                "request body length does not match content-length",
            )
            .into());
        }
    }

    Request::from_normalized_parts(
        method,
        path,
        normalize_version(version),
        headers,
        body_bytes,
        query_params,
        Some(remote_addr),
    )
    .map_err(|error| HttpError::bad_request(error.to_string()).into())
}

async fn read_body_limited(
    mut body: Incoming,
    max_request_bytes: usize,
) -> Result<Bytes, FrameworkError> {
    let mut collected = Vec::new();

    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|_| HttpError::bad_request("invalid request body"))?;
        if let Some(chunk) = frame.data_ref() {
            if collected.len() + chunk.len() > max_request_bytes {
                return Err(
                    HttpError::payload_too_large("request exceeds maximum allowed size").into(),
                );
            }
            collected.extend_from_slice(chunk);
        }
    }

    Ok(Bytes::from(collected))
}

fn normalize_headers<'a, I>(
    headers: I,
    options: &ServerOptions,
) -> Result<HeaderMap, FrameworkError>
where
    I: IntoIterator<
        Item = (
            &'a hyper::header::HeaderName,
            &'a hyper::header::HeaderValue,
        ),
    >,
{
    let mut normalized = HeaderMap::new();
    let mut header_count = 0usize;
    let mut header_bytes = 0usize;
    let mut seen_content_length = false;

    for (key, raw_value) in headers {
        header_count += 1;
        if header_count > options.max_header_count {
            return Err(HttpError::bad_request("too many request headers").into());
        }
        let value = raw_value
            .to_str()
            .map_err(|_| HttpError::bad_request("request header value is invalid"))?;
        header_bytes += key.as_str().len() + value.len();
        if header_bytes > options.max_header_bytes {
            return Err(
                HttpError::bad_request("request headers exceed maximum allowed size").into(),
            );
        }
        if key.as_str().eq_ignore_ascii_case("content-length") && seen_content_length {
            return Err(
                HttpError::bad_request("duplicate content-length headers are not allowed").into(),
            );
        }
        if key.as_str().eq_ignore_ascii_case("content-length") {
            seen_content_length = true;
        }
        normalized.append(key.clone(), raw_value.clone());
    }

    Ok(normalized)
}

fn normalize_version(version: Version) -> String {
    match version {
        Version::HTTP_09 => "HTTP/0.9".to_string(),
        Version::HTTP_10 => "HTTP/1.0".to_string(),
        Version::HTTP_11 => "HTTP/1.1".to_string(),
        Version::HTTP_2 => "HTTP/2".to_string(),
        Version::HTTP_3 => "HTTP/3".to_string(),
        other => format!("{other:?}"),
    }
}

fn validate_host_header(version: Version, headers: &HeaderMap) -> Result<(), FrameworkError> {
    if version != Version::HTTP_11 {
        return Ok(());
    }

    let values = headers
        .get_all(hyper::header::HOST)
        .iter()
        .map(|value| {
            value
                .to_str()
                .map(str::trim)
                .map(str::to_string)
                .map_err(|_| HttpError::bad_request("host header is invalid").into())
        })
        .collect::<Result<Vec<_>, FrameworkError>>()?;

    match values.as_slice() {
        [value] if !value.is_empty() => Ok(()),
        [] => Err(HttpError::bad_request("host header is required for HTTP/1.1").into()),
        [value] if value.is_empty() => Err(HttpError::bad_request("host header is invalid").into()),
        _ => Err(HttpError::bad_request("duplicate host headers are not allowed").into()),
    }
}

fn extract_content_type(request: &Request) -> Result<Option<String>, FrameworkError> {
    let values = request
        .header_values("content-type")
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();

    if values.is_empty() {
        return Ok(None);
    }

    if values.len() > 1 {
        return Err(
            HttpError::bad_request("duplicate content-type headers are not allowed").into(),
        );
    }

    let media_type = values[0]
        .split(';')
        .next()
        .map(str::trim)
        .unwrap_or_default()
        .to_ascii_lowercase();

    if media_type.is_empty() || !media_type.contains('/') {
        return Err(HttpError::bad_request("content-type header is invalid").into());
    }

    Ok(Some(media_type))
}

async fn dispatch_request(
    request: Request,
    router: Arc<Router>,
    middleware: Arc<MiddlewareStack>,
    host: Arc<HostState>,
    runtime_state: Arc<RuntimeState>,
) -> Response {
    let route = match router.resolve(&request.method, &request.path) {
        RouteResolution::Matched(route) => route,
        RouteResolution::MethodNotAllowed { allow } => {
            runtime_state
                .method_not_allowed_total
                .fetch_add(1, Ordering::SeqCst);
            return method_not_allowed_response(&allow);
        }
        RouteResolution::NotFound => return Response::not_found(),
    };

    Span::current().record(
        "route",
        tracing::field::display(route.route_pattern.as_ref()),
    );

    if let Err(error) = enforce_route_contract(&request, route.contract) {
        record_contract_error(runtime_state.as_ref(), &error);
        Span::current().record(
            "status",
            tracing::field::display(error.to_response().status_code),
        );
        return error.to_response();
    }

    runtime_state.active_requests.fetch_add(1, Ordering::SeqCst);
    let ctx = RequestContext::new(
        request,
        route.route_pattern.clone(),
        route.path_params,
        host,
    );
    let response = match middleware
        .execute(&route.middleware, ctx, route.handler)
        .await
    {
        Ok(response) => response,
        Err(error) => error.to_response(),
    };
    Span::current().record("status", tracing::field::display(response.status_code));
    runtime_state.active_requests.fetch_sub(1, Ordering::SeqCst);
    response
}

pub(crate) fn method_not_allowed_response(allow: &[Method]) -> Response {
    HttpError::method_not_allowed("method not allowed")
        .to_response()
        .with_header("Allow", Router::format_allow_header(allow))
}

pub(crate) fn enforce_route_contract(
    request: &Request,
    contract: RouteContract,
) -> Result<(), FrameworkError> {
    match contract.body {
        RequestBodyKind::None => {
            if !request.body.is_empty() {
                return Err(
                    HttpError::bad_request("request body is not allowed for this route").into(),
                );
            }
        }
        RequestBodyKind::Bytes => {}
        RequestBodyKind::Text | RequestBodyKind::Json => {
            let expected = contract.required_content_type().unwrap_or_default();
            let media_type = extract_content_type(request)?
                .ok_or_else(|| HttpError::unsupported_media_type("missing content-type header"))?;
            if media_type != expected {
                return Err(HttpError::unsupported_media_type(format!(
                    "expected content-type {expected}"
                ))
                .into());
            }
        }
    }

    Ok(())
}

pub(crate) fn record_contract_error(runtime_state: &RuntimeState, error: &FrameworkError) {
    if let FrameworkError::Http(error) = error {
        if error.status_code == 415 {
            runtime_state
                .content_type_rejections_total
                .fetch_add(1, Ordering::SeqCst);
        } else if error.status_code == 400 {
            runtime_state
                .body_rejections_total
                .fetch_add(1, Ordering::SeqCst);
        }
    }
}

pub(crate) fn record_method_not_allowed(runtime_state: &RuntimeState) {
    runtime_state
        .method_not_allowed_total
        .fetch_add(1, Ordering::SeqCst);
}

struct ResponseWriter;

impl ResponseWriter {
    fn write(response: Response) -> HyperResponse<Full<Bytes>> {
        let mut builder = HyperResponse::builder().status(response.status_code);
        for (key, value) in response.headers {
            builder = builder.header(key, value);
        }
        builder
            .body(Full::new(Bytes::from(response.body)))
            .unwrap_or_else(|_| {
                HyperResponse::builder()
                    .status(500)
                    .header("Content-Type", "text/plain; charset=utf-8")
                    .body(Full::new(Bytes::from_static(b"500 Internal Server Error")))
                    .unwrap_or_else(|_| {
                        HyperResponse::new(Full::new(Bytes::from_static(
                            b"500 Internal Server Error",
                        )))
                    })
            })
    }
}

impl hyper::service::Service<HyperRequest<Incoming>> for DispatchPipeline {
    type Response = HyperResponse<Full<Bytes>>;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn call(&self, request: HyperRequest<Incoming>) -> Self::Future {
        let pipeline = self.clone();
        Box::pin(async move {
            let permit = match pipeline.semaphore.clone().try_acquire_owned() {
                Ok(permit) => permit,
                Err(TryAcquireError::NoPermits) => {
                    pipeline
                        .runtime_state
                        .limiter_saturated_total
                        .fetch_add(1, Ordering::SeqCst);
                    match pipeline.semaphore.clone().acquire_owned().await {
                        Ok(permit) => permit,
                        Err(_) => {
                            return Ok(ResponseWriter::write(Response::internal_server_error()));
                        }
                    }
                }
                Err(TryAcquireError::Closed) => {
                    return Ok(ResponseWriter::write(Response::internal_server_error()));
                }
            };
            let _permit = permit;
            let runtime_state = Arc::clone(&pipeline.runtime_state);
            let response = match timeout(
                pipeline.security.request_timeout,
                process_request(request, pipeline.clone()),
            )
            .await
            {
                Ok(response) => response,
                Err(_) => {
                    runtime_state
                        .request_timeout_total
                        .fetch_add(1, Ordering::SeqCst);
                    Response::from_error(408, "Request Timeout", "408 Request Timeout")
                }
            };
            Ok(ResponseWriter::write(response))
        })
    }
}
