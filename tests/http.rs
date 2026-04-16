use vantus::{Method, ParseError, Request, Response};
use serde::Serialize;

#[test]
fn parses_simple_get_request_with_query() {
    let raw = b"GET /hello?name=world HTTP/1.1\r\nHost: example\r\n\r\n";
    let request = Request::from_bytes(raw).unwrap();
    assert_eq!(request.method, Method::Get);
    assert_eq!(request.path, "/hello");
}

#[test]
fn rejects_invalid_percent_encoding() {
    let err = Request::from_bytes(b"GET /bad?value=%ZZ HTTP/1.1\r\n\r\n").unwrap_err();
    assert!(matches!(err, ParseError::InvalidPercentEncoding));
}

#[test]
fn response_serializes_json() {
    #[derive(Serialize)]
    struct Payload<'a> {
        message: &'a str,
    }

    let response = Response::json_serialized(&Payload { message: "ok" }).unwrap();
    assert_eq!(
        String::from_utf8(response.body).unwrap(),
        "{\"message\":\"ok\"}"
    );
}
