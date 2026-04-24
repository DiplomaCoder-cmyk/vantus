---
title: Technical Deep Dive
description: Architecture analysis of Vantus based on the actual backend implementation.
---

# Technical Deep Dive

This analysis is derived from the framework source itself: `src/app`, `src/routing`, `src/di`, `src/runtime`, `src/config`, and the `vantus_macros` proc-macro crate.

## Architectural Layers

Vantus is split into a few clean layers:

| Layer | Main files | Responsibility |
| --- | --- | --- |
| Public entrypoint | `src/lib.rs` | Re-exports the builder, macros, extractors, modules, runtime types, and configuration surface. |
| Composition + lifecycle | `src/app/application.rs`, `src/app/module.rs` | Host construction, module registration, lifecycle hooks, composition context, and host state. |
| Routing | `src/routing/*` | Route definition, path normalization, per-method indexes, request context, and boxed handlers. |
| Extraction / request DI | `src/di/mod.rs` | Typed handler inputs and request-local state / identity extraction. |
| Middleware | `src/middleware.rs`, `src/app/modules.rs`, `src/app/observability.rs` | Pipeline assembly, ordering, and framework-supplied middleware. |
| Runtime | `src/runtime/mod.rs` | Hyper integration, request normalization, timeouts, concurrency, rate limiting, and server lifecycle. |
| Configuration | `src/config.rs` | Layered config loading, parsing, binding, and server validation. |
| Macro expansion | `vantus_macros/src/lib.rs` | Generates `Module` implementations, route registration glue, and extractor calls. |

## Composition Model

### `HostBuilder` is the composition root

The framework centers everything around `HostBuilder`. It owns:

- the `Router`
- the global `MiddlewareStack`
- the list of `RuntimeModule`s
- the `ConfigurationBuilder`
- runtime security overrides like max body size, request timeout, and rate limiter
- infrastructure dependencies like `LogSink` and `IdGenerator`

That means application assembly happens in one explicit place rather than being spread across route handlers.

### Composition happens in two phases

1. Builder setup
   Add config sources, runtime overrides, framework modules, and application modules.
2. `build()` / `try_build()`
   Bind configuration, run composition hooks, validate runtime settings, and freeze the host state.

`compose_with_config(...)` is especially important. It lets the framework finish configuration binding before application modules are constructed, which keeps config-aware dependency wiring explicit and testable.

## Dependency Injection Story

Vantus uses explicit constructor injection, not a runtime service container.

### What that means in practice

- Application dependencies belong on module or controller structs.
- Those structs are constructed manually in normal Rust code.
- Handler parameters are reserved for request-derived values only.
- Middleware can attach request-scoped state or identity values through `RequestContext`.

This gives Vantus two DI modes:

| DI scope | Mechanism | Intended use |
| --- | --- | --- |
| Application lifetime | constructor injection on module/controller structs | services, repositories, domain clients, shared configuration |
| Request lifetime | `RequestContext::insert_state` / `insert_identity` | auth context, derived caller data, middleware-produced request metadata |

### Why the framework looks this way

The macro crate actively enforces this boundary. Unsupported handler parameter types fail at compile time, and the error message points callers back to request-derived extractors instead of allowing arbitrary service injection.

## Routing Architecture

### Route declarations

Route methods are declared inside `#[module]` or `#[controller]` impl blocks with attributes like `#[vantus::get("/users/{id}")]`.

At compile time, the proc macro:

- finds route methods
- validates the path template
- infers request body contracts from extractor usage
- generates extractor statements for each handler parameter
- wraps the method in a boxed `Handler`
- emits `RouteDefinition` registrations

### Route storage

At runtime, `Router` stores a `HashMap<Method, MethodRouteIndex>`. Each method index contains:

- a `matchit::Router<usize>` matcher
- a parallel `Vec<Route>` storing the route metadata

This makes route matching a two-step lookup:

1. select the method-specific index
2. resolve the normalized path inside `matchit`

If the path exists under a different method, Vantus returns `MethodNotAllowed` with a synthesized `Allow` header instead of collapsing that case into `404`.

### Path normalization rules

Incoming request paths are normalized before matching. That lets routes match consistently even when the incoming path contains redundant slashes or trailing slashes. Route templates are also normalized on registration, so route storage and lookup stay aligned.

## Request Lifecycle

The runtime request path is intentionally layered:

```text
socket accept
  -> Hyper request
  -> request normalization
  -> header and body validation
  -> pre-middleware max-body / rate-limit checks
  -> route resolution
  -> route contract enforcement
  -> ordered middleware execution
  -> handler execution
  -> response serialization back to Hyper
```

