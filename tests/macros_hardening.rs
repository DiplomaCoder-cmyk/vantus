use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use vantus::__private::Next;
use vantus::{
    FrameworkError, Header, HostBuilder, HostContext, HttpError, IntoResponse, JsonBody,
    Middleware, MiddlewareFuture, Module, Path, Query, Request, RequestContext, RequestState,
    Response, TextBody, controller, middleware, module,
};

#[derive(Clone)]
struct GreetingController {
    prefix: &'static str,
}

#[derive(Serialize, serde::Deserialize)]
struct EchoPayload {
    value: String,
}

#[derive(Serialize)]
struct EchoedPayload {
    echoed: String,
}

#[derive(Debug)]
struct DemoRouteError;

impl IntoResponse for DemoRouteError {
    fn into_response(self) -> Result<Response, FrameworkError> {
        Ok(HttpError::new(418, "I'm a teapot", "teapot").to_response())
    }
}

fn write_ephemeral_config(name: &str, contents: &str) -> std::path::PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("vantus-macros-{name}-{unique}.properties"));
    std::fs::write(&path, contents).unwrap();
    path
}

fn middleware_events(ctx: &RequestContext) -> Arc<Mutex<Vec<String>>> {
    if let Some(events) = ctx.state::<Mutex<Vec<String>>>() {
        events
    } else {
        ctx.insert_state(Mutex::new(Vec::<String>::new()));
        ctx.state::<Mutex<Vec<String>>>()
            .expect("middleware state inserted")
    }
}

fn record_event(ctx: &RequestContext, label: &str) {
    middleware_events(ctx)
        .lock()
        .unwrap()
        .push(label.to_string());
}

#[controller]
impl GreetingController {
    #[vantus::get("/users/{id}")]
    fn show(
        &self,
        id: Path<u32>,
        mode: Query<String>,
        trace_id: Header<String>,
        ctx: RequestContext,
    ) -> Response {
        Response::text(format!(
            "{}:{}:{}:{}",
            self.prefix,
            id.into_inner(),
            mode.into_inner(),
            format_args!(
                "{}:{}",
                trace_id.into_inner(),
                ctx.app_config().service_name
            )
        ))
    }

    #[vantus::post("/echo")]
    fn echo(&self, body: TextBody) -> EchoedPayload {
        EchoedPayload {
            echoed: format!("{} {}", self.prefix, body.as_str()),
        }
    }

    #[vantus::post("/json/{id}")]
    async fn json_echo(
        &self,
        id: Path<u32>,
        mode: Option<Query<String>>,
        body: JsonBody<EchoPayload>,
    ) -> serde_json::Value {
        serde_json::json!({
            "id": id.into_inner(),
            "mode": mode.map(|value| value.into_inner()),
            "echoed": format!("{} {}", self.prefix, body.into_inner().value),
        })
    }

    #[vantus::get("/typed-error")]
    fn typed_error(&self) -> Result<EchoedPayload, DemoRouteError> {
        Err(DemoRouteError)
    }
}

#[derive(Default)]
struct ImplFirst;

impl Middleware for ImplFirst {
    fn handle(&self, ctx: RequestContext, next: Next) -> MiddlewareFuture {
        Box::pin(async move {
            record_event(&ctx, "impl-first");
            next.run(ctx).await
        })
    }
}

#[derive(Default)]
struct RouteFirst;

impl Middleware for RouteFirst {
    fn handle(&self, ctx: RequestContext, next: Next) -> MiddlewareFuture {
        Box::pin(async move {
            record_event(&ctx, "route-first");
            next.run(ctx).await
        })
    }
}

#[derive(Default)]
struct MiddlewareController;

#[middleware(ImplFirst)]
#[controller]
impl MiddlewareController {
    #[middleware(RouteFirst)]
    #[vantus::get("/ordered")]
    fn ordered(&self, events: RequestState<Mutex<Vec<String>>>) -> Vec<String> {
        events.as_ref().lock().unwrap().push("handler".to_string());
        events.as_ref().lock().unwrap().clone()
    }
}

#[derive(Clone)]
struct MacroModule {
    controller: Arc<GreetingController>,
    started: Arc<AtomicBool>,
    stopped: Arc<AtomicBool>,
}

#[module]
impl MacroModule {
    fn configure_routes(
        &self,
        routes: &mut dyn vantus::__private::RouteRegistrar,
    ) -> Result<(), FrameworkError> {
        <GreetingController as Module>::configure_routes_arc(Arc::clone(&self.controller), routes)
    }

    async fn on_start(&self, _host: &HostContext) -> Result<(), FrameworkError> {
        self.started.store(true, Ordering::SeqCst);
        Ok(())
    }

