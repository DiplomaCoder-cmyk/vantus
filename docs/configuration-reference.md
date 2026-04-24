# Configuration Reference

Configuration is loaded through `ConfigurationBuilder` using:

- `application.properties`
- `application.{profile}.properties`
- environment overrides with prefix `APP_` by default

The effective precedence is:

1. base config file
2. profile-specific config file
3. environment overrides

Environment overrides are applied last and win even when the file-based key uses a different alias form such as hyphenated vs dotted segments.

Built-in `AppConfig` fields cover:

- `service.name`
- `service.enable-info`
- `service.readiness`
- `server.address`
- `server.bind-host`
- `server.bind-port`
- `server.protocol`
- `server.request-timeout-seconds`
- `server.read-timeout-seconds`
- `server.handler-timeout-seconds`
- `server.graceful-shutdown-seconds`
- `server.max-request-bytes`
- `server.max-header-bytes`
- `server.max-header-count`
- `server.concurrency-limit`
- `server.keep-alive`
- `server.tcp-nodelay`
- `server.trusted-proxies`

`HostBuilder` can override selected runtime security limits after config binding:

- `HostBuilder::max_body_size(bytes)` overrides `server.max-request-bytes`
- `HostBuilder::request_timeout(duration)` overrides the outer request deadline
- `HostBuilder::rate_limiter(...)` adds a builder-managed pre-middleware global rate limiter

For custom configuration types:

- implement `FromConfiguration` for full manual control
- or call `Configuration::bind::<T>()` for serde-backed binding
- bind application-specific config during `compose_with_config(...)`
- pass the resulting values into modules and controllers explicitly
