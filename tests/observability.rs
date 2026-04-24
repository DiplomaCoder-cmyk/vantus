use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use vantus::{
    AtomicIdGenerator, FrameworkError, HostBuilder, HostContext, LogLevel, LogSink, Request,
    RequestLogEvent, RequestState, Response, TextBody, module,
};
use vantus_observability::{
    ObservabilityModule, ReadinessCheck, ReadinessContributor, ReadinessRegistry, RequestId,
};

fn write_ephemeral_config(name: &str) -> std::path::PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path =
        std::env::temp_dir().join(format!("vantus-observability-{name}-{unique}.properties"));
    std::fs::write(
        &path,
        "server.bind-port=0\nserver.protocol=auto\nserver.read-timeout-seconds=3\nserver.handler-timeout-seconds=3\n",
    )
    .unwrap();
    path
}

struct HealthyDatabase;

#[async_trait]
impl ReadinessContributor for HealthyDatabase {
    async fn check(&self) -> ReadinessCheck {
        ReadinessCheck::healthy("database", "connected")
    }
}

#[derive(Clone, Default)]
struct ProbeModule {
    readiness: Arc<ReadinessRegistry>,
}

#[module]
impl ProbeModule {
    #[vantus::get("/request-id")]
    fn request_id(&self, request_id: RequestState<RequestId>) -> Response {
        Response::text(request_id.as_ref().as_str().to_string())
    }

    #[vantus::post("/echo")]
    fn echo(&self, body: TextBody) -> Response {
        Response::text(body.as_str().to_string())
    }

    async fn on_start(&self, host: &HostContext) -> Result<(), FrameworkError> {
        self.readiness.register(Arc::new(HealthyDatabase)).await;
        host.log_sink().log_text(
            vantus::LogLevel::Info,
            "tests.observability",
            "probe module started",
        );
        Ok(())
    }
}

#[derive(Clone, Default)]
struct CapturedLogSink {
    request_events: Arc<Mutex<Vec<(String, RequestLogEvent)>>>,
    text_events: Arc<Mutex<Vec<(LogLevel, String, String)>>>,
}

impl LogSink for CapturedLogSink {
    fn log_text(&self, level: LogLevel, target: &str, message: &str) {
        self.text_events
            .lock()
            .unwrap()
            .push((level, target.to_string(), message.to_string()));
    }

    fn log_request(&self, target: &str, event: &RequestLogEvent) {
        self.request_events
            .lock()
            .unwrap()
            .push((target.to_string(), event.clone()));
    }
}

#[derive(Clone, Default)]
struct OrderModule;

#[module]
impl OrderModule {
    #[vantus::get("/orders/{id}")]
    fn show(&self) -> Response {
        Response::text("ok")
    }
}

#[tokio::test]
async fn observability_module_exposes_request_id_and_diag_routes() {
    let config_path = write_ephemeral_config("diag");
    let observability = ObservabilityModule::default();
    let mut builder = HostBuilder::new();
    builder.config_file(&config_path);
    builder.module(observability.clone());
    builder.module(ProbeModule {
        readiness: observability.readiness_registry(),
    });
    let host = builder.build();

    let request_id_response = host
        .handle(Request::from_bytes(b"GET /request-id HTTP/1.1\r\nHost: local\r\n\r\n").unwrap())
        .await;
    assert_eq!(request_id_response.status_code, 200);
    assert!(
        request_id_response
            .headers
            .iter()
            .any(|(key, _)| key == "X-Request-Id")
    );

    let server = host.serve().await.unwrap();
    let diag = host_request(server.local_addr(), "/diag").await;
    assert!(diag.contains("\"runtime\""), "{diag}");
    assert!(diag.contains("\"limiter_saturated_total\""), "{diag}");

    let ready = host_request(server.local_addr(), "/ready").await;
    assert!(ready.contains("\"database\""), "{ready}");

    server.shutdown();
    server.wait().await.unwrap();
    let _ = std::fs::remove_file(config_path);
}

#[tokio::test]
async fn request_state_uses_typed_context_state_api() {
    let config_path = write_ephemeral_config("request-state");
    let observability = ObservabilityModule::default();
    let mut builder = HostBuilder::new();
    builder.config_file(&config_path);
    builder.module(observability.clone());
    builder.module(ProbeModule {
        readiness: observability.readiness_registry(),
    });
    let host = builder.build();

    let response = host
        .handle(Request::from_bytes(b"GET /request-id HTTP/1.1\r\nHost: local\r\n\r\n").unwrap())
        .await;

    assert_eq!(response.status_code, 200);
    assert!(!String::from_utf8(response.body).unwrap().is_empty());
    let _ = std::fs::remove_file(config_path);
}

