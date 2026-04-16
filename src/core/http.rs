use std::collections::HashMap;
use std::fmt;

use serde::Serialize;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum Method {
    Get,
    Post,
    Put,
    Delete,
    Patch,
    Head,
    Options,
    Other(String),
}

impl Method {
    pub fn from_http_str(value: &str) -> Self {
        match value {
            "GET" => Self::Get,
            "POST" => Self::Post,
            "PUT" => Self::Put,
            "DELETE" => Self::Delete,
            "PATCH" => Self::Patch,
            "HEAD" => Self::Head,
            "OPTIONS" => Self::Options,
            other => Self::Other(other.to_string()),
        }
    }
}

impl fmt::Display for Method {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Method::Get => write!(f, "GET"),
            Method::Post => write!(f, "POST"),
            Method::Put => write!(f, "PUT"),
            Method::Delete => write!(f, "DELETE"),
            Method::Patch => write!(f, "PATCH"),
            Method::Head => write!(f, "HEAD"),
            Method::Options => write!(f, "OPTIONS"),
            Method::Other(value) => write!(f, "{value}"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Request {
    pub method: Method,
    pub path: String,
    pub version: String,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
    pub query_params: HashMap<String, Vec<String>>,
}

impl Request {
    pub fn body_as_string(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Request, ParseError> {
        let (head, body) = split_head_body(bytes);
        let head = std::str::from_utf8(head).map_err(|_| ParseError::InvalidUtf8)?;

        let mut lines = head.split("\r\n");
        let request_line = lines.next().ok_or(ParseError::MissingRequestLine)?;
        if request_line.trim().is_empty() {
            return Err(ParseError::MissingRequestLine);
        }
        let mut parts = request_line.split_whitespace();

        let method = parts
            .next()
            .ok_or(ParseError::InvalidRequestLine)
            .map(Method::from_http_str)?;
        let raw_path = parts
            .next()
            .ok_or(ParseError::InvalidRequestLine)?
            .to_string();
        let version = parts
            .next()
            .ok_or(ParseError::InvalidRequestLine)?
            .to_string();

        if parts.next().is_some() {
            return Err(ParseError::InvalidRequestLine);
        }
        if !version.starts_with("HTTP/") {
            return Err(ParseError::InvalidHttpVersion);
        }
        if !raw_path.starts_with('/') {
            return Err(ParseError::InvalidPath);
        }

        let mut headers = HashMap::new();
        for line in lines {
            if line.is_empty() {
                continue;
            }
            let (key, value) = line.split_once(':').ok_or(ParseError::InvalidHeaderLine)?;
            let key = key.trim();
            let value = value.trim();
            if key.is_empty() {
                return Err(ParseError::InvalidHeaderLine);
            }
            headers.insert(key.to_string(), value.to_string());
        }

        let (path, query_params) = if let Some((path, query)) = raw_path.split_once('?') {
            (path.to_string(), parse_query(query)?)
        } else {
            (raw_path, HashMap::new())
        };

        let body = if let Some(content_length) = header_value(&headers, "content-length") {
            let expected = content_length
                .parse::<usize>()
                .map_err(|_| ParseError::InvalidContentLength)?;
            if body.len() < expected {
                return Err(ParseError::BodyTooShort {
                    expected,
                    actual: body.len(),
                });
            }
            body[..expected].to_vec()
        } else {
            body.to_vec()
        };

        Ok(Request {
            method,
            path,
            version,
            headers,
            body,
            query_params,
        })
    }
}

fn split_head_body(bytes: &[u8]) -> (&[u8], &[u8]) {
    bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| (&bytes[..index], &bytes[index + 4..]))
        .unwrap_or((bytes, &[]))
}

fn header_value<'a>(headers: &'a HashMap<String, String>, key: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(header_key, _)| header_key.eq_ignore_ascii_case(key))
        .map(|(_, value)| value.as_str())
}

fn parse_query(query: &str) -> Result<HashMap<String, Vec<String>>, ParseError> {
    let mut params = HashMap::new();
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }

        let (raw_key, raw_value) = if let Some((key, value)) = pair.split_once('=') {
            (key, value)
        } else {
            (pair, "")
        };

        let key = percent_decode(raw_key)?;
        let value = percent_decode(raw_value)?;
        params.entry(key).or_insert_with(Vec::new).push(value);
    }
    Ok(params)
}

fn percent_decode(value: &str) -> Result<String, ParseError> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut idx = 0;

    while idx < bytes.len() {
        match bytes[idx] {
            b'+' => {
                decoded.push(b' ');
                idx += 1;
            }
            b'%' => {
                if idx + 2 >= bytes.len() {
                    return Err(ParseError::InvalidPercentEncoding);
                }

                let high = decode_hex(bytes[idx + 1])?;
                let low = decode_hex(bytes[idx + 2])?;
                decoded.push((high << 4) | low);
                idx += 3;
            }
            byte => {
                decoded.push(byte);
                idx += 1;
            }
        }
    }

    String::from_utf8(decoded).map_err(|_| ParseError::InvalidPercentEncoding)
}

