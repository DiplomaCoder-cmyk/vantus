---
title: Quick Start
description: Build a small Vantus application with explicit module composition and typed request extraction.
---

# Quick Start

This guide walks through the shortest path from an empty crate to a running Vantus application.

## Mental Model

Build Vantus apps with three ideas in mind:

1. Define modules and controllers as ordinary Rust types.
2. Put long-lived dependencies on `self`, not in handler parameters.
3. Use handler parameters only for request-derived values.

## Minimal Application

```rust
use std::time::Duration;

use vantus::{HostBuilder, RequestContext, Response, TextBody, module};

#[derive(Clone)]
struct ApiModule {
    service_name: String,
}

#[module]
impl ApiModule {
    #[vantus::post("/hello")]
    fn hello(&self, ctx: RequestContext, body: TextBody) -> Response {
        Response::json_value(serde_json::json!({
            service: self.service_name.clone(),
            environment: ctx.app_config().environment,
            message: format!("hello, {}", body.as_str()),
        }))
    }
}

fn main() {
    let mut builder = HostBuilder::new();
    builder.request_timeout(Duration::from_secs(5));
    builder.max_body_size(64 * 1024);

    builder.compose_with_config(|_configuration, app, context| {
        context.module(ApiModule {
            service_name: app.service_name.clone(),
        });
        Ok(())
    });
    builder.build().run_blocking();
}
```

## What Happens Here

- `HostBuilder::new()` creates the composition root.
- `compose_with_config(...)` runs after framework config has been loaded and bound.
- The module is constructed explicitly with application data from `AppConfig`.
- The route macro generates the registration and extractor glue.
- `TextBody` is parsed from the request body, while `RequestContext` gives access to config and runtime data.
- `build().run_blocking()` finalizes the host and starts the runtime.

## Recommended Next Steps

1. Add `with_web_platform()` for `/health`, `/info`, request logging, panic recovery, and security headers.
2. Add `with_observability()` for `/live`, `/ready`, `/diag`, `/metrics`, and request IDs.
3. Apply runtime limits with `max_body_size(...)`, `request_timeout(...)`, and `rate_limiter(...)`.
4. Move app-specific module wiring into `compose_with_config(...)` if construction depends on configuration.

## Project Layout Pattern

```text
src/
  main.rs
  modules/
    api.rs
    admin.rs
  services/
    users.rs
    emails.rs
```

Keep application services in normal Rust modules, construct them in `main`, and inject them into Vantus modules explicitly.

## Continue Reading

- [API Reference](api-reference.md)
- [Technical Deep Dive](technical-deep-dive.md)
- [Configuration Reference](configuration-reference.md)
- [Extraction Reference](extraction-reference.md)
- [Production Notes](production-notes.md)

See [`examples/main.rs`](https://github.com/DiplomaCoder-cmyk/vantus/blob/main/examples/main.rs) for the runnable example in this repository.

For production deployment guidance, continue with [production-notes.md](production-notes.md) and [publishing-checklist.md](publishing-checklist.md).
