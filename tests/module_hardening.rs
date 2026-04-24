use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use vantus::{
    BodyBytes, FrameworkError, HostBuilder, HostContext, IdGenerator, JsonBody, Request,
    RequestContext, Response, TextBody, module,
};

fn write_ephemeral_config(name: &str, contents: &str) -> std::path::PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("vantus-{name}-{unique}.properties"));
    std::fs::write(&path, contents).unwrap();
    path
}

#[derive(Clone)]
struct GreetingModule {
    service_name: String,
    started: Arc<AtomicBool>,
    stopped: Arc<AtomicBool>,
}

#[module]
impl GreetingModule {
    #[vantus::get("/service")]
    fn service(&self) -> Response {
        Response::text(self.service_name.clone())
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

#[derive(Deserialize)]
struct Payload {
    value: String,
}

#[derive(Clone, Default)]
struct ProtocolModule;

#[module]
impl ProtocolModule {
    #[vantus::get("/ping")]
    fn ping(&self) -> Response {
        Response::text("pong")
    }

    #[vantus::post("/text")]
    fn text(&self, body: TextBody) -> Response {
        Response::text(body.as_str().to_string())
    }

    #[vantus::post("/json")]
    fn json(&self, body: JsonBody<Payload>) -> Response {
        Response::text(body.into_inner().value)
    }

    #[vantus::post("/bytes")]
    fn bytes(&self, body: BodyBytes) -> Response {
        Response::text(String::from_utf8_lossy(body.as_slice()).into_owned())
    }

    #[vantus::get("/config-name")]
    fn config_name(&self, ctx: RequestContext) -> Response {
        Response::text(ctx.app_config().service_name.clone())
    }
}

struct SequenceIdGenerator {
    counter: AtomicUsize,
}

impl SequenceIdGenerator {
    fn new() -> Self {
        Self {
            counter: AtomicUsize::new(0),
        }
    }
}

impl IdGenerator for SequenceIdGenerator {
    fn next_id(&self) -> String {
        let next = self.counter.fetch_add(1, Ordering::SeqCst) + 1;
        format!("singleton-{next}")
    }
}

#[derive(Clone, Default)]
struct DependencyProbeModule;

#[module]
impl DependencyProbeModule {
    #[vantus::get("/singleton-id")]
    fn singleton_id(&self, ctx: RequestContext) -> Response {
        Response::text(ctx.id_generator().next_id())
    }
}

#[derive(Clone)]
struct LifecycleModule {
    name: &'static str,
    events: Arc<Mutex<Vec<String>>>,
}

#[module]
impl LifecycleModule {
    async fn on_start(&self, host: &HostContext) -> Result<(), FrameworkError> {
        self.events
            .lock()
            .unwrap()
            .push(format!("start:{}", self.name));
        let token = host.background_tasks().cancellation_token();
        let name = self.name;
        let events = Arc::clone(&self.events);
        host.background_tasks()
            .spawn(async move {
                token.cancelled().await;
                events.lock().unwrap().push(format!("cancel:{name}"));
            })
            .await;
        Ok(())
    }

    async fn on_stop(&self, host: &HostContext) -> Result<(), FrameworkError> {
        let cancelled = host.background_tasks().cancellation_token().is_cancelled();
        self.events
            .lock()
            .unwrap()
            .push(format!("stop:{}:{cancelled}", self.name));
        Ok(())
    }
}

#[derive(Clone)]
struct FailingStartModule {
    name: &'static str,
    events: Arc<Mutex<Vec<String>>>,
}

#[module]
impl FailingStartModule {
    async fn on_start(&self, _host: &HostContext) -> Result<(), FrameworkError> {
        self.events
            .lock()
            .unwrap()
            .push(format!("start:{}", self.name));
        Err(FrameworkError::startup(format!(
            "{} failed to start",
            self.name
        )))
    }

    async fn on_stop(&self, _host: &HostContext) -> Result<(), FrameworkError> {
        self.events
            .lock()
            .unwrap()
            .push(format!("stop:{}", self.name));
        Ok(())
    }
}

#[tokio::test]
async fn compose_with_config_builds_constructor_injected_modules() {
    let config_path = write_ephemeral_config(
        "compose-with-config",
        "service.name=hardening-suite\nserver.bind-port=0\n",
    );
    let started = Arc::new(AtomicBool::new(false));
    let stopped = Arc::new(AtomicBool::new(false));
    let mut builder = HostBuilder::new();
    builder.config_file(&config_path);
    builder.compose_with_config({
        let started = Arc::clone(&started);
        let stopped = Arc::clone(&stopped);
        move |_configuration, app, context| {
            context.module(GreetingModule {
                service_name: app.service_name.clone(),
                started: Arc::clone(&started),
                stopped: Arc::clone(&stopped),
            });
            Ok(())
        }
    });

    let host = builder.build();
    let response = host
        .handle(Request::from_bytes(b"GET /service HTTP/1.1\r\n\r\n").unwrap())
        .await;
    assert_eq!(response.status_code, 200);
    assert_eq!(String::from_utf8(response.body).unwrap(), "hardening-suite");

    let server = host.serve().await.unwrap();
    server.shutdown();
    server.wait().await.unwrap();
    assert!(started.load(Ordering::SeqCst));
    assert!(stopped.load(Ordering::SeqCst));
    let _ = std::fs::remove_file(config_path);
}

#[tokio::test]
async fn route_contracts_reject_invalid_bodies_and_content_types() {
    let mut builder = HostBuilder::new();
    builder.module(ProtocolModule);
    let host = builder.build();

    let wrong_media = host
        .handle(
            Request::from_bytes(
                b"POST /json HTTP/1.1\r\nContent-Type: text/plain\r\nContent-Length: 16\r\n\r\n{\"value\":\"mars\"}",
            )
            .unwrap(),
        )
        .await;
    assert_eq!(wrong_media.status_code, 415);

    let missing_media = host
        .handle(
            Request::from_bytes(b"POST /text HTTP/1.1\r\nContent-Length: 5\r\n\r\nhello").unwrap(),
        )
        .await;
    assert_eq!(missing_media.status_code, 415);

    let body_not_allowed = host
        .handle(
            Request::from_bytes(b"GET /ping HTTP/1.1\r\nContent-Length: 4\r\n\r\npong").unwrap(),
        )
        .await;
    assert_eq!(body_not_allowed.status_code, 400);

    let bytes_accept_any_type = host
        .handle(
            Request::from_bytes(
                b"POST /bytes HTTP/1.1\r\nContent-Type: application/octet-stream\r\nContent-Length: 4\r\n\r\ndata",
            )
            .unwrap(),
        )
        .await;
    assert_eq!(bytes_accept_any_type.status_code, 200);
    assert_eq!(
        String::from_utf8(bytes_accept_any_type.body).unwrap(),
        "data"
    );
}

#[tokio::test]
async fn web_platform_adds_security_headers() {
    let mut builder = HostBuilder::new();
    builder.with_web_platform();
    builder.module(ProtocolModule);
    let host = builder.build();

    let response = host
        .handle(Request::from_bytes(b"GET /ping HTTP/1.1\r\n\r\n").unwrap())
        .await;

    assert_eq!(response.status_code, 200);
    assert!(
        response
            .headers
            .iter()
            .any(|(key, value)| key == "X-Content-Type-Options" && value == "nosniff")
    );
    assert!(
        response
            .headers
            .iter()
            .any(|(key, value)| key == "X-Frame-Options" && value == "DENY")
    );
}

#[tokio::test]
async fn request_context_exposes_app_config_without_handler_injection() {
    let config_path = write_ephemeral_config(
        "ctx-app-config",
        "service.name=ctx-service\nserver.bind-port=0\n",
    );
    let mut builder = HostBuilder::new();
    builder.config_file(&config_path);
    builder.module(ProtocolModule);
    let host = builder.build();

    let response = host
        .handle(Request::from_bytes(b"GET /config-name HTTP/1.1\r\n\r\n").unwrap())
        .await;

    assert_eq!(response.status_code, 200);
    assert_eq!(String::from_utf8(response.body).unwrap(), "ctx-service");
    let _ = std::fs::remove_file(config_path);
}

#[tokio::test]
async fn singleton_dependencies_are_shared_across_host_and_request_contexts() {
    let mut builder = HostBuilder::new();
    builder.id_generator(SequenceIdGenerator::new());
    builder.module(DependencyProbeModule);
    let host = builder.build();

    let first = host.context().id_generator().next_id();
    let second = host
        .handle(Request::from_bytes(b"GET /singleton-id HTTP/1.1\r\nHost: local\r\n\r\n").unwrap())
        .await;
    let third = host
        .handle(Request::from_bytes(b"GET /singleton-id HTTP/1.1\r\nHost: local\r\n\r\n").unwrap())
        .await;

    assert_eq!(first, "singleton-1");
    assert_eq!(String::from_utf8(second.body).unwrap(), "singleton-2");
    assert_eq!(String::from_utf8(third.body).unwrap(), "singleton-3");
}

#[tokio::test]
async fn module_registration_order_preserves_startup_order() {
    let config_path = write_ephemeral_config(
        "module-registration-order",
        "service.name=lifecycle\nserver.bind-port=0\n",
    );
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut builder = HostBuilder::new();
    builder.config_file(&config_path);
    builder.module(LifecycleModule {
        name: "first",
        events: Arc::clone(&events),
    });
    builder.module(LifecycleModule {
        name: "second",
        events: Arc::clone(&events),
    });
    let host = builder.build();
    let server = host.serve().await.unwrap();

    server.shutdown();
    server.wait().await.unwrap();

    let starts = events
        .lock()
        .unwrap()
        .iter()
        .filter(|event| event.starts_with("start:"))
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(starts, vec!["start:first", "start:second"]);
    let _ = std::fs::remove_file(config_path);
}

#[tokio::test]
async fn graceful_shutdown_cancels_background_tasks_before_reverse_stop_order() {
    let config_path = write_ephemeral_config(
        "graceful-shutdown-order",
        "service.name=lifecycle\nserver.bind-port=0\n",
    );
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut builder = HostBuilder::new();
    builder.config_file(&config_path);
    builder.module(LifecycleModule {
        name: "first",
        events: Arc::clone(&events),
    });
    builder.module(LifecycleModule {
        name: "second",
        events: Arc::clone(&events),
    });
    let host = builder.build();
    let server = host.serve().await.unwrap();

    server.shutdown();
    server.wait().await.unwrap();

    let events = events.lock().unwrap().clone();
    let cancel_first = events
        .iter()
        .position(|event| event == "cancel:first")
        .unwrap();
    let cancel_second = events
        .iter()
        .position(|event| event == "cancel:second")
        .unwrap();
    let stop_second = events
        .iter()
        .position(|event| event == "stop:second:true")
        .unwrap();
    let stop_first = events
        .iter()
        .position(|event| event == "stop:first:true")
        .unwrap();

    assert!(cancel_first < stop_second, "{events:?}");
    assert!(cancel_second < stop_second, "{events:?}");
    assert!(stop_second < stop_first, "{events:?}");
    let _ = std::fs::remove_file(config_path);
}

#[tokio::test]
async fn startup_failures_roll_back_already_started_modules() {
    let config_path = write_ephemeral_config(
        "startup-rollback",
        "service.name=lifecycle\nserver.bind-port=0\n",
    );
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut builder = HostBuilder::new();
    builder.config_file(&config_path);
    builder.module(LifecycleModule {
        name: "healthy",
        events: Arc::clone(&events),
    });
    builder.module(FailingStartModule {
        name: "broken",
        events: Arc::clone(&events),
    });

    let error = match builder.build().serve().await {
        Ok(_) => panic!("startup should fail"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("failed to start"));

    let events = events.lock().unwrap().clone();
    assert_eq!(
        events,
        vec![
            "start:healthy".to_string(),
            "start:broken".to_string(),
            "cancel:healthy".to_string(),
            "stop:healthy:true".to_string(),
        ]
    );
    let _ = std::fs::remove_file(config_path);
}
