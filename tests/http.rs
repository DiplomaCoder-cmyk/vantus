use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use vantus::{
    FrameworkError, GlobalRateLimiter, Header, HostBuilder, HttpError, IntoResponse, JsonBody,
    Method, Middleware, MiddlewareFuture, ParseError, Path, Query, Request, RequestContext,
    RequestState, Response, TextBody, module,
};

#[test]
fn parses_simple_get_request_with_query() {
    let raw = b"GET /hello?name=world HTTP/1.1\r\nHost: example\r\n\r\n";
    let request = Request::from_bytes(raw).unwrap();
    assert_eq!(request.method, Method::Get);
    assert_eq!(request.path, "/hello");
}

#[test]
fn rejects_invalid_percent_encoding() {
    let err = Request::from_bytes(b"GET /bad?value=%ZZ HTTP/1.1\r\n\r\n").unwrap_err();
    assert!(matches!(err, ParseError::InvalidPercentEncoding));
}

#[test]
fn response_serializes_json() {
    #[derive(Serialize)]
    struct Payload<'a> {
        message: &'a str,
    }

    let response = Response::json_serialized(&Payload { message: "ok" }).unwrap();
    assert_eq!(
        String::from_utf8(response.body).unwrap(),
        "{\"message\":\"ok\"}"
    );
}

#[test]
fn typed_success_into_response_serializes_as_json() {
    #[derive(Serialize)]
    struct Payload<'a> {
        message: &'a str,
    }

    let response = Payload { message: "typed" }.into_response().unwrap();
    assert_eq!(response.status_code, 200);
    assert!(
        response
            .headers
            .iter()
            .any(|(key, value)| key == "Content-Type" && value == "application/json; charset=utf-8")
    );
    assert_eq!(
        String::from_utf8(response.body).unwrap(),
        "{\"message\":\"typed\"}"
    );
}

#[test]
fn typed_error_into_response_renders_custom_status() {
    #[derive(Debug)]
    struct Teapot;

    impl IntoResponse for Teapot {
        fn into_response(self) -> Result<Response, FrameworkError> {
            Ok(HttpError::new(418, "I'm a teapot", "teapot").to_response())
        }
    }

    let response = Teapot.into_response().unwrap();
    assert_eq!(response.status_code, 418);
    assert_eq!(String::from_utf8(response.body).unwrap(), "teapot");
}

#[test]
fn preserves_multi_value_headers_and_body_bytes() {
    let raw = b"GET /hello HTTP/1.1\r\nAccept: text/plain\r\nAccept: application/json\r\nX-Forwarded-For: 203.0.113.10\r\nX-Forwarded-For: 198.51.100.8\r\nContent-Length: 5\r\n\r\nhello";
    let request = Request::from_bytes(raw).unwrap();

    let accept_values = request.header_values("accept").collect::<Vec<_>>();
    assert_eq!(accept_values, vec!["text/plain", "application/json"]);
    assert_eq!(request.body.as_ref(), b"hello");
}

#[test]
fn client_ip_uses_first_forwarded_for_value_from_multi_value_headers() {
    let raw = b"GET /hello HTTP/1.1\r\nX-Forwarded-For: 203.0.113.10\r\nX-Forwarded-For: 198.51.100.8\r\n\r\n";
    let mut request = Request::from_bytes(raw).unwrap();
    request.remote_addr = Some("127.0.0.1:8080".parse().unwrap());

    let client_ip = request
        .client_ip(&["127.0.0.1".parse().unwrap()])
        .unwrap()
        .to_string();
    assert_eq!(client_ip, "198.51.100.8");
}

#[test]
fn client_ip_ignores_spoofed_leftmost_forwarded_value() {
    let raw = b"GET /hello HTTP/1.1\r\nX-Forwarded-For: 9.9.9.9, 198.51.100.8\r\n\r\n";
    let mut request = Request::from_bytes(raw).unwrap();
    request.remote_addr = Some("127.0.0.1:8080".parse().unwrap());

    let client_ip = request
        .client_ip(&["127.0.0.1".parse().unwrap()])
        .unwrap()
        .to_string();
    assert_eq!(client_ip, "198.51.100.8");
}

#[test]
fn rejects_invalid_request_line_spacing() {
    let err = Request::from_bytes(b"GET  /hello HTTP/1.1\r\n\r\n").unwrap_err();
    assert!(matches!(err, ParseError::InvalidRequestLine));
}

