# Production Notes

- Prefer constructor injection and explicit `compose_with_config(...)` composition over runtime lookup.
- `WebPlatformModule` provides `/health`, `/info`, panic recovery, security headers, and request logging.
- `ObservabilityModule` provides request IDs, `/live`, `/ready`, `/diag`, `/metrics`, and readiness contributors.
- The default request ID generator is `UuidIdGenerator`; use `AtomicIdGenerator` only for tests and local demos.
- Startup rollback now stops already-started modules and cancels framework background tasks if a later module fails during `on_start`.
- Route contracts reject malformed bodies and wrong media types before handler execution.
- HTTP/1.1 requests require a valid `Host` header.
- Use `HostBuilder::max_body_size(...)`, `request_timeout(...)`, and `rate_limiter(...)` for hard runtime limits enforced before middleware.
- `RuntimeState` exposes counters for totals, timeouts, rate limits, method-not-allowed responses, and body/content-type rejections.
- The in-memory global rate limiter now opportunistically prunes stale fully-refilled IP buckets to avoid unbounded growth on long-lived public servers.
- Install your own `tracing-subscriber` / exporter setup in the binary; the framework keeps observability explicit and does not register global tracing state.
- Treat TLS termination, CORS, compression, and authentication as edge concerns for now. Put them in middleware or a front proxy rather than assuming built-in first-party support.
- CI runs `cargo audit` and `cargo deny`, and Dependabot is configured for weekly Cargo and GitHub Actions updates.
- Review [publishing-checklist.md](publishing-checklist.md) and [`SECURITY.md`](https://github.com/DiplomaCoder-cmyk/vantus/blob/main/SECURITY.md) before every release.
- Suggested dependency remediation policy:
  - critical advisories: patch immediately
  - high severity: patch within 7 days
  - moderate severity: patch within 30 days
