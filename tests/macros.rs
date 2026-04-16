use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use vantus::{
    AppConfig, FrameworkError, HostBuilder, HostContext, Module, Path, Query, Request, Response,
    Service, ServiceCollection, TextBody, controller, module,
};
use serde::Serialize;

#[derive(Default)]
struct GreetingService;

impl GreetingService {
    fn greet(&self, value: &str) -> String {
        format!("hello {value}")
    }
}

#[derive(Clone, Default)]
struct MacroController;

#[controller]
impl MacroController {
    #[vantus::get("/users/{id}")]
    fn show(&self, id: Path<u32>, mode: Query<String>, config: AppConfig) -> Response {
        Response::text(format!(
            "{}:{}:{}",
            id.into_inner(),
            mode.into_inner(),
            config.service_name
        ))
    }

    #[vantus::post("/echo")]
    fn echo(
        &self,
        service: Service<GreetingService>,
        body: TextBody,
    ) -> Result<Response, FrameworkError> {
        #[derive(Serialize)]
        struct Payload {
            echoed: String,
        }

        Response::json_serialized(&Payload {
            echoed: service.as_ref().greet(body.as_str()),
        })
        .map_err(|error| FrameworkError::Internal(error.to_string()))
    }
}

#[derive(Clone)]
struct MacroModule {
    started: Arc<AtomicBool>,
    stopped: Arc<AtomicBool>,
}

#[allow(dead_code)]
#[module]
impl MacroModule {
    fn configure_services(&self, services: &mut ServiceCollection) -> Result<(), FrameworkError> {
        services.add_singleton(GreetingService);
        Ok(())
    }

    fn configure_routes(
        &self,
        routes: &mut dyn vantus::__private::RouteRegistrar,
    ) -> Result<(), FrameworkError> {
        <MacroController as Module>::configure_routes(&MacroController, routes)
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
async fn controller_macro_registers_routes_and_extracts_values() {
    let mut builder = HostBuilder::new();
    builder.module(MacroModule {
        started: Arc::new(AtomicBool::new(false)),
        stopped: Arc::new(AtomicBool::new(false)),
    });
    let host = builder.build().unwrap();

    let response = host
        .handle(Request::from_bytes(b"GET /users/42?mode=verbose HTTP/1.1\r\n\r\n").unwrap())
        .await;
    assert_eq!(
        String::from_utf8(response.body).unwrap(),
        "42:verbose:mini-backend"
    );
}

#[tokio::test]
async fn module_macro_forwards_async_lifecycle_hooks() {
    let started = Arc::new(AtomicBool::new(false));
    let stopped = Arc::new(AtomicBool::new(false));
    let mut builder = HostBuilder::new();
    builder.module(MacroModule {
        started: Arc::clone(&started),
        stopped: Arc::clone(&stopped),
    });

    let host = builder.build().unwrap();
    let server = host.serve().await.unwrap();
    assert!(started.load(Ordering::SeqCst));

    server.shutdown();
    server.wait().await.unwrap();
    assert!(stopped.load(Ordering::SeqCst));
}

#[test]
fn macro_invalid_parameter_patterns_fail_with_compile_errors() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/*.rs");
}