#[test]
fn rejects_unsupported_http_versions() {
    let err = Request::from_bytes(b"GET /hello HTTP/2\r\n\r\n").unwrap_err();
    assert!(matches!(err, ParseError::InvalidHttpVersion));
}

#[test]
fn rejects_path_traversal_segments() {
    let err = Request::from_bytes(b"GET /../../etc/passwd HTTP/1.1\r\n\r\n").unwrap_err();
    assert!(matches!(err, ParseError::PathTraversal));
}

#[test]
fn rejects_too_many_headers() {
    let mut raw = b"GET /hello HTTP/1.1\r\n".to_vec();
    for index in 0..101 {
        raw.extend_from_slice(format!("X-Test-{index}: value\r\n").as_bytes());
    }
    raw.extend_from_slice(b"\r\n");

    let err = Request::from_bytes(&raw).unwrap_err();
    assert!(matches!(err, ParseError::TooManyHeaders));
}

#[test]
fn rejects_too_many_query_params() {
    let params = (0..129)
        .map(|index| format!("k{index}=v"))
        .collect::<Vec<_>>()
        .join("&");
    let raw = format!("GET /hello?{params} HTTP/1.1\r\n\r\n");

    let err = Request::from_bytes(raw.as_bytes()).unwrap_err();
    assert!(matches!(err, ParseError::TooManyQueryParams));
}

#[test]
fn rejects_query_values_that_are_too_long() {
    let long_value = "a".repeat(8_193);
    let raw = format!("GET /hello?key={long_value} HTTP/1.1\r\n\r\n");

    let err = Request::from_bytes(raw.as_bytes()).unwrap_err();
    assert!(matches!(err, ParseError::QueryValueTooLong));
}

#[test]
fn body_str_borrows_utf8_without_allocating() {
    let request =
        Request::from_bytes(b"POST /hello HTTP/1.1\r\nContent-Length: 5\r\n\r\nhello").unwrap();
    assert_eq!(request.body_str(), Some("hello"));
}

#[test]
fn invalid_response_headers_are_ignored() {
    let response = Response::text("ok").with_header("bad\nheader", "value");
    let wire = String::from_utf8(response.to_http_bytes()).unwrap();

    assert!(!wire.contains("bad\nheader"));
    assert!(wire.contains("Content-Length: 2"));
}

fn write_ephemeral_config(name: &str) -> std::path::PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("vantus-http-{name}-{unique}.properties"));
    std::fs::write(
        &path,
        "server.bind-port=0\nserver.protocol=auto\nserver.read-timeout-seconds=3\nserver.handler-timeout-seconds=3\n",
    )
    .unwrap();
    path
}

fn write_ephemeral_config_with_contents(name: &str, contents: &str) -> std::path::PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("vantus-http-{name}-{unique}.properties"));
    std::fs::write(&path, contents).unwrap();
    path
}

#[derive(Clone, Default)]
struct EchoModule;

#[derive(Serialize)]
struct EchoResult<'a> {
    status: &'a str,
}

#[module]
impl EchoModule {
    #[vantus::post("/echo")]
    fn echo(&self) -> EchoResult<'static> {
        EchoResult { status: "ok" }
    }
}

#[derive(Clone, Default)]
struct TextModule;

#[module]
impl TextModule {
    #[vantus::post("/text")]
    fn text(&self, body: TextBody) -> Response {
        Response::text(body.as_str().to_string())
    }
}

#[derive(Clone)]
struct CountingTextModule {
    hits: Arc<AtomicUsize>,
}

#[module]
impl CountingTextModule {
    #[vantus::post("/counted")]
    fn counted(&self, body: TextBody) -> Response {
        let _ = body;
        self.hits.fetch_add(1, Ordering::SeqCst);
        Response::text("accepted")
    }
}

#[derive(Clone, Default)]
struct SlowModule;

#[module]
impl SlowModule {
    #[vantus::get("/slow")]
    async fn slow(&self) -> Response {
        tokio::time::sleep(Duration::from_millis(200)).await;
        Response::text("slow")
    }
}

#[derive(Clone, Default)]
struct PingModule;

#[module]
impl PingModule {
    #[vantus::get("/ping")]
    fn ping(&self) -> Response {
        Response::text("pong")
    }
}

