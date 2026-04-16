# Migration to `vantus` 0.2

`vantus` `0.2.0` is a breaking macro-only release.

## What changed

- Manual route registration is no longer part of the supported public application API.
- Public wrapper modules for older compatibility shapes were removed.
- `HostBuilder` remains the application bootstrap entrypoint, but route definitions are expected to come from macro-defined modules/controllers.

## Migrate application code

### Before

- explicit router/route-definition construction
- direct builder route registration
- parallel public wrapper modules

### After

- `#[module]` for service registration and route grouping
- `#[controller]` for controller-style route impls
- `HostBuilder` for startup, config loading, and module mounting

## Example migration

```rust
use vantus::{
    AppConfig, Config, FrameworkError, HostBuilder, Response, Service, TextBody, module,
};

#[derive(Default)]
struct EchoService;

impl EchoService {
    fn echo(&self, value: &str) -> String {
        value.to_string()
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
        services.add_singleton(EchoService);
        Ok(())
    }

    #[vantus::post("/echo")]
    fn echo(
        &self,
        service: Service<EchoService>,
        config: Config<AppConfig>,
        body: TextBody,
    ) -> Result<Response, FrameworkError> {
        Ok(Response::json_value(serde_json::json!({
            "service": config.as_ref().service_name,
            "echo": service.as_ref().echo(body.as_str())
        })))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut builder = HostBuilder::new();
    builder.group("/api", |api| {
        api.module(ApiModule);
    });

    builder.build()?.run().await?;
    Ok(())
}
```

## Supported hooks

The supported macro authoring model is:

- `configure_services`
- route methods annotated with `#[get]`, `#[post]`, `#[put]`, `#[delete]`
- `on_start`
- `on_stop`

If you were relying on older public routing internals directly, treat them as removed and migrate to macro-defined modules/controllers.
