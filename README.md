# `vantus`

`vantus` is a macro-first async Rust web platform built around:

- `HostBuilder` for bootstrapping
- `#[module]` and `#[controller]` for application structure
- typed extraction for path/query/body/config/service inputs
- a lightweight DI container with singleton/scoped/transient lifetimes
- layered configuration binding
- an async runtime powered by `tokio` and `hyper`

Version `0.2.0` is a breaking macro-only release. Manual route construction and direct router registration are no longer part of the supported public application API.

## Quick start

```rust
use vantus::{
    AppConfig, Config, FrameworkError, HostBuilder, Response, Service, TextBody, module,
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
struct ApiModule;

#[module]
impl ApiModule {
    fn configure_services(
        &self,
        services: &mut vantus::ServiceCollection,
    ) -> Result<(), FrameworkError> {
        services.add_singleton(GreetingService);
        Ok(())
    }

    #[vantus::post("/greet")]
    fn greet(
        &self,
        config: Config<AppConfig>,
        service: Service<GreetingService>,
        body: TextBody,
    ) -> Result<Response, FrameworkError> {
        #[derive(Serialize)]
        struct Payload {
            service: String,
            message: String,
        }

        Response::json_serialized(&Payload {
            service: config.as_ref().service_name.clone(),
            message: service.as_ref().greet(body.as_str()),
        })
        .map_err(|error| FrameworkError::Internal(error.to_string()))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut builder = HostBuilder::new();
    builder.group("/api", |api| {
        api.module(ApiModule);
    });

    let host = builder.build()?;
    host.run().await?;
    Ok(())
}
```

See [examples/macro_controller.rs](examples/macro_controller.rs) and [src/main.rs](src/main.rs).

## Supported authoring model

- Use `HostBuilder` to configure files/env/profile loading and mount macro-defined modules.
- Use `#[module]` for service registration, lifecycle hooks, and module route groups.
- Use `#[controller]` for route-focused types when you want a separate controller abstraction.
- Use typed inputs like `Path<T>`, `Query<T>`, `TextBody`, `JsonBody<T>`, `Service<T>`, `Config<T>`, and `AppConfig`.

## Extraction model

The supported extraction inputs are:

- `RequestContext`
- `Path<T>`
- `Query<T>`
- `QueryMap`
- `TextBody`
- `BodyBytes`
- `JsonBody<T>`
- `Service<T>`
- `Config<T>`
- any `T: FromConfiguration`

## Configuration

`ConfigurationBuilder` merges:

- `application.properties`
- `application.{profile}.properties`
- environment variables using the configured prefix, default `APP_`

`AppConfig` provides the built-in production settings for:

- service name
- environment and profile
- info/readiness toggles
- server address
- request timeout
- graceful shutdown timeout
- max request bytes
- concurrency limit

## Default platform module

`WebPlatformModule` provides a production-oriented starter set:

- `/health`
- `/info`
- request logging
- panic recovery middleware

## Production notes

- This crate is async-first and intended to run on `tokio`.
- The public app-facing API is macro-first; internal routing primitives are reserved for framework internals and proc-macro expansion.
- `0.2.0` is a breaking pre-1.0 release. See [docs/migration-to-macros.md](docs/migration-to-macros.md) and [CHANGELOG.md](CHANGELOG.md).

## Verification

Release checks:

```bash
cargo fmt
cargo test
cargo clippy --all-targets --all-features -- -D warnings
cargo doc --no-deps
cargo package
cargo publish --dry-run
```