#[derive(Clone, Default)]
struct PanicModule;

#[module]
impl PanicModule {
    #[vantus::get("/explode")]
    fn explode(&self) -> Response {
        panic!("boom");
    }

    #[vantus::get("/healthy")]
    fn healthy(&self) -> Response {
        Response::text("still-ok")
    }
}

#[derive(Debug, Deserialize)]
struct StrictJsonPayload {
    count: u32,
    label: String,
}

#[derive(Clone, Default)]
struct ExtractionModule;

#[module]
impl ExtractionModule {
    #[vantus::post("/json/strict")]
    fn strict_json(&self, body: JsonBody<StrictJsonPayload>) -> Response {
        let body = body.into_inner();
        Response::text(format!("{}:{}", body.count, body.label))
    }

    #[vantus::get("/headers/required")]
    fn required_header(&self, trace_id: Header<String>) -> Response {
        Response::text(trace_id.into_inner())
    }

    #[vantus::get("/items/{id}/typed")]
    fn typed_item(&self, id: Path<u32>) -> Response {
        Response::text(format!("item-{}", id.into_inner()))
    }

    #[vantus::get("/search")]
    fn search(&self, q: Option<Query<String>>) -> Response {
        Response::text(
            q.map(|value| value.into_inner())
                .unwrap_or_else(|| "none".to_string()),
        )
    }
}

#[derive(Clone, Default)]
struct BodySnapshotMiddleware;

impl Middleware for BodySnapshotMiddleware {
    fn handle(&self, ctx: RequestContext, next: vantus::__private::Next) -> MiddlewareFuture {
        let snapshot = ctx.request().body_as_string();
        ctx.insert_state(snapshot);
        next.run(ctx)
    }
}

#[derive(Clone, Default)]
struct BodyReplayModule;

#[module]
impl BodyReplayModule {
    fn configure_middleware(&self, middleware: &mut vantus::__private::MiddlewareStack) {
        middleware.add(BodySnapshotMiddleware);
    }

    #[vantus::post("/body/reconsume")]
    fn reconsume(&self, body: TextBody, snapshot: RequestState<String>) -> Response {
        Response::text(format!("{}|{}", snapshot.as_ref(), body.as_str()))
    }
}

#[tokio::test]
async fn comma_separated_content_length_is_rejected() {
    let config_path = write_ephemeral_config("dup-content-length");
    let mut builder = HostBuilder::new();
    builder.config_file(&config_path);
    builder.module(EchoModule);
    let host = builder.build();
    let server = host.serve().await.expect("Server should start");

    let raw = b"POST /echo HTTP/1.1\r\nHost: local\r\nContent-Length: 2, 2\r\nConnection: close\r\n\r\nok";
    let response = raw_request(server.local_addr(), raw).await;
    assert!(response.starts_with("HTTP/1.1 400"), "{response}");

    server.shutdown();
    server.wait().await.unwrap();
    let _ = std::fs::remove_file(config_path);
}

#[tokio::test]
async fn malformed_content_length_is_rejected() {
    let config_path = write_ephemeral_config("bad-content-length");
    let mut builder = HostBuilder::new();
    builder.config_file(&config_path);
    builder.module(EchoModule);
    let host = builder.build();
    let server = host.serve().await.unwrap();

    let raw =
        b"POST /echo HTTP/1.1\r\nHost: local\r\nContent-Length: abc\r\nConnection: close\r\n\r\nok";
    let response = raw_request(server.local_addr(), raw).await;
    assert!(response.starts_with("HTTP/1.1 400"), "{response}");

    server.shutdown();
    server.wait().await.unwrap();
    let _ = std::fs::remove_file(config_path);
}

#[tokio::test]
async fn missing_host_header_is_rejected_for_http11() {
    let config_path = write_ephemeral_config("missing-host");
    let mut builder = HostBuilder::new();
    builder.config_file(&config_path);
    builder.module(PingModule);
    let host = builder.build();
    let server = host.serve().await.unwrap();

    let raw = b"GET /ping HTTP/1.1\r\nConnection: close\r\n\r\n";
    let response = raw_request(server.local_addr(), raw).await;
    assert!(response.starts_with("HTTP/1.1 400"), "{response}");
    assert!(
        response.contains("host header is required for HTTP/1.1"),
        "{response}"
    );

    server.shutdown();
    server.wait().await.unwrap();
    let _ = std::fs::remove_file(config_path);
}

