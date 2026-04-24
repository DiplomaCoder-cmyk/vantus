use proptest::prelude::*;
use vantus::{Header, HostBuilder, Query, Request, Response, module};

#[derive(Clone, Default)]
struct PropertyModule;

#[module]
impl PropertyModule {
    #[vantus::get("/query")]
    fn query(&self, name: Query<String>) -> Response {
        Response::text(name.into_inner())
    }

    #[vantus::get("/header")]
    fn header(&self, trace_id: Header<String>) -> Response {
        Response::text(trace_id.into_inner())
    }

    #[vantus::get("/items/{id}")]
    fn item(&self, id: vantus::Path<String>) -> Response {
        Response::text(id.into_inner())
    }
}

fn handle(raw: String) -> Response {
    let mut builder = HostBuilder::new();
    builder.module(PropertyModule);
    let host = builder.build();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime");

    runtime.block_on(async {
        host.handle(Request::from_bytes(raw.as_bytes()).expect("request parses"))
            .await
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    #[test]
    fn request_parser_round_trips_query_values(value in "[a-z0-9]{1,12}") {
        let raw = format!("GET /query?name={value} HTTP/1.1\r\nHost: local\r\n\r\n");
        let request = Request::from_bytes(raw.as_bytes()).expect("request parses");
        prop_assert_eq!(
            request.query_params.get("name").and_then(|values| values.first()).map(String::as_str),
            Some(value.as_str())
        );
    }

    #[test]
    fn typed_header_extractor_round_trips_values(value in "[a-z0-9-]{1,16}") {
        let raw = format!("GET /header HTTP/1.1\r\ntrace-id: {value}\r\n\r\n");
        let response = handle(raw);
        prop_assert_eq!(response.status_code, 200);
        prop_assert_eq!(String::from_utf8(response.body).expect("utf8 body"), value);
    }

    #[test]
    fn normalized_paths_resolve_to_the_same_route(id in "[a-z0-9]{1,12}") {
        let raw = format!("GET //items//{id}// HTTP/1.1\r\n\r\n");
        let response = handle(raw);
        prop_assert_eq!(response.status_code, 200);
        prop_assert_eq!(String::from_utf8(response.body).expect("utf8 body"), id);
    }
}
