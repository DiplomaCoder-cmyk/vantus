# Production Notes

- `mini_backend` is designed for async execution on `tokio`.
- `WebPlatformModule` gives a sensible default production baseline with health/info endpoints and panic recovery.
- Keep service registration deterministic and fail fast during startup.
- Use profile-specific config files and environment overrides for deployment differences.
- Treat the hidden internal routing APIs as unsupported implementation detail.