#[tokio::test]
async fn duplicate_host_headers_are_rejected() {
    let config_path = write_ephemeral_config("duplicate-host");
    let mut builder = HostBuilder::new();
    builder.config_file(&config_path);
    builder.module(PingModule);
    let host = builder.build();
    let server = host.serve().await.unwrap();

    let raw = b"GET /ping HTTP/1.1\r\nHost: local\r\nHost: backup\r\nConnection: close\r\n\r\n";
    let response = raw_request(server.local_addr(), raw).await;
    assert!(response.starts_with("HTTP/1.1 400"), "{response}");

    server.shutdown();
    server.wait().await.unwrap();
    let _ = std::fs::remove_file(config_path);
}

#[tokio::test]
async fn malformed_host_header_is_rejected() {
    let config_path = write_ephemeral_config("malformed-host");
    let mut builder = HostBuilder::new();
    builder.config_file(&config_path);
    builder.module(PingModule);
    let host = builder.build();
    let server = host.serve().await.unwrap();

    let raw = b"GET /ping HTTP/1.1\r\nHost:   \r\nConnection: close\r\n\r\n";
    let response = raw_request(server.local_addr(), raw).await;
    assert!(response.starts_with("HTTP/1.1 400"), "{response}");

    server.shutdown();
    server.wait().await.unwrap();
    let _ = std::fs::remove_file(config_path);
}

#[tokio::test]
async fn duplicate_content_type_headers_are_rejected() {
    let config_path = write_ephemeral_config("duplicate-content-type");
    let mut builder = HostBuilder::new();
    builder.config_file(&config_path);
    builder.module(TextModule);
    let host = builder.build();
    let server = host.serve().await.unwrap();

    let raw = b"POST /text HTTP/1.1\r\nHost: local\r\nContent-Type: text/plain\r\nContent-Type: application/json\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello";
    let response = raw_request(server.local_addr(), raw).await;
    assert!(response.starts_with("HTTP/1.1 400"), "{response}");

    server.shutdown();
    server.wait().await.unwrap();
    let _ = std::fs::remove_file(config_path);
}

#[tokio::test]
async fn invalid_content_type_header_is_rejected() {
    let config_path = write_ephemeral_config("invalid-content-type");
    let mut builder = HostBuilder::new();
    builder.config_file(&config_path);
    builder.module(TextModule);
    let host = builder.build();
    let server = host.serve().await.unwrap();

    let raw = b"POST /text HTTP/1.1\r\nHost: local\r\nContent-Type: invalid\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello";
    let response = raw_request(server.local_addr(), raw).await;
    assert!(response.starts_with("HTTP/1.1 400"), "{response}");

    server.shutdown();
    server.wait().await.unwrap();
    let _ = std::fs::remove_file(config_path);
}

#[tokio::test]
async fn malformed_json_syntax_returns_bad_request() {
    let mut builder = HostBuilder::new();
    builder.module(ExtractionModule);
    let host = builder.build();

    let response = host
        .handle(
            Request::from_bytes(
                b"POST /json/strict HTTP/1.1\r\nHost: local\r\nContent-Type: application/json\r\n\r\n{\"count\": 3, \"label\": ",
            )
            .unwrap(),
        )
        .await;

    assert_eq!(response.status_code, 400);
    assert!(
        String::from_utf8(response.body)
            .unwrap()
            .contains("failed to parse field: body")
    );
}

#[tokio::test]
async fn typed_schema_mismatch_returns_bad_request() {
    let mut builder = HostBuilder::new();
    builder.module(ExtractionModule);
    let host = builder.build();

    let response = host
        .handle(
            Request::from_bytes(
                b"POST /json/strict HTTP/1.1\r\nHost: local\r\nContent-Type: application/json\r\n\r\n{\"count\":\"abc\",\"label\":7}",
            )
            .unwrap(),
        )
        .await;

    assert_eq!(response.status_code, 400);
    assert!(
        String::from_utf8(response.body)
            .unwrap()
            .contains("failed to parse field: body")
    );
}

