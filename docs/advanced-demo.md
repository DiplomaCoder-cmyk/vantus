# Advanced Demo Status

`vantus` `0.3.0` no longer ships the older multi-module demo that relied on the removed runtime-DI model.

The supported release example is [`examples/main.rs`](https://github.com/DiplomaCoder-cmyk/vantus/blob/main/examples/main.rs), which shows the current public architecture:

- `HostBuilder` bootstrapping
- `compose_with_config(...)` for explicit construction
- constructor-injected modules
- typed request extractors
- `with_web_platform()` and `with_observability()` for the first-party production layers
- builder-level request hardening with body size, timeout, and rate limiting

Run it from the repository root:

```powershell
cargo run --example main
```

If you need a larger sample application, build on that example rather than reviving the archived DI-era demo layout. The framework release intentionally treats TLS termination, CORS, compression, and authentication as proxy or middleware concerns until those features are designed as first-class APIs.
