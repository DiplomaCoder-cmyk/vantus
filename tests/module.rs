use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use vantus::{
    AppConfig, Config, FrameworkError, HostBuilder, HostContext, Response, Service,
    ServiceCollection, TextBody, module,
};

#[derive(Default)]
struct CounterService {
    ids: AtomicUsize,
}

impl CounterService {
    fn next(&self) -> usize {
        self.ids.fetch_add(1, Ordering::SeqCst) + 1
    }
}

#[derive(Clone)]
struct LifecycleModule {
    started: Arc<AtomicBool>,
    stopped: Arc<AtomicBool>,
}

#[allow(dead_code)]
#[module]
impl LifecycleModule {
    fn configure_services(&self, services: &mut ServiceCollection) -> Result<(), FrameworkError> {
        services.add_singleton(CounterService::default());
        services.add_scoped::<usize, _>(|scope| Ok(scope.resolve::<CounterService>()?.next()));
        Ok(())
    }

    #[vantus::post("/echo")]
    fn echo(
        &self,
        request_id: Service<usize>,
        config: Config<AppConfig>,
        body: TextBody,
    ) -> Response {
        Response::json_value(serde_json::json!({
            "request_id": *request_id.as_ref(),
            "service": config.as_ref().service_name,
            "echo": body.as_str()
        }))
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
async fn host_runs_async_startup_and_shutdown_and_serves_requests() {
    let started = Arc::new(AtomicBool::new(false));
    let stopped = Arc::new(AtomicBool::new(false));
    let mut builder = HostBuilder::new();
    builder.module(LifecycleModule {
        started: Arc::clone(&started),
        stopped: Arc::clone(&stopped),
    });

    let host = builder.build().unwrap();
    let response = host
        .handle(
            vantus::Request::from_bytes(
                b"POST /echo HTTP/1.1\r\nContent-Length: 5\r\n\r\nhello",
            )
            .unwrap(),
        )
        .await;
    let body = String::from_utf8(response.body).unwrap();
    assert!(body.contains("\"echo\":\"hello\""), "{body}");

    let server = host.serve().await.unwrap();
    assert!(started.load(Ordering::SeqCst));

    server.shutdown();
    server.wait().await.unwrap();
    assert!(stopped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn scoped_services_are_isolated_per_request() {
    let mut services = ServiceCollection::new();
    services.add_singleton(CounterService::default());
    services.add_scoped::<usize, _>(|scope| Ok(scope.resolve::<CounterService>()?.next()));
    let container = Arc::new(services.build());

    let first = container.create_scope().resolve::<usize>().unwrap();
    let second = container.create_scope().resolve::<usize>().unwrap();
    assert_ne!(*first, *second);
}