#[tokio::test]
async fn missing_required_header_returns_bad_request() {
    let mut builder = HostBuilder::new();
    builder.module(ExtractionModule);
    let host = builder.build();

    let response = host
        .handle(
            Request::from_bytes(b"GET /headers/required HTTP/1.1\r\nHost: local\r\n\r\n").unwrap(),
        )
        .await;

    assert_eq!(response.status_code, 400);
    assert_eq!(
        String::from_utf8(response.body).unwrap(),
        "missing field: trace-id"
    );
}

#[tokio::test]
async fn unsupported_media_type_returns_415_for_json_route() {
    let mut builder = HostBuilder::new();
    builder.module(ExtractionModule);
    let host = builder.build();

    let response = host
        .handle(
            Request::from_bytes(
                b"POST /json/strict HTTP/1.1\r\nHost: local\r\nContent-Type: text/plain\r\n\r\n{\"count\":3,\"label\":\"mars\"}",
            )
            .unwrap(),
        )
        .await;

    assert_eq!(response.status_code, 415);
    assert!(
        String::from_utf8(response.body)
            .unwrap()
            .contains("expected content-type application/json")
    );
}

#[tokio::test]
async fn path_parameter_type_validation_returns_bad_request() {
    let mut builder = HostBuilder::new();
    builder.module(ExtractionModule);
    let host = builder.build();

    let response = host
        .handle(
            Request::from_bytes(b"GET /items/not-a-number/typed HTTP/1.1\r\nHost: local\r\n\r\n")
                .unwrap(),
        )
        .await;

    assert_eq!(response.status_code, 400);
    assert_eq!(
        String::from_utf8(response.body).unwrap(),
        "failed to parse field: id"
    );
}

#[tokio::test]
async fn query_string_optionality_distinguishes_missing_and_present_values() {
    let mut builder = HostBuilder::new();
    builder.module(ExtractionModule);
    let host = builder.build();

    let missing = host
        .handle(Request::from_bytes(b"GET /search HTTP/1.1\r\nHost: local\r\n\r\n").unwrap())
        .await;
    let present = host
        .handle(Request::from_bytes(b"GET /search?q=rust HTTP/1.1\r\nHost: local\r\n\r\n").unwrap())
        .await;

    assert_eq!(missing.status_code, 200);
    assert_eq!(String::from_utf8(missing.body).unwrap(), "none");
    assert_eq!(present.status_code, 200);
    assert_eq!(String::from_utf8(present.body).unwrap(), "rust");
}

#[tokio::test]
async fn buffered_request_body_can_be_reconsumed_across_middleware_and_handler() {
    let mut builder = HostBuilder::new();
    builder.module(BodyReplayModule);
    let host = builder.build();

    let response = host
        .handle(
            Request::from_bytes(
                b"POST /body/reconsume HTTP/1.1\r\nHost: local\r\nContent-Type: text/plain\r\nContent-Length: 5\r\n\r\nhello",
            )
            .unwrap(),
        )
        .await;

    assert_eq!(response.status_code, 200);
    assert_eq!(String::from_utf8(response.body).unwrap(), "hello|hello");
}

#[tokio::test]
async fn builder_max_body_size_override_rejects_large_payloads_with_413() {
    let config_path = write_ephemeral_config("max-body-override");
    let mut builder = HostBuilder::new();
    builder.config_file(&config_path);
    builder.max_body_size(2);
    builder.module(EchoModule);
    let host = builder.build();
    let server = host.serve().await.unwrap();

    let raw =
        b"POST /echo HTTP/1.1\r\nHost: local\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello";
    let response = raw_request(server.local_addr(), raw).await;
    assert!(response.starts_with("HTTP/1.1 413"), "{response}");

    server.shutdown();
    server.wait().await.unwrap();
    let _ = std::fs::remove_file(config_path);
}

#[tokio::test]
async fn payload_limit_violation_is_rejected_before_handler_execution() {
    let config_path = write_ephemeral_config("payload-limit-violation");
    let hits = Arc::new(AtomicUsize::new(0));
    let mut builder = HostBuilder::new();
    builder.config_file(&config_path);
    builder.max_body_size(4);
    builder.module(CountingTextModule {
        hits: Arc::clone(&hits),
    });
    let host = builder.build();
    let server = host.serve().await.unwrap();

    let response = raw_request(
        server.local_addr(),
        b"POST /counted HTTP/1.1\r\nHost: local\r\nContent-Type: text/plain\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello",
    )
    .await;

    assert!(response.starts_with("HTTP/1.1 413"), "{response}");
    assert_eq!(hits.load(Ordering::SeqCst), 0);

    server.shutdown();
    server.wait().await.unwrap();
    let _ = std::fs::remove_file(config_path);
}

