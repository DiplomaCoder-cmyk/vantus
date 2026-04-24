use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use futures_util::future::join_all;
use vantus::{
    HostBuilder, Middleware, MiddlewareFuture, ObservabilityModule, Request, RequestContext,
    RequestState, Response, TextBody, module,
};

fn write_ephemeral_config(name: &str, contents: &str) -> std::path::PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("vantus-runtime-{name}-{unique}.properties"));
    std::fs::write(&path, contents).unwrap();
    path
}

#[derive(Clone, Default)]
struct SlowConnectionModule;

#[module]
impl SlowConnectionModule {
    #[vantus::get("/slow")]
    async fn slow(&self) -> Response {
        tokio::time::sleep(Duration::from_millis(75)).await;
        Response::text("slow-ok")
    }
}

#[derive(Clone, Default)]
struct LatencyOverheadMiddleware;

impl Middleware for LatencyOverheadMiddleware {
    fn handle(&self, ctx: RequestContext, next: vantus::__private::Next) -> MiddlewareFuture {
        Box::pin(async move {
            let started = Instant::now();
            let response = next.run(ctx).await?;
            Ok(response.with_header(
                "X-Middleware-Latency-Us",
                started.elapsed().as_micros().to_string(),
            ))
        })
    }
}

#[derive(Clone, Default)]
struct LatencyModule;

#[module]
impl LatencyModule {
    fn configure_middleware(&self, middleware: &mut vantus::__private::MiddlewareStack) {
        middleware.add(LatencyOverheadMiddleware);
    }

    #[vantus::get("/fast")]
    fn fast(&self) -> Response {
        Response::text("ok")
    }
}

#[derive(Default)]
struct LeakCounters {
    created: AtomicUsize,
    dropped: AtomicUsize,
}

struct TrackedRequestState {
    dropped: Arc<LeakCounters>,
}

impl Drop for TrackedRequestState {
    fn drop(&mut self) {
        self.dropped.dropped.fetch_add(1, Ordering::SeqCst);
    }
}

#[derive(Clone)]
struct LeakTrackingMiddleware {
    counters: Arc<LeakCounters>,
}

impl Middleware for LeakTrackingMiddleware {
    fn handle(&self, ctx: RequestContext, next: vantus::__private::Next) -> MiddlewareFuture {
        self.counters.created.fetch_add(1, Ordering::SeqCst);
        ctx.insert_state(TrackedRequestState {
            dropped: Arc::clone(&self.counters),
        });
        next.run(ctx)
    }
}

#[derive(Clone)]
struct LeakTrackingModule {
    counters: Arc<LeakCounters>,
}

#[module]
impl LeakTrackingModule {
    fn configure_middleware(&self, middleware: &mut vantus::__private::MiddlewareStack) {
        middleware.add(LeakTrackingMiddleware {
            counters: Arc::clone(&self.counters),
        });
    }

    #[vantus::post("/echo")]
    fn echo(&self, body: TextBody, _state: RequestState<TrackedRequestState>) -> Response {
        Response::text(body.as_str().to_string())
    }
}

#[tokio::test]
async fn high_concurrency_connection_limit_saturates_without_dropping_requests() {
    let config_path = write_ephemeral_config(
        "connection-limit",
        "server.bind-port=0\nserver.protocol=auto\nserver.concurrency-limit=1\nserver.read-timeout-seconds=5\nserver.handler-timeout-seconds=5\n",
    );
    let mut builder = HostBuilder::new();
    builder.config_file(&config_path);
    builder.module(ObservabilityModule::default());
    builder.module(SlowConnectionModule);
    let host = builder.build();
    let runtime = host.context().runtime_state();
    let server = host.serve().await.unwrap();

    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..8 {
        let addr = server.local_addr();
        tasks.spawn(async move {
            raw_request(
                addr,
                b"GET /slow HTTP/1.1\r\nHost: local\r\nConnection: close\r\n\r\n",
            )
            .await
        });
    }

    while let Some(result) = tasks.join_next().await {
        let response = result.unwrap();
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    }

    server.shutdown();
    server.wait().await.unwrap();

    let snapshot = runtime.snapshot();
    assert_eq!(snapshot.concurrency_limit, 1);
    assert!(snapshot.limiter_saturated_total > 0, "{snapshot:?}");
    assert_eq!(snapshot.active_connections, 0);
    let _ = std::fs::remove_file(config_path);
}

#[tokio::test]
async fn middleware_latency_overhead_stays_low_for_noop_pipeline() {
    let mut builder = HostBuilder::new();
    builder.module(LatencyModule);
    let host = builder.build();
    let request = Request::from_bytes(b"GET /fast HTTP/1.1\r\nHost: local\r\n\r\n").unwrap();
    let mut total_latency_us = 0u128;

    for _ in 0..64 {
        let response = host.handle(request.clone()).await;
        assert_eq!(response.status_code, 200);
        let latency_us = response
            .headers
            .iter()
            .find(|(key, _)| key == "X-Middleware-Latency-Us")
            .map(|(_, value)| value.parse::<u128>().unwrap())
            .expect("latency header");
        total_latency_us += latency_us;
    }

    let average_latency_us = total_latency_us / 64;
    assert!(average_latency_us < 50_000, "{average_latency_us}");
}

#[tokio::test]
async fn memory_leak_under_stress_releases_request_scoped_state() {
    let counters = Arc::new(LeakCounters::default());
    let mut builder = HostBuilder::new();
    builder.module(LeakTrackingModule {
        counters: Arc::clone(&counters),
    });
    let host = builder.build();
    let runtime = host.context().runtime_state();
    let request = Request::from_bytes(
        b"POST /echo HTTP/1.1\r\nHost: local\r\nContent-Type: text/plain\r\n\r\npayload",
    )
    .unwrap();

    let responses = join_all((0..256).map(|_| host.handle(request.clone()))).await;
    for response in responses {
        assert_eq!(response.status_code, 200);
    }

    tokio::task::yield_now().await;
    let snapshot = runtime.snapshot();
    assert_eq!(snapshot.active_requests, 0);
    assert_eq!(counters.created.load(Ordering::SeqCst), 256);
    assert_eq!(
        counters.created.load(Ordering::SeqCst),
        counters.dropped.load(Ordering::SeqCst)
    );
}

async fn raw_request(addr: std::net::SocketAddr, raw: &[u8]) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    let mut stream = TcpStream::connect(addr).await.unwrap();
    stream.write_all(raw).await.unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    String::from_utf8_lossy(&response).into_owned()
}
