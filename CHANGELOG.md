# Changelog

## 0.3.0

- made `vantus` and `vantus_macros` a breaking `0.3.0` release centered on explicit composition and typed request contracts
- removed dead runtime-DI compatibility artifacts from the published package, tests, and docs
- tightened the crates.io package surface with an explicit allowlist and release-oriented metadata
- added release verification coverage for doctests, examples, package dry runs, and property-based request checks
- documented the production model clearly: constructor injection, route contracts, UUID request IDs, tracing, metrics, and proxy-first edge concerns