#[tokio::test]
async fn builder_request_timeout_override_triggers_408() {
    let config_path = write_ephemeral_config("request-timeout-override");
    let mut builder = HostBuilder::new();
    builder.config_file(&config_path);
    builder.request_timeout(Duration::from_millis(50));
    builder.module(SlowModule);
    let host = builder.build();
    let server = host.serve().await.unwrap();

    let raw = b"GET /slow HTTP/1.1\r\nHost: local\r\nConnection: close\r\n\r\n";
    let response = raw_request(server.local_addr(), raw).await;
    assert!(response.starts_with("HTTP/1.1 408"), "{response}");

    server.shutdown();
    server.wait().await.unwrap();
    let _ = std::fs::remove_file(config_path);
}

#[tokio::test]
async fn global_rate_limit_burst_only_allows_requests_within_capacity() {
    let config_path = write_ephemeral_config("rate-limit-burst");
    let mut builder = HostBuilder::new();
    builder.config_file(&config_path);
    builder.rate_limiter(GlobalRateLimiter::new(3, 1, Duration::from_secs(60)));
    builder.module(PingModule);
    let host = builder.build();
    let server = host.serve().await.unwrap();

    let raw = b"GET /ping HTTP/1.1\r\nHost: local\r\nConnection: close\r\n\r\n".to_vec();
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..12 {
        let addr = server.local_addr();
        let raw = raw.clone();
        tasks.spawn(async move { raw_request(addr, &raw).await });
    }

    let mut ok = 0;
    let mut rejected = 0;
    while let Some(result) = tasks.join_next().await {
        let response = result.unwrap();
        if response.starts_with("HTTP/1.1 200") {
            ok += 1;
        } else if response.starts_with("HTTP/1.1 429") {
            rejected += 1;
        } else {
            panic!("unexpected response: {response}");
        }
    }

    assert_eq!(ok, 3);
    assert_eq!(rejected, 9);

    server.shutdown();
    server.wait().await.unwrap();
    let _ = std::fs::remove_file(config_path);
}

#[tokio::test]
async fn global_rate_limiter_blocks_repeated_requests_from_same_ip() {
    let config_path = write_ephemeral_config("rate-limit-same-ip");
    let mut builder = HostBuilder::new();
    builder.config_file(&config_path);
    builder.rate_limiter(GlobalRateLimiter::new(1, 1, Duration::from_secs(60)));
    builder.module(PingModule);
    let host = builder.build();
    let server = host.serve().await.unwrap();

    let raw = b"GET /ping HTTP/1.1\r\nHost: local\r\nConnection: close\r\n\r\n";
    let first = raw_request(server.local_addr(), raw).await;
    let second = raw_request(server.local_addr(), raw).await;

    assert!(first.starts_with("HTTP/1.1 200"), "{first}");
    assert!(second.starts_with("HTTP/1.1 429"), "{second}");

    server.shutdown();
    server.wait().await.unwrap();
    let _ = std::fs::remove_file(config_path);
}

#[tokio::test]
async fn slowloris_body_upload_triggers_read_timeout() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    let config_path = write_ephemeral_config_with_contents(
        "slowloris-read-timeout",
        "server.bind-port=0\nserver.protocol=auto\nserver.read-timeout-seconds=1\nserver.handler-timeout-seconds=3\n",
    );
    let mut builder = HostBuilder::new();
    builder.config_file(&config_path);
    builder.module(TextModule);
    let host = builder.build();
    let server = host.serve().await.unwrap();

    let mut stream = TcpStream::connect(server.local_addr()).await.unwrap();
    stream
        .write_all(
            b"POST /text HTTP/1.1\r\nHost: local\r\nContent-Type: text/plain\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhe",
        )
        .await
        .unwrap();
    stream.flush().await.unwrap();

    tokio::time::sleep(Duration::from_millis(1_500)).await;

    let mut response = Vec::new();
    tokio::time::timeout(Duration::from_secs(3), stream.read_to_end(&mut response))
        .await
        .unwrap()
        .unwrap();
    let response = String::from_utf8_lossy(&response).into_owned();
    assert!(response.starts_with("HTTP/1.1 408"), "{response}");

    server.shutdown();
    server.wait().await.unwrap();
    let _ = std::fs::remove_file(config_path);
}

