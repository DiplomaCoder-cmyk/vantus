# Migration to `vantus` 0.3

`vantus` `0.3.0` is the breaking production-hardening release for the macro-first model.

## What changed

- Manual route registration is no longer part of the supported public application API.
- Runtime DI-style handler injection was removed from the supported model.
- `HostBuilder` remains the bootstrap entrypoint, but application dependencies are now composed explicitly.

## New authoring model

- `#[module]` and `#[controller]` define routes.
- Dependencies live on `self`.
- `RequestContext` gives handlers access to framework-managed runtime state such as `AppConfig`.
- `compose_with_config(...)` constructs configuration-aware modules before the host is finalized.

## Example migration

```rust
#[derive(Clone)]
struct EchoController {
    service_name: String,
}

#[vantus::controller]
impl EchoController {
    #[vantus::post("/echo")]
    fn echo(&self, body: vantus::TextBody) -> vantus::Response {
        vantus::Response::json_value(serde_json::json!({
            "service": self.service_name,
            "echo": body.as_str(),
        }))
    }
}
```

Removed from the supported public model:

- `configure_services`
- `Service<T>`
- `Config<T>`
- `bind_config(...)`
- `bind_sub_config(...)`
- direct application use of internal routing types