    async fn on_stop(&self, _host: &HostContext) -> Result<(), FrameworkError> {
        self.stopped.store(true, Ordering::SeqCst);
        Ok(())
    }
}

#[tokio::test]
async fn macro_routes_use_constructor_injection_and_request_extractors() {
    let started = Arc::new(AtomicBool::new(false));
    let stopped = Arc::new(AtomicBool::new(false));
    let config_path =
        write_ephemeral_config("constructor-injection", "service.name=mini-backend\n");
    let mut builder = HostBuilder::new();
    builder.config_file(&config_path);
    builder.module(MacroModule {
        controller: Arc::new(GreetingController { prefix: "hello" }),
        started: Arc::clone(&started),
        stopped: Arc::clone(&stopped),
    });
    let host = builder.build();

    let response = host
        .handle(
            Request::from_bytes(b"GET /users/42?mode=full HTTP/1.1\r\ntrace-id: req-9\r\n\r\n")
                .unwrap(),
        )
        .await;

    assert_eq!(response.status_code, 200);
    assert_eq!(
        String::from_utf8(response.body).unwrap(),
        "hello:42:full:req-9:mini-backend"
    );

    let server = host.serve().await.unwrap();
    server.shutdown();
    server.wait().await.unwrap();
    assert!(started.load(Ordering::SeqCst));
    assert!(stopped.load(Ordering::SeqCst));
    let _ = std::fs::remove_file(config_path);
}

#[tokio::test]
async fn macro_routes_handle_text_json_and_typed_errors() {
    let mut builder = HostBuilder::new();
    builder.module(MacroModule {
        controller: Arc::new(GreetingController { prefix: "hello" }),
        started: Arc::new(AtomicBool::new(false)),
        stopped: Arc::new(AtomicBool::new(false)),
    });
    let host = builder.build();

    let text = host
        .handle(
            Request::from_bytes(
                b"POST /echo HTTP/1.1\r\nContent-Type: text/plain\r\nContent-Length: 5\r\n\r\nworld",
            )
            .unwrap(),
        )
        .await;
    assert_eq!(text.status_code, 200);
    assert_eq!(
        String::from_utf8(text.body).unwrap(),
        "{\"echoed\":\"hello world\"}"
    );

    let json = host
        .handle(
            Request::from_bytes(
                b"POST /json/7?mode=debug HTTP/1.1\r\nContent-Type: application/json\r\nContent-Length: 16\r\n\r\n{\"value\":\"mars\"}",
            )
            .unwrap(),
        )
        .await;
    assert_eq!(json.status_code, 200);
    let json_body = String::from_utf8(json.body).unwrap();
    assert!(json_body.contains("\"id\":7"), "{json_body}");
    assert!(json_body.contains("\"mode\":\"debug\""), "{json_body}");
    assert!(
        json_body.contains("\"echoed\":\"hello mars\""),
        "{json_body}"
    );

    let typed_error = host
        .handle(Request::from_bytes(b"GET /typed-error HTTP/1.1\r\n\r\n").unwrap())
        .await;
    assert_eq!(typed_error.status_code, 418);
}

#[tokio::test]
async fn middleware_attributes_preserve_order() {
    #[derive(Clone, Default)]
    struct CombinedModule;

    #[module]
    impl CombinedModule {
        fn configure_routes(
            &self,
            routes: &mut dyn vantus::__private::RouteRegistrar,
        ) -> Result<(), FrameworkError> {
            <MiddlewareController as Module>::configure_routes_arc(
                Arc::new(MiddlewareController),
                routes,
            )
        }

        fn configure_middleware(&self, middleware: &mut vantus::__private::MiddlewareStack) {
            middleware.add(ImplFirst);
        }
    }

    let mut builder = HostBuilder::new();
    builder.module(CombinedModule);
    let host = builder.build();

    let response = host
        .handle(Request::from_bytes(b"GET /ordered HTTP/1.1\r\n\r\n").unwrap())
        .await;

    assert_eq!(response.status_code, 200);
    assert_eq!(
        String::from_utf8(response.body).unwrap(),
        "[\"impl-first\",\"impl-first\",\"route-first\",\"handler\"]"
    );
}

#[test]
fn macro_ui_compile_failures_cover_hardened_contracts() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/manual_api_removed.rs");
    t.compile_fail("tests/ui/service_parameter_removed.rs");
    t.compile_fail("tests/ui/body_extractor_on_get.rs");
    t.compile_fail("tests/ui/multiple_body_extractors.rs");
    t.compile_fail("tests/ui/path_parameter_mismatch.rs");
}
