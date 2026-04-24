# Security Policy

## Supported Scope

The published crate is intended to be used behind a normal production edge:

- TLS termination at a load balancer, ingress, or reverse proxy
- authentication and authorization in application middleware
- CORS and compression configured explicitly by the application or proxy

The framework itself provides hardened request parsing, request-size limits, request deadlines, content-type enforcement, host-header validation, request IDs, structured logging hooks, and optional rate limiting.

## Reporting a Vulnerability

If you discover a security issue in `vantus`, please report it privately to the maintainer before opening a public issue.

Include:

- affected version or commit
- impact summary
- reproduction steps or proof of concept
- suggested remediation if you have one

## Release Hygiene

Before publishing a release:

- run `cargo test --lib --tests`
- run `cargo test --doc`
- run `cargo clippy --all-targets --all-features -- -D warnings`
- run `cargo doc --no-deps`
- run `cargo audit`
- run `cargo deny check`
- review docs for any newly added public API or behavior changes
