use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::thread;

use vantus::{
    AtomicIdGenerator, HostBuilder, IdGenerator, RequestContext, Response, UuidIdGenerator, module,
};

struct FixedIdGenerator {
    value: &'static str,
}

impl IdGenerator for FixedIdGenerator {
    fn next_id(&self) -> String {
        self.value.to_string()
    }
}

#[derive(Clone, Default)]
struct IdProbeModule;

#[module]
impl IdProbeModule {
    #[vantus::get("/generated")]
    fn generated(&self, ctx: RequestContext) -> Response {
        Response::text(ctx.id_generator().next_id())
    }
}

#[test]
fn atomic_generator_returns_distinct_prefixed_ids() {
    let generator = AtomicIdGenerator::with_prefix("order");
    let first = generator.next_id();
    let second = generator.next_id();

    assert_ne!(first, second);
    assert!(first.starts_with("order-"), "{first}");
    assert!(second.starts_with("order-"), "{second}");
}

#[test]
fn uuid_generator_returns_parseable_distinct_ids() {
    let generator = UuidIdGenerator;
    let first = generator.next_id();
    let second = generator.next_id();

    assert_ne!(first, second);
    assert!(uuid::Uuid::parse_str(&first).is_ok(), "{first}");
    assert!(uuid::Uuid::parse_str(&second).is_ok(), "{second}");
}

#[test]
fn atomic_generator_is_collision_free_under_concurrency() {
    let generator = Arc::new(AtomicIdGenerator::with_prefix("concurrent"));
    let values = Arc::new(Mutex::new(Vec::new()));

    thread::scope(|scope| {
        for _ in 0..8 {
            let generator = Arc::clone(&generator);
            let values = Arc::clone(&values);
            scope.spawn(move || {
                for _ in 0..32 {
                    values.lock().unwrap().push(generator.next_id());
                }
            });
        }
    });

    let values = values.lock().unwrap();
    let unique = values.iter().cloned().collect::<HashSet<_>>();
    assert_eq!(values.len(), unique.len());
}

#[tokio::test]
async fn builder_can_inject_custom_id_generator_into_routes() {
    let mut builder = HostBuilder::new();
    builder.id_generator(FixedIdGenerator { value: "custom-42" });
    builder.module(IdProbeModule);
    let host = builder.build();

    let response = host
        .handle(
            vantus::Request::from_bytes(b"GET /generated HTTP/1.1\r\nHost: local\r\n\r\n").unwrap(),
        )
        .await;

    assert_eq!(response.status_code, 200);
    assert_eq!(String::from_utf8(response.body).unwrap(), "custom-42");
}
