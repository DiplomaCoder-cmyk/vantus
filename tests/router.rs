use vantus::{HostBuilder, Path, Response, module};

#[derive(Clone, Default)]
struct RoutingModule;

#[module]
impl RoutingModule {
    #[vantus::get("/users/{id}")]
    fn show(&self, id: Path<u32>) -> Response {
        Response::text(format!("user {}", id.into_inner()))
    }
}

#[tokio::test]
async fn macro_registered_route_matches_path_params() {
    let mut builder = HostBuilder::new();
    builder.module(RoutingModule);
    let host = builder.build().unwrap();

    let response = host
        .handle(vantus::Request::from_bytes(b"GET /users/42 HTTP/1.1\r\n\r\n").unwrap())
        .await;

    assert_eq!(response.status_code, 200);
    assert_eq!(String::from_utf8(response.body).unwrap(), "user 42");
}
