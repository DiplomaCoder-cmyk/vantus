# Extraction Reference

Supported handler inputs:

- `RequestContext`
- `Path<T>`
- `Query<T>`
- `Option<Query<T>>`
- `Header<T>`
- `Option<Header<T>>`
- `QueryMap`
- `BodyBytes`
- `TextBody`
- `JsonBody<T>`
- `RequestState<T>`
- `Option<RequestState<T>>`
- `IdentityState<T>`
- `Option<IdentityState<T>>`

Behavior:

- invalid request binding maps to request-level `400` responses
- middleware can attach identity with `RequestContext::insert_identity(value)`
- middleware can attach typed per-request state with `RequestContext::insert_state(value)`
- cloned `RequestContext` values share request-local state and identity storage
- `JsonBody<T>` and `TextBody` participate in route contract checks before handler execution
- handler parameters are intentionally limited to request-derived values; application dependencies belong on `self`
