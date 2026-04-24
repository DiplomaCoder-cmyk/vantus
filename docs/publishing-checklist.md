# Publishing Checklist

Use this checklist before releasing `vantus`.

## Code and API

- confirm the public API compiles cleanly on the advertised MSRV
- remove accidental debug-only output or temporary diagnostics
- review breaking changes in macros, extractors, routing, and configuration
- make sure new public items have rustdoc or reference docs

## Runtime Safety

- verify startup failures roll back already-started modules cleanly
- verify graceful shutdown cancels background tasks before module teardown
- verify request limits, timeouts, and content-type guards still trigger before handlers
- review any in-memory caches or maps for unbounded growth risks

## Documentation

- update `README.md` quick-start snippets if the public API changed
- update `CHANGELOG.md`
- update `docs/configuration-reference.md` for new config keys or precedence rules
- update `docs/production-notes.md` with any new operational guidance
- update `SECURITY.md` if the support or disclosure policy changed

## Verification

```powershell
$env:CARGO_TARGET_DIR="target_plan"
cargo fmt
cargo test --lib --tests
cargo test --doc
cargo clippy --all-targets --all-features -- -D warnings
cargo doc --no-deps
```

## Release Readiness

- confirm package metadata, included files, and docs links are correct
- confirm examples still compile
- tag the release only after the changelog and docs are in sync
