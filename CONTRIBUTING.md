# Contributing to Vantus

Thanks for contributing to `vantus`.

This repository is a Rust backend framework with three closely related parts:

- `src/`: the main `vantus` framework crate
- `vantus_macros/`: the proc-macro crate that expands `#[module]`, `#[controller]`, and route attributes
- `docs/`: the VitePress documentation site published through GitHub Pages

The framework is intentionally opinionated: application dependencies belong on module/controller structs, while handler parameters are limited to request-derived extractors. If you are changing public behavior, please keep that design boundary intact.

## Before You Start

- Read [README.md](README.md) for the public framing.
- Read [docs/technical-deep-dive.md](docs/technical-deep-dive.md) if you are touching routing, extraction, macros, middleware, or runtime flow.
- Read [docs/publishing-checklist.md](docs/publishing-checklist.md) for release-sensitive changes.

## Repository Map

- `src/lib.rs`: public re-exports and the main surface area contributors should treat as user-facing API
- `src/app/`: host construction, module registration, lifecycle hooks, and framework modules
- `src/routing/`: route definitions, matching, request context, and handler plumbing
- `src/di/`: request extractors and request-scoped state/identity handling
- `src/runtime/`: Hyper integration, request normalization, server lifecycle, and rate limiting
- `src/config.rs`: layered config loading and binding
- `src/middleware.rs`: middleware traits, futures, and ordering
- `vantus_macros/src/lib.rs`: compile-time route parsing and contract enforcement
- `tests/`: integration, hardening, feature, and trybuild coverage
- `examples/`: runnable usage examples, including the optional CLI path
- `docs/`: end-user docs; keep these in sync with real framework behavior

## Local Setup

Rust:

- Use Rust `1.85` or newer. MSRV is enforced in CI.
- Install `rustfmt` and `clippy`.

Docs:

- The docs site uses Node `22` in CI.
- From the repo root, install docs dependencies with `npm --prefix docs ci`.

## Common Workflows

Framework checks:

```powershell
$env:CARGO_TARGET_DIR="target_plan"
cargo fmt
cargo test --lib --tests
cargo test --doc
cargo test --examples --no-run
cargo clippy --all-targets --all-features -- -D warnings
cargo doc --no-deps
```

Docs checks:

```powershell
npm --prefix docs ci
npm --prefix docs run build
```

Optional CLI checks:

```powershell
cargo test --features cli --test cli
cargo run --example cli --features cli -- --help
```

## Contribution Guidelines

### Keep the architecture explicit

- Prefer constructor injection on module/controller structs over adding runtime service lookup.
- Keep handler parameters limited to request-derived extractors such as `Path<T>`, `Query<T>`, `Header<T>`, `TextBody`, `JsonBody<T>`, `RequestState<T>`, and `IdentityState<T>`.
- Treat `HostBuilder` as the composition root; avoid scattering framework assembly logic across unrelated APIs.

### When changing macros

- Update `vantus_macros/src/lib.rs` and the corresponding compile-fail tests in `tests/ui/` together.
- Preserve the current contract checks around route syntax, path parameters, body extractors, and unsupported handler parameter types unless the change is intentional and documented.
- Add or update runtime-facing tests in `tests/macros_hardening.rs` when macro expansion changes behavior.

### When changing routing, runtime, or middleware

- Add integration coverage in `tests/router.rs`, `tests/http.rs`, `tests/security.rs`, `tests/module_hardening.rs`, or `tests/runtime_stress.rs` as appropriate.
- Be careful with request normalization, method-specific route resolution, and pre-handler safety checks. These are core framework guarantees.
- Preserve deterministic middleware ordering by stage, source, and registration order.

### When changing configuration or CLI behavior

- Update `tests/config.rs` or `tests/cli.rs`.
- Update [docs/configuration-reference.md](docs/configuration-reference.md) or [docs/cli-reference.md](docs/cli-reference.md) if the user-facing behavior changed.

### When changing docs

- Keep examples aligned with the actual API exposed by `src/lib.rs`.
- Prefer documenting behavior that is verified by tests or directly visible in the implementation.
- Rebuild the VitePress site locally before opening a PR.

## Testing Expectations

Aim to add the narrowest test that proves the behavior you changed:

- public API / integration behavior: `tests/*.rs`
- compile-time macro diagnostics: `tests/ui/*.rs` and `.stderr`
- examples still compile: `cargo test --examples --no-run`
- docs/site regressions: `npm --prefix docs run build`

If you change release packaging, metadata, or included files, make sure the release workflow assumptions in [Cargo.toml](Cargo.toml) and [docs/publishing-checklist.md](docs/publishing-checklist.md) still hold.

## Documentation and Release Hygiene

Please update these when they are affected by your change:

- [README.md](README.md)
- [CHANGELOG.md](CHANGELOG.md)
- [SECURITY.md](SECURITY.md)
- relevant files in `docs/`

Changes that alter public API, configuration, runtime guarantees, middleware ordering, observability behavior, or CLI flags should almost always include docs updates.

## Pull Requests

Good PRs in this repo usually:

- explain the behavior change and why it is needed
- call out affected areas such as macros, routing, runtime, config, or docs
- mention the verification commands you ran
- note any follow-up work or intentional limitations

Small, focused PRs are much easier to review than mixed refactors plus behavior changes.