### Detailed flow

1. `serve(...)` accepts a TCP connection and wraps it with Hyper.
2. `normalize_request(...)` converts the Hyper request into Vantus `Request`.
3. Header limits, `Host` validation, duplicate `Content-Length` rejection, and body-size enforcement happen before routing.
4. `enforce_pre_middleware_limits(...)` applies global body-size and rate-limit checks.
5. `Router::resolve(...)` matches the method/path pair.
6. `enforce_route_contract(...)` validates whether the request body is allowed and whether its content type matches the inferred route contract.
7. `MiddlewareStack::execute(...)` merges global and route-local middleware and executes them in deterministic order.
8. The boxed handler runs with fully extracted arguments.
9. The final `Response` is translated back to a Hyper response.

## Middleware Ordering

Middleware ordering is one of the most important runtime details in this codebase.

### Ordering rules

Vantus sorts middleware by:

1. `MiddlewareStage`
2. source (`Global` before `Route`)
3. registration index

The built-in stages are:

- `Logging`
- `Recovery`
- `Auth`
- `Validation`
- `Response`

This gives you a predictable rule set:

- stage always wins over registration timing
- global middleware at the same stage runs before route-local middleware
- within the same stage and source, source order is preserved

### Built-in middleware

The framework ships with two main middleware stacks:

| Module | Middleware | Effect |
| --- | --- | --- |
| `WebPlatformModule` | `RequestLogger`, `PanicRecovery`, `SecurityHeaders` | baseline web safety and logging |
| `ObservabilityModule` | request IDs, metrics, structured logging | operational visibility and request correlation |

## Configuration Model

Configuration is layered but still explicit.

### Loading rules

`ConfigurationBuilder` resolves:

1. environment
2. profile
3. base config file
4. profile-specific config file
5. environment overrides

It can parse `.properties`, `.toml`, `.json`, and optionally YAML when the `yaml-config` feature is enabled.

### Binding rules

`AppConfig::from_configuration(...)` translates the layered key-value store into the built-in runtime model. After that:

- `RuntimeSettings::from_config(...)` validates the server options
- `HostBuilder` applies builder-level security overrides
- application code can bind additional config during `compose_with_config(...)`

This is a clean separation between framework config, application config, and runtime overrides.

## Observability and Runtime State

The observability design is additive rather than magical.

### Runtime metrics

`RuntimeState` tracks process-level counters such as:

- total requests
- active requests
- active connections
- timeout totals
- rate-limit rejections
- method-not-allowed totals
- content-type and body rejection totals

### `ObservabilityModule`

This module layers on:

- `X-Request-Id` generation
- structured request logging
- in-flight and route-level request metrics
- `/live`, `/ready`, `/diag`, and `/metrics`
- a readiness registry with async contributors

Crucially, the framework does not install a global tracing subscriber or exporter. That responsibility stays in the binary.

## Macro Expansion Strategy

The proc-macro crate is opinionated, but straightforward.

### What the macros generate

- `Module` implementations
- route registration glue
- extractor calls for handler arguments
- response conversion via `IntoResponse`
- `RuntimeModule` forwarding for `#[module]` impl blocks that declare lifecycle hooks

### What the macros intentionally reject

- bare route attributes outside `#[module]` / `#[controller]`
- invalid or duplicate path parameters
- multiple body extractors on one route
- body extractors on `GET`, `HEAD`, and `OPTIONS`
- handler parameter types outside the supported request-derived set

That keeps the ergonomic layer aligned with the runtime model instead of adding a second abstraction system on top of it.

## Notable Design Trade-Offs

### Strengths

- Very explicit composition model
- Strong compile-time guard rails around route definitions
- Clear request-lifetime vs application-lifetime dependency boundaries
- Good operational defaults without forcing a full platform opinion on the user

### Trade-offs

- Less magical than container-based frameworks, so more wiring happens in user code
- Middleware types must be `Default` to be attached declaratively with the macro attribute
- There is no built-in auth/CORS/compression/TLS story yet; those concerns are pushed to middleware or edge infrastructure

## When Vantus Fits Best

Vantus is a strong fit when you want:

- explicit application assembly
- compile-time routing and extraction guarantees
- a small internal surface area over Hyper
- request guards and operational endpoints without adopting a heavy inversion-of-control runtime

If your team prefers dynamic service lookup or convention-heavy controller injection, the framework will feel intentionally strict. That strictness is part of the design.
