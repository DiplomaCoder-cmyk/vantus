# Quick Start

1. Define a module or controller as a normal Rust type.
2. Put dependencies on `self` and construct that type explicitly.
3. Add route methods with `#[vantus::get]`, `#[vantus::post]`, and the other route macros.
4. Use `HostBuilder::compose_with_config(...)` when module construction depends on configuration.
5. Add runtime limits with `max_body_size(...)`, `request_timeout(...)`, and `rate_limiter(...)`.
6. Add `with_web_platform()` and `with_observability()` when you want the built-in platform and ops layers.
7. Build the host and call `run_blocking()`.

Example:

```rust
use serde::Serialize;
use vantus::{HostBuilder, RequestContext, TextBody, module};

#[derive(Clone)]
struct ApiModule {
    service_name: String,
}

#[derive(Serialize)]
struct GreetingPayload {
    service: String,
    message: String,
}

#[module]
impl ApiModule {
    #[vantus::post("/hello")]
    fn hello(&self, ctx: RequestContext, body: TextBody) -> GreetingPayload {
        GreetingPayload {
            service: self.service_name.clone(),
            message: format!("{}: {}", ctx.app_config().environment, body.as_str()),
        }
    }
}

fn main() {
    let mut builder = HostBuilder::new();
    builder.compose_with_config(|_configuration, app, context| {
        context.module(ApiModule {
            service_name: app.service_name.clone(),
        });
        Ok(())
    });
    builder.build().run_blocking();
}
```

See [../examples/main.rs](../examples/main.rs) for the runnable example.

For production deployment guidance, continue with [production-notes.md](production-notes.md) and [publishing-checklist.md](publishing-checklist.md).
