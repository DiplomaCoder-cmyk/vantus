---
layout: home

hero:
  name: Vantus
  text: Explicit Rust Backend Composition
  tagline: Macro-first routing, request-derived handler inputs, and hardened HTTP defaults without a runtime service locator.
  image:
    src: /vantus-mark.svg
    alt: Vantus
  actions:
    - theme: brand
      text: Quick Start
      link: /quick-start
    - theme: alt
      text: API Reference
      link: /api-reference
    - theme: alt
      text: Technical Deep Dive
      link: /technical-deep-dive

features:
  - title: Composition Root First
    details: HostBuilder is the application root. Modules are constructed explicitly and mounted intentionally, with config-aware composition handled in one place.
  - title: Typed Request Extraction
    details: Handler parameters are limited to request-derived inputs like Path<T>, Query<T>, Header<T>, TextBody, JsonBody<T>, RequestState<T>, and IdentityState<T>.
  - title: Guard Rails Before Handlers
    details: Request size limits, timeouts, route-contract checks, header validation, method matching, and optional rate limiting all run before handler logic.
  - title: Middleware With Deterministic Ordering
    details: Middleware is ordered by stage first and source second, so logging, recovery, auth, validation, and response shaping stay predictable.
  - title: Built-In Platform Modules
    details: WebPlatformModule and ObservabilityModule provide health, info, readiness, request IDs, metrics, structured logging, and safety defaults.
  - title: Macro Ergonomics, Runtime Transparency
    details: The proc-macro crate generates route registration and extraction glue, but the runtime stays close to plain Rust traits and data structures.
---

## What This Site Covers

This documentation site was generated from a full read of the Vantus backend codebase, not just the public README. The architecture pages reflect how the framework actually composes modules, builds route indexes, threads request state, and executes middleware at runtime.

If you are new to the project, start with [Quick Start](/quick-start). If you are evaluating the framework design, jump to [Technical Deep Dive](/technical-deep-dive). If you need a surface map of the public types and extension points, use [API Reference](/api-reference).

## Architecture Snapshot

```text
HostBuilder
  -> ConfigurationBuilder + AppConfig binding
  -> module registration + composition hooks
  -> Router + MiddlewareStack + HostState
  -> ApplicationHost / ServerHandle

Incoming request
  -> Hyper normalization
  -> body/header validation
  -> rate limit + size guards
  -> route resolution
  -> route contract enforcement
  -> ordered middleware pipeline
  -> handler extraction + response conversion
```

## Core Principles

- Dependencies live on module structs, not in handler parameters.
- Configuration is layered, bound once, and passed downward explicitly.
- Routing and extraction rules are inferred from macro-decorated impl methods.
- Middleware is powerful, but its ordering rules are stable and inspectable.
- Operational behavior stays explicit: no surprise tracing subscriber, auth layer, or global container appears behind the scenes.
