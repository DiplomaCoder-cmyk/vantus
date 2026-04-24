use serde::Serialize;
use vantus::{
    HostBuilder, Identity, IdentityState, Middleware, MiddlewareFuture, Request, RequestContext,
    Response, module,
};

#[derive(Clone, Debug, Serialize)]
struct TestIdentity {
    subject: String,
}

impl Identity for TestIdentity {}

#[derive(Default)]
struct TestIdentityMiddleware;

impl Middleware for TestIdentityMiddleware {
    fn handle(&self, ctx: RequestContext, next: vantus::__private::Next) -> MiddlewareFuture {
        Box::pin(async move {
            let subject = ctx
                .request()
                .header("x-subject")
                .unwrap_or("demo-user")
                .to_string();
            ctx.insert_identity(TestIdentity { subject });
            next.run(ctx).await
        })
    }
}

#[derive(Clone, Default)]
struct IdentityModule;

#[module]
impl IdentityModule {
    fn configure_middleware(&self, middleware: &mut vantus::__private::MiddlewareStack) {
        middleware.add(TestIdentityMiddleware);
    }

    #[vantus::get("/identity/required")]
    fn required(&self, identity: IdentityState<TestIdentity>) -> Response {
        Response::json_value(serde_json::json!({
            "subject": identity.as_ref().subject
        }))
    }

    #[vantus::get("/identity/optional")]
    fn optional(&self, identity: Option<IdentityState<TestIdentity>>) -> Response {
        let subject = identity
            .as_ref()
            .map(|identity| identity.as_ref().subject.as_str())
            .unwrap_or("anonymous");
        Response::text(subject.to_string())
    }
}

#[derive(Clone, Default)]
struct IdentityMissingModule;

#[module]
impl IdentityMissingModule {
    #[vantus::get("/identity/missing-required")]
    fn missing_required(&self, _identity: IdentityState<TestIdentity>) -> Response {
        Response::text("unreachable")
    }

    #[vantus::get("/identity/missing-optional")]
    fn missing_optional(&self, identity: Option<IdentityState<TestIdentity>>) -> Response {
        let subject = identity
            .as_ref()
            .map(|identity| identity.as_ref().subject.as_str())
            .unwrap_or("anonymous");
        Response::text(subject.to_string())
    }
}

#[tokio::test]
async fn identity_state_extracts_identity_inserted_by_middleware() {
    let mut builder = HostBuilder::new();
    builder.module(IdentityModule);
    let host = builder.build();

    let response = host
        .handle(
            Request::from_bytes(b"GET /identity/required HTTP/1.1\r\nX-Subject: alice\r\n\r\n")
                .unwrap(),
        )
        .await;

    assert_eq!(response.status_code, 200);
    assert_eq!(
        String::from_utf8(response.body).unwrap(),
        "{\"subject\":\"alice\"}"
    );
}

#[tokio::test]
async fn missing_required_identity_returns_bad_request() {
    let mut builder = HostBuilder::new();
    builder.module(IdentityMissingModule);
    let host = builder.build();

    let response = host
        .handle(Request::from_bytes(b"GET /identity/missing-required HTTP/1.1\r\n\r\n").unwrap())
        .await;

    assert_eq!(response.status_code, 400);
    assert!(
        String::from_utf8(response.body)
            .unwrap()
            .contains("request identity"),
    );
}

#[tokio::test]
async fn optional_identity_state_resolves_to_none_when_missing() {
    let mut builder = HostBuilder::new();
    builder.module(IdentityMissingModule);
    let host = builder.build();

    let response = host
        .handle(Request::from_bytes(b"GET /identity/missing-optional HTTP/1.1\r\n\r\n").unwrap())
        .await;

    assert_eq!(response.status_code, 200);
    assert_eq!(String::from_utf8(response.body).unwrap(), "anonymous");
}

#[tokio::test]
async fn optional_identity_state_can_read_inserted_identity() {
    let mut builder = HostBuilder::new();
    builder.module(IdentityModule);
    let host = builder.build();

    let response = host
        .handle(
            Request::from_bytes(b"GET /identity/optional HTTP/1.1\r\nX-Subject: bob\r\n\r\n")
                .unwrap(),
        )
        .await;

    assert_eq!(response.status_code, 200);
    assert_eq!(String::from_utf8(response.body).unwrap(), "bob");
}