#[tokio::test]
async fn panic_recovery_isolates_failing_requests_from_healthy_ones() {
    let config_path = write_ephemeral_config("panic-recovery");
    let mut builder = HostBuilder::new();
    builder.config_file(&config_path);
    builder.with_web_platform();
    builder.module(PanicModule);
    let host = builder.build();
    let server = host.serve().await.unwrap();

    let panic_response = raw_request(
        server.local_addr(),
        b"GET /explode HTTP/1.1\r\nHost: local\r\nConnection: close\r\n\r\n",
    )
    .await;
    let healthy_response = raw_request(
        server.local_addr(),
        b"GET /healthy HTTP/1.1\r\nHost: local\r\nConnection: close\r\n\r\n",
    )
    .await;

    assert!(
        panic_response.starts_with("HTTP/1.1 500"),
        "{panic_response}"
    );
    assert!(
        healthy_response.starts_with("HTTP/1.1 200"),
        "{healthy_response}"
    );
    assert!(healthy_response.ends_with("still-ok"), "{healthy_response}");

    server.shutdown();
    server.wait().await.unwrap();
    let _ = std::fs::remove_file(config_path);
}

#[tokio::test]
async fn rate_limiter_honors_trusted_proxy_forwarded_ips() {
    let config_path = write_ephemeral_config_with_contents(
        "rate-limit-trusted-proxy",
        "server.bind-port=0\nserver.protocol=auto\nserver.trusted-proxies=127.0.0.1\n",
    );
    let mut builder = HostBuilder::new();
    builder.config_file(&config_path);
    builder.rate_limiter(GlobalRateLimiter::new(1, 1, Duration::from_secs(60)));
    builder.module(PingModule);
    let host = builder.build();
    let server = host.serve().await.unwrap();

    let first = raw_request(
        server.local_addr(),
        b"GET /ping HTTP/1.1\r\nHost: local\r\nX-Forwarded-For: 203.0.113.10\r\nConnection: close\r\n\r\n",
    )
    .await;
    let second = raw_request(
        server.local_addr(),
        b"GET /ping HTTP/1.1\r\nHost: local\r\nX-Forwarded-For: 198.51.100.8\r\nConnection: close\r\n\r\n",
    )
    .await;
    let third = raw_request(
        server.local_addr(),
        b"GET /ping HTTP/1.1\r\nHost: local\r\nX-Forwarded-For: 203.0.113.10\r\nConnection: close\r\n\r\n",
    )
    .await;

    assert!(first.starts_with("HTTP/1.1 200"), "{first}");
    assert!(second.starts_with("HTTP/1.1 200"), "{second}");
    assert!(third.starts_with("HTTP/1.1 429"), "{third}");

    server.shutdown();
    server.wait().await.unwrap();
    let _ = std::fs::remove_file(config_path);
}

#[tokio::test]
async fn untrusted_proxy_headers_cannot_spoof_rate_limit_identity() {
    let config_path = write_ephemeral_config_with_contents(
        "rate-limit-spoof",
        "server.bind-port=0\nserver.protocol=auto\nserver.trusted-proxies=203.0.113.254\n",
    );
    let mut builder = HostBuilder::new();
    builder.config_file(&config_path);
    builder.rate_limiter(GlobalRateLimiter::new(1, 1, Duration::from_secs(60)));
    builder.module(PingModule);
    let host = builder.build();
    let server = host.serve().await.unwrap();

    let first = raw_request(
        server.local_addr(),
        b"GET /ping HTTP/1.1\r\nHost: local\r\nX-Forwarded-For: 198.51.100.8\r\nConnection: close\r\n\r\n",
    )
    .await;
    let second = raw_request(
        server.local_addr(),
        b"GET /ping HTTP/1.1\r\nHost: local\r\nX-Forwarded-For: 203.0.113.10\r\nConnection: close\r\n\r\n",
    )
    .await;

    assert!(first.starts_with("HTTP/1.1 200"), "{first}");
    assert!(second.starts_with("HTTP/1.1 429"), "{second}");

    server.shutdown();
    server.wait().await.unwrap();
    let _ = std::fs::remove_file(config_path);
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
