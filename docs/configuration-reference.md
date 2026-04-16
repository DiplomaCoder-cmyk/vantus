# Configuration Reference

Configuration is loaded through `ConfigurationBuilder` using:

- `application.properties`
- `application.{profile}.properties`
- environment overrides with prefix `APP_` by default

Built-in `AppConfig` fields cover:

- `service.name`
- `service.enable-info`
- `service.readiness`
- `server.address`
- `server.request-timeout-seconds`
- `server.graceful-shutdown-seconds`
- `server.max-request-bytes`
- `server.concurrency-limit`