fn decode_hex(byte: u8) -> Result<u8, ParseError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => Err(ParseError::InvalidPercentEncoding),
    }
}

#[derive(Clone, Debug)]
pub struct Response {
    pub status_code: u16,
    pub status_text: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    pub fn new(status_code: u16, status_text: impl Into<String>, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status_code,
            status_text: status_text.into(),
            headers: Vec::new(),
            body: body.into(),
        }
    }

    pub fn ok(body: impl Into<Vec<u8>>) -> Self {
        Self::new(200, "OK", body)
    }

    pub fn not_found() -> Self {
        Self::from_error(404, "Not Found", "404 Not Found")
    }

    pub fn bad_request(message: impl Into<Vec<u8>>) -> Self {
        let message = message.into();
        Self::from_error(
            400,
            "Bad Request",
            String::from_utf8_lossy(&message).into_owned(),
        )
    }

    pub fn internal_server_error() -> Self {
        Self::from_error(500, "Internal Server Error", "500 Internal Server Error")
    }

    pub fn from_error(
        status_code: u16,
        status_text: impl Into<String>,
        body: impl Into<String>,
    ) -> Self {
        Self::new(status_code, status_text, body.into().into_bytes())
            .with_header("Content-Type", "text/plain; charset=utf-8")
    }

    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((key.into(), value.into()));
        self
    }

    pub fn text(body: impl Into<String>) -> Self {
        Self::ok(body.into().into_bytes()).with_header("Content-Type", "text/plain; charset=utf-8")
    }

    pub fn html(body: impl Into<String>) -> Self {
        Self::ok(body.into().into_bytes()).with_header("Content-Type", "text/html; charset=utf-8")
    }

    pub fn json(body: impl Into<String>) -> Self {
        Self::ok(body.into().into_bytes())
            .with_header("Content-Type", "application/json; charset=utf-8")
    }

    pub fn json_value(value: serde_json::Value) -> Self {
        Self::json(value.to_string())
    }

    pub fn json_serialized<T: Serialize>(value: &T) -> Result<Self, serde_json::Error> {
        serde_json::to_vec(value).map(|body| {
            Self::ok(body).with_header("Content-Type", "application/json; charset=utf-8")
        })
    }

    pub fn to_http_bytes(&self) -> Vec<u8> {
        let mut response = format!("HTTP/1.1 {} {}\r\n", self.status_code, self.status_text);
        let mut has_content_length = false;
        let mut has_connection = false;

        for (key, value) in &self.headers {
            if key.eq_ignore_ascii_case("content-length") {
                has_content_length = true;
            }
            if key.eq_ignore_ascii_case("connection") {
                has_connection = true;
            }
            response.push_str(&format!("{key}: {value}\r\n"));
        }

        if !has_content_length {
            response.push_str(&format!("Content-Length: {}\r\n", self.body.len()));
        }
        if !has_connection {
            response.push_str("Connection: close\r\n");
        }

        response.push_str("\r\n");
        let mut bytes = response.into_bytes();
        bytes.extend_from_slice(&self.body);
        bytes
    }
}

#[derive(Debug)]
pub enum ParseError {
    MissingRequestLine,
    InvalidRequestLine,
    InvalidHttpVersion,
    InvalidPath,
    InvalidUtf8,
    InvalidHeaderLine,
    InvalidContentLength,
    InvalidPercentEncoding,
    RequestTooLarge { limit: usize },
    BodyTooShort { expected: usize, actual: usize },
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::MissingRequestLine => write!(f, "request line is missing"),
            ParseError::InvalidRequestLine => write!(f, "request line is invalid"),
            ParseError::InvalidHttpVersion => write!(f, "http version is invalid"),
            ParseError::InvalidPath => write!(f, "request path is invalid"),
            ParseError::InvalidUtf8 => write!(f, "request headers are not valid utf-8"),
            ParseError::InvalidHeaderLine => write!(f, "request header line is invalid"),
            ParseError::InvalidContentLength => write!(f, "content-length header is invalid"),
            ParseError::InvalidPercentEncoding => {
                write!(f, "request query percent-encoding is invalid")
            }
            ParseError::RequestTooLarge { limit } => {
                write!(f, "request exceeds maximum allowed size ({limit} bytes)")
            }
            ParseError::BodyTooShort { expected, actual } => write!(
                f,
                "request body is shorter than content-length (expected {expected}, got {actual})"
            ),
        }
    }
}

impl std::error::Error for ParseError {}
