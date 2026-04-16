# Extraction Reference

Supported handler inputs:

- `RequestContext`
- `Path<T>`
- `Query<T>`
- `QueryMap`
- `TextBody`
- `BodyBytes`
- `JsonBody<T>`
- `Service<T>`
- `Config<T>`
- any `T: FromConfiguration`

Behavior:

- invalid request binding maps to framework errors and request-level bad input responses
- services resolve through the built-in container
- config-backed types bind from layered configuration sources