#[tokio::test]
async fn observability_module_uses_shared_id_generator_service() {
    let config_path = write_ephemeral_config("custom-request-id");
    let observability = ObservabilityModule::default();
    let mut builder = HostBuilder::new();
    builder.config_file(&config_path);
    builder.id_generator(AtomicIdGenerator::with_prefix("req"));
    builder.module(observability.clone());
    builder.module(ProbeModule {
        readiness: observability.readiness_registry(),
    });
    let host = builder.build();

    let response = host
        .handle(Request::from_bytes(b"GET /request-id HTTP/1.1\r\nHost: local\r\n\r\n").unwrap())
        .await;
    let body = String::from_utf8(response.body).unwrap();

    assert!(body.starts_with("req-"), "{body}");
    assert!(
        response
            .headers
            .iter()
            .any(|(key, value)| key == "X-Request-Id" && value.starts_with("req-"))
    );
    let _ = std::fs::remove_file(config_path);
}

#[tokio::test]
async fn metrics_endpoint_reports_runtime_and_route_counters() {
    let config_path = write_ephemeral_config("metrics");
    let observability = ObservabilityModule::default();
    let mut builder = HostBuilder::new();
    builder.config_file(&config_path);
    builder.module(observability.clone());
    builder.module(ProbeModule {
        readiness: observability.readiness_registry(),
    });
    let host = builder.build();
    let server = host.serve().await.unwrap();

    let ok = raw_request(
        server.local_addr(),
        b"GET /request-id HTTP/1.1\r\nHost: local\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(ok.starts_with("HTTP/1.1 200"), "{ok}");

    let wrong_method = raw_request(
        server.local_addr(),
        b"POST /request-id HTTP/1.1\r\nHost: local\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(wrong_method.starts_with("HTTP/1.1 405"), "{wrong_method}");

    let wrong_media = raw_request(
        server.local_addr(),
        b"POST /echo HTTP/1.1\r\nHost: local\r\nContent-Type: application/json\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello",
    )
    .await;
    assert!(wrong_media.starts_with("HTTP/1.1 415"), "{wrong_media}");

    let metrics = host_request(server.local_addr(), "/metrics").await;
    assert!(metrics.contains("vantus_requests_total"), "{metrics}");
    assert!(
        metrics.contains("vantus_route_requests_total{"),
        "{metrics}"
    );
    assert!(
        metrics.contains("vantus_runtime_total_requests"),
        "{metrics}"
    );
    assert!(
        metrics.contains("vantus_runtime_method_not_allowed_total 1"),
        "{metrics}"
    );
    assert!(
        metrics.contains("vantus_runtime_content_type_rejections_total 1"),
        "{metrics}"
    );

    server.shutdown();
    server.wait().await.unwrap();
    let _ = std::fs::remove_file(config_path);
}

#[tokio::test]
async fn liveness_and_readiness_toggle_with_host_lifecycle() {
    let config_path = write_ephemeral_config("lifecycle-toggle");
    let observability = ObservabilityModule::default();
    let mut builder = HostBuilder::new();
    builder.config_file(&config_path);
    builder.module(observability.clone());
    builder.module(ProbeModule {
        readiness: observability.readiness_registry(),
    });
    let host = builder.build();

    let pre_live = host
        .handle(Request::from_bytes(b"GET /live HTTP/1.1\r\nHost: local\r\n\r\n").unwrap())
        .await;
    let pre_ready = host
        .handle(Request::from_bytes(b"GET /ready HTTP/1.1\r\nHost: local\r\n\r\n").unwrap())
        .await;
    assert_eq!(
        String::from_utf8(pre_live.body).unwrap(),
        "{\"status\":\"stopped\"}"
    );
    assert_eq!(pre_ready.status_code, 503);

    let server = host.serve().await.unwrap();
    let live = host_request(server.local_addr(), "/live").await;
    let ready = host_request(server.local_addr(), "/ready").await;
    assert!(live.contains("\"status\":\"ok\""), "{live}");
    assert!(ready.contains("\"status\":\"ok\""), "{ready}");

    server.shutdown();
    server.wait().await.unwrap();

    let mut post_builder = HostBuilder::new();
    post_builder.module(observability.clone());
    let post_host = post_builder.build();
    let post_live = post_host
        .handle(Request::from_bytes(b"GET /live HTTP/1.1\r\nHost: local\r\n\r\n").unwrap())
        .await;
    let post_ready = post_host
        .handle(Request::from_bytes(b"GET /ready HTTP/1.1\r\nHost: local\r\n\r\n").unwrap())
        .await;
    assert_eq!(
        String::from_utf8(post_live.body).unwrap(),
        "{\"status\":\"stopped\"}"
    );
    assert_eq!(post_ready.status_code, 503);
    let _ = std::fs::remove_file(config_path);
}

#[tokio::test]
async fn request_id_is_propagated_to_state_header_and_structured_logs() {
    let logs = CapturedLogSink::default();
    let observability = ObservabilityModule::default();
    let mut builder = HostBuilder::new();
    builder.log_sink(logs.clone());
    builder.module(observability.clone());
    builder.module(ProbeModule {
        readiness: observability.readiness_registry(),
    });
    let host = builder.build();

    let response = host
        .handle(Request::from_bytes(b"GET /request-id HTTP/1.1\r\nHost: local\r\n\r\n").unwrap())
        .await;
    let body = String::from_utf8(response.body).unwrap();
    let header = response
        .headers
        .iter()
        .find(|(key, _)| key == "X-Request-Id")
        .map(|(_, value)| value.clone())
        .expect("request id header");
    let request_events = logs.request_events.lock().unwrap().clone();
    let (_, event) = request_events.last().expect("structured request log");

    assert_eq!(body, header);
    assert_eq!(event.request_id.as_deref(), Some(body.as_str()));
    assert_eq!(event.status_code, 200);
    assert_eq!(event.path, "/request-id");
}

#[tokio::test]
async fn prometheus_metrics_increment_with_request_volume() {
    let observability = ObservabilityModule::default();
    let mut builder = HostBuilder::new();
    builder.module(observability.clone());
    builder.module(ProbeModule {
        readiness: observability.readiness_registry(),
    });
    let host = builder.build();

    for _ in 0..2 {
        let response = host
            .handle(
                Request::from_bytes(b"GET /request-id HTTP/1.1\r\nHost: local\r\n\r\n").unwrap(),
            )
            .await;
        assert_eq!(response.status_code, 200);
    }

    let metrics = host
        .handle(Request::from_bytes(b"GET /metrics HTTP/1.1\r\nHost: local\r\n\r\n").unwrap())
        .await;
    let body = String::from_utf8(metrics.body).unwrap();

    assert!(body.contains("vantus_requests_total 2"), "{body}");
    assert!(
        body.contains(
            "vantus_route_requests_total{method=\"GET\",route=\"/request-id\",status=\"200\"} 2"
        ),
        "{body}"
    );
}

#[tokio::test]
async fn structured_logging_includes_request_context_and_sanitized_paths() {
    let config_path = write_ephemeral_config("structured-log-context");
    std::fs::write(
        &config_path,
        "server.bind-port=0\nserver.protocol=auto\nserver.trusted-proxies=127.0.0.1\n",
    )
    .unwrap();
    let logs = CapturedLogSink::default();
    let observability = ObservabilityModule::default();
    let mut builder = HostBuilder::new();
    builder.config_file(&config_path);
    builder.log_sink(logs.clone());
    builder.module(observability);
    builder.module(OrderModule);
    let host = builder.build();
    let server = host.serve().await.unwrap();

    let response = raw_request(
        server.local_addr(),
        b"GET /orders/1234567890 HTTP/1.1\r\nHost: local\r\nX-Forwarded-For: 203.0.113.10\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");

    server.shutdown();
    server.wait().await.unwrap();

    let request_events = logs.request_events.lock().unwrap().clone();
    let (_, event) = request_events.last().expect("structured request log");
    assert_eq!(event.path, "/orders/:redacted");
    assert_eq!(event.client_ip.as_deref(), Some("203.0.113.10"));
    assert_eq!(event.status_code, 200);
    assert!(event.request_id.is_some());
    assert!(event.headers.is_empty());
    let _ = std::fs::remove_file(config_path);
}

#[tokio::test]
async fn diagnostic_endpoints_are_not_exposed_without_observability_module() {
    let mut builder = HostBuilder::new();
    builder.module(ProbeModule::default());
    let host = builder.build();

    let diag = host
        .handle(Request::from_bytes(b"GET /diag HTTP/1.1\r\nHost: local\r\n\r\n").unwrap())
        .await;
    let metrics = host
        .handle(Request::from_bytes(b"GET /metrics HTTP/1.1\r\nHost: local\r\n\r\n").unwrap())
        .await;

    assert_eq!(diag.status_code, 404);
    assert_eq!(metrics.status_code, 404);
}

async fn host_request(addr: std::net::SocketAddr, path: &str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    let mut stream = TcpStream::connect(addr).await.unwrap();
    let request = format!("GET {path} HTTP/1.1\r\nHost: local\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    String::from_utf8_lossy(&response).into_owned()
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
