use vantus::{HostBuilder, Path, Request, Response, module};

#[derive(Clone, Default)]
struct RoutingModule;

#[derive(Clone, Default)]
struct AlternateRoutingModule;

#[derive(Clone, Default)]
struct CanonicalRoutingModule;

#[derive(Clone, Default)]
struct MethodModule;

#[module]
impl RoutingModule {
    #[vantus::get("/users/{id}")]
    fn show(&self, id: Path<u32>) -> Response {
        Response::text(format!("user {}", id.into_inner()))
    }
}

#[module]
impl AlternateRoutingModule {
    #[vantus::get("/users/{name}")]
    fn show(&self, name: Path<String>) -> Response {
        Response::text(format!("user {}", name.into_inner()))
    }
}

#[module]
impl CanonicalRoutingModule {
    #[vantus::get("/teams//{id}//")]
    fn team(&self, id: Path<u32>) -> Response {
        Response::text(format!("team {}", id.into_inner()))
    }

    #[vantus::get("/users/me")]
    fn me(&self) -> Response {
        Response::text("me")
    }

    #[vantus::get("/users/{id}")]
    fn user(&self, id: Path<String>) -> Response {
        Response::text(format!("user {}", id.into_inner()))
    }
}

#[module]
impl MethodModule {
    #[vantus::get("/items")]
    fn list(&self) -> Response {
        Response::text("items")
    }

    #[vantus::post("/items")]
    fn create(&self) -> Response {
        Response::text("created")
    }
}

#[tokio::test]
async fn macro_registered_route_matches_path_params() {
    let mut builder = HostBuilder::new();
    builder.module(RoutingModule);
    let host = builder.build();

    let response = host
        .handle(Request::from_bytes(b"GET /users/42 HTTP/1.1\r\nHost: local\r\n\r\n").unwrap())
        .await;

    assert_eq!(response.status_code, 200);
    assert_eq!(String::from_utf8(response.body).unwrap(), "user 42");
}

#[tokio::test]
async fn request_paths_are_normalized_before_routing() {
    let mut builder = HostBuilder::new();
    builder.module(CanonicalRoutingModule);
    let host = builder.build();

    let response = host
        .handle(Request::from_bytes(b"GET /teams//42// HTTP/1.1\r\nHost: local\r\n\r\n").unwrap())
        .await;

    assert_eq!(response.status_code, 200);
    assert_eq!(String::from_utf8(response.body).unwrap(), "team 42");
}

#[tokio::test]
async fn static_routes_win_over_dynamic_routes() {
    let mut builder = HostBuilder::new();
    builder.module(CanonicalRoutingModule);
    let host = builder.build();

    let response = host
        .handle(Request::from_bytes(b"GET /users/me HTTP/1.1\r\nHost: local\r\n\r\n").unwrap())
        .await;

    assert_eq!(response.status_code, 200);
    assert_eq!(String::from_utf8(response.body).unwrap(), "me");
}

#[tokio::test]
async fn wrong_method_returns_405_and_allow_header() {
    let mut builder = HostBuilder::new();
    builder.module(MethodModule);
    let host = builder.build();

    let response = host
        .handle(Request::from_bytes(b"PUT /items HTTP/1.1\r\nHost: local\r\n\r\n").unwrap())
        .await;

    assert_eq!(response.status_code, 405);
    assert!(
        response
            .headers
            .iter()
            .any(|(key, value)| key == "Allow" && value == "GET, POST")
    );
}

#[test]
#[should_panic(expected = "Module registration failed")]
fn duplicate_routes_fail_with_build_error() {
    let mut builder = HostBuilder::new();
    builder.module(RoutingModule);
    builder.module(RoutingModule);
    builder.build();
}

#[test]
fn semantically_equivalent_parameterized_routes_conflict() {
    let mut builder = HostBuilder::new();
    builder.module(RoutingModule);
    builder.module(AlternateRoutingModule);

    let error = builder.try_build().err().expect("build should fail");
    let message = error.to_string();

    assert!(message.contains("duplicate route registration"));
    assert!(message.contains("/users/{id}"));
    assert!(message.contains("/users/{name}"));
}
