# CLI Reference

`vantus` now includes an optional first-party CLI bootstrap layer behind the `cli` feature.
The runnable entrypoint lives in `examples/cli.rs`.

This CLI is for runtime concerns:

- choosing config file, environment, and profile
- enabling or disabling the built-in web platform and observability modules
- setting request body and timeout overrides
- configuring the in-memory global rate limiter
- selecting the shared request-ID generator
- previewing the startup plan with `--print-startup-plan` or `--dry-run`

Startup plans annotate whether each value came from the CLI, a runtime mode preset, a feature default, or still depends on config/default resolution.

It does not control Cargo build profiles. Use:

- `cargo run` for debug/dev binaries
- `cargo run --release` for optimized release binaries

## Example

```powershell
cargo run --example cli --features cli -- `
  --mode production `
  --config examples/application.properties `
  --web-platform `
  --observability `
  --request-timeout-ms 5000 `
  --max-body-bytes 65536 `
  --rate-limit-capacity 120 `
  --rate-limit-refill-tokens 120 `
  --rate-limit-refill-seconds 60
```

## Runtime Mode

`--mode development` and `--mode production` apply runtime defaults for:

- environment/profile if not explicitly set
- request timeout if not explicitly set
- max body size if not explicitly set

Explicit CLI flags still win over the mode preset.

## Recommended Flags

- `--config <PATH>` to pin the config file used in production
- `--environment <NAME>` and `--profile <NAME>` when you need runtime separation
- `--observability` for `/live`, `/ready`, `/diag`, and `/metrics`
- `--web-platform` for `/health`, `/info`, security headers, and panic recovery
- `--dry-run` in CI or release automation to validate startup intent before launch
