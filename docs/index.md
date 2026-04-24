---
layout: home

hero:
  name: Vantus
  text: Build Rust Backends With Fewer Hidden Rules
  tagline: Macro-first routing, request-derived handler inputs, and hardened HTTP defaults without a runtime service locator or mystery dependency graph.
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

## Choose Your Track

- [Quick Start](/quick-start): Build a small service first and get comfortable with Vantus composition and request extraction.
- [Technical Deep Dive](/technical-deep-dive): Inspect the request pipeline, route contracts, middleware ordering, and runtime behavior.
- [Production Notes](/production-notes): Review the operational defaults, deployment boundaries, and safety guidance before shipping.

This site was assembled from the actual Vantus source tree, not just the README. The goal is to make the framework legible from the outside: what you construct explicitly, what the macros generate, and what guarantees are enforced before handler code runs.

If you are new to the project, start with [Quick Start](/quick-start). If you are evaluating the framework design, jump to [Technical Deep Dive](/technical-deep-dive). If you need a surface map of the public types and extension points, use [API Reference](/api-reference).

## Why Teams Reach For Vantus

- `Application wiring`: Build services with normal Rust constructors and mount them from `HostBuilder`.
- `Handler signatures`: Keep them request-focused with `Path<T>`, `Query<T>`, `Header<T>`, `TextBody`, `JsonBody<T>`, and typed request state.
- `Middleware behavior`: Run it in a deterministic stage order so auth, validation, recovery, and response shaping stay inspectable.
- `Operational defaults`: Enforce limits, timeouts, contract checks, and secure headers before business logic.
- `Macro usage`: Use proc macros for route registration ergonomics without hiding the runtime model.

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

## Reading Order

1. [Quick Start](/quick-start) for the first runnable module.
2. [API Reference](/api-reference) for the public surface area.
3. [Technical Deep Dive](/technical-deep-dive) for internal behavior and design tradeoffs.
4. [Production Notes](/production-notes) and [Publishing Checklist](/publishing-checklist) before shipping.
