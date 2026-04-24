use std::collections::HashMap;
use std::fmt;
use std::fmt::Write as _;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;

use bytes::Bytes;
use http::{HeaderMap, HeaderName, HeaderValue};
use serde::Serialize;

use crate::{LogLevel, emit_default_log};

const MAX_HEADERS: usize = 100;
const MAX_QUERY_PARAMS: usize = 128;
const MAX_QUERY_VALUE_LEN: usize = 8_192;

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
    pub headers: HeaderMap,
    pub body: Bytes,
    pub query_params: HashMap<String, Vec<String>>,
    pub remote_addr: Option<SocketAddr>,
}

impl Request {
    pub fn body_str(&self) -> Option<&str> {
        std::str::from_utf8(self.body.as_ref()).ok()
    }

    pub fn body_as_string(&self) -> String {
        String::from_utf8_lossy(self.body.as_ref()).into_owned()
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Request, ParseError> {
        let (head, body) = split_head_body(bytes);
        let head = std::str::from_utf8(head).map_err(|_| ParseError::InvalidUtf8)?;

        let mut lines = head.split("\r\n");
        let request_line = lines.next().ok_or(ParseError::MissingRequestLine)?;
        if request_line.trim().is_empty() {
            return Err(ParseError::MissingRequestLine);
        }
        let (method, raw_path, version) = parse_request_line(request_line)?;

        if !matches!(version.as_str(), "HTTP/1.0" | "HTTP/1.1") {
            return Err(ParseError::InvalidHttpVersion);
        }
        let mut headers = HeaderMap::new();
        let mut header_count = 0usize;
        for line in lines {
            if line.is_empty() {
                continue;
            }
            header_count += 1;
            if header_count > MAX_HEADERS {
                return Err(ParseError::TooManyHeaders);
            }
            let (key, value) = line.split_once(':').ok_or(ParseError::InvalidHeaderLine)?;
            let key = key.trim();
            let value = value.trim();
            if key.is_empty() {
                return Err(ParseError::InvalidHeaderLine);
            }
            let key = HeaderName::try_from(key).map_err(|_| ParseError::InvalidHeaderLine)?;
            let value = HeaderValue::from_str(value).map_err(|_| ParseError::InvalidHeaderLine)?;
            headers.append(key, value);
        }

        let (raw_path, query_params) = if let Some((path, query)) = raw_path.split_once('?') {
            (path.to_string(), parse_query(query)?)
        } else {
            (raw_path, HashMap::new())
        };
        let path = normalize_request_path(&raw_path)?;

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
            Bytes::copy_from_slice(&body[..expected])
        } else {
            Bytes::copy_from_slice(body)
        };

        Ok(Request {
            method,
            path,
            version,
            headers,
            body,
            query_params,
            remote_addr: None,
        })
    }

    pub(crate) fn from_normalized_parts(
        method: Method,
        path: String,
        version: String,
        headers: HeaderMap,
        body: Bytes,
        query_params: HashMap<String, Vec<String>>,
        remote_addr: Option<SocketAddr>,
    ) -> Result<Request, ParseError> {
        let path = normalize_request_path(&path)?;
        Ok(Request {
            method,
            path,
            version,
            headers,
            body,
            query_params,
            remote_addr,
        })
    }

    pub(crate) fn parse_query(query: &str) -> Result<HashMap<String, Vec<String>>, ParseError> {
        parse_query(query)
    }

    pub fn client_ip(&self, trusted_proxies: &[IpAddr]) -> Option<IpAddr> {
        let remote_addr = self.remote_addr?;
        if !trusted_proxies.contains(&remote_addr.ip()) {
            return Some(remote_addr.ip());
        }

        let forwarded = header_values(&self.headers, "x-forwarded-for")
            .flat_map(|value| value.split(','))
            .filter_map(|item| item.trim().parse::<IpAddr>().ok())
            .collect::<Vec<_>>();

        for candidate in forwarded.into_iter().rev() {
            if !trusted_proxies.contains(&candidate) {
                return Some(candidate);
            }
        }

        Some(remote_addr.ip())
    }

    pub fn header(&self, key: &str) -> Option<&str> {
        self.headers.get(key).and_then(|value| value.to_str().ok())
    }

    pub fn header_values<'a>(&'a self, key: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        header_values(&self.headers, key)
    }
}

fn split_head_body(bytes: &[u8]) -> (&[u8], &[u8]) {
    bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| (&bytes[..index], &bytes[index + 4..]))
        .unwrap_or((bytes, &[]))
}

fn header_value<'a>(headers: &'a HeaderMap, key: &str) -> Option<&'a str> {
    headers.get(key).and_then(|value| value.to_str().ok())
}

fn header_values<'a>(headers: &'a HeaderMap, key: &'a str) -> impl Iterator<Item = &'a str> + 'a {
    headers
        .get_all(key)
        .iter()
        .filter_map(|value| value.to_str().ok())
}

fn parse_request_line(request_line: &str) -> Result<(Method, String, String), ParseError> {
    if request_line.contains('\t') {
        return Err(ParseError::InvalidRequestLine);
    }

    let mut parts = request_line.split(' ');
    let method = parts.next().ok_or(ParseError::InvalidRequestLine)?;
    let path = parts.next().ok_or(ParseError::InvalidRequestLine)?;
    let version = parts.next().ok_or(ParseError::InvalidRequestLine)?;

    if method.is_empty()
        || path.is_empty()
        || version.is_empty()
        || parts.next().is_some()
        || request_line.contains("  ")
    {
        return Err(ParseError::InvalidRequestLine);
    }

    Ok((
        Method::from_http_str(method),
        path.to_string(),
        version.to_string(),
    ))
}

fn parse_query(query: &str) -> Result<HashMap<String, Vec<String>>, ParseError> {
    let mut params = HashMap::new();
    let mut pair_count = 0usize;
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        pair_count += 1;
        if pair_count > MAX_QUERY_PARAMS {
            return Err(ParseError::TooManyQueryParams);
        }

        let (raw_key, raw_value) = if let Some((key, value)) = pair.split_once('=') {
            (key, value)
        } else {
            (pair, "")
        };

        let key = percent_decode(raw_key)?;
        let value = percent_decode(raw_value)?;
        if key.len() > MAX_QUERY_VALUE_LEN || value.len() > MAX_QUERY_VALUE_LEN {
            return Err(ParseError::QueryValueTooLong);
        }
        params.entry(key).or_insert_with(Vec::new).push(value);
    }
    Ok(params)
}

fn normalize_request_path(path: &str) -> Result<String, ParseError> {
    if !path.starts_with('/') || path.contains('\0') || path.contains('\\') {
        return Err(ParseError::InvalidPath);
    }

    let mut normalized_segments = Vec::new();
    for segment in path.split('/') {
        if segment.is_empty() {
            continue;
        }
        if segment == "." || segment == ".." {
            return Err(ParseError::PathTraversal);
        }
        normalized_segments.push(segment);
    }

    if normalized_segments.is_empty() {
        Ok("/".to_string())
    } else {
        Ok(format!("/{}", normalized_segments.join("/")))
    }
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
        let key = key.into();
        let value = value.into();

        match (
            HeaderName::from_bytes(key.as_bytes()),
            HeaderValue::from_str(&value),
        ) {
            (Ok(_), Ok(valid_value)) => {
                self.headers.push((key, value_from_header(valid_value)));
            }
            _ => emit_default_log(
                LogLevel::Warn,
                "vantus.http",
                &format!("ignored invalid response header: {}", key),
            ),
        }
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
        match serde_json::to_vec(&value) {
            Ok(body) => {
                Self::ok(body).with_header("Content-Type", "application/json; charset=utf-8")
            }
            Err(_) => Self::internal_server_error(),
        }
    }

    pub fn json_serialized<T: Serialize>(value: &T) -> Result<Self, serde_json::Error> {
        serde_json::to_vec(value).map(|body| {
            Self::ok(body).with_header("Content-Type", "application/json; charset=utf-8")
        })
    }

    pub fn to_http_bytes(&self) -> Vec<u8> {
        let mut response = String::with_capacity(64 + self.headers.len() * 32 + self.body.len());
        let _ = write!(
            response,
            "HTTP/1.1 {} {}\r\n",
            self.status_code, self.status_text
        );
        let mut has_content_length = false;
        let mut has_connection = false;

        for (key, value) in &self.headers {
            if HeaderName::from_bytes(key.as_bytes()).is_err()
                || HeaderValue::from_str(value).is_err()
            {
                emit_default_log(
                    LogLevel::Warn,
                    "vantus.http",
                    &format!(
                        "ignored invalid response header during serialization: {}",
                        key
                    ),
                );
                continue;
            }
            if key.eq_ignore_ascii_case("content-length") {
                has_content_length = true;
            }
            if key.eq_ignore_ascii_case("connection") {
                has_connection = true;
            }
            let _ = write!(response, "{key}: {value}\r\n");
        }

        if !has_content_length {
            let _ = write!(response, "Content-Length: {}\r\n", self.body.len());
        }
        if !has_connection {
            response.push_str("Connection: close\r\n");
        }

        response.push_str("\r\n");
        let mut bytes = response.into_bytes();
        bytes.extend_from_slice(&self.body);
        bytes
    }

    pub async fn file_async(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref();
        match tokio::fs::read(path).await {
            Ok(content) => {
                let mut res = Self::ok(content);

                if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
                    res = res.with_header("Content-Type", mime_for_ext(ext));
                }
                res
            }
            Err(_) => {
                emit_default_log(
                    LogLevel::Warn,
                    "vantus.http",
                    &format!("file not found at {:?}", path),
                );
                Self::not_found()
            }
        }
    }

    #[deprecated(note = "use Response::file_async instead")]
    pub fn file(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref().to_path_buf();

        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                emit_default_log(
                    LogLevel::Warn,
                    "vantus.http",
                    "Response::file is deprecated inside async runtimes; use Response::file_async",
                );
                match read_file_bytes_compat(&path, handle) {
                    Ok(content) => response_from_file_bytes(&path, content),
                    Err(_) => {
                        emit_default_log(
                            LogLevel::Warn,
                            "vantus.http",
                            &format!("file not found at {:?}", path),
                        );
                        Self::not_found()
                    }
                }
            }
            Err(_) => match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime.block_on(Self::file_async(path)),
                Err(_) => Self::internal_server_error(),
            },
        }
    }
}

fn read_file_bytes_compat(path: &Path, handle: tokio::runtime::Handle) -> std::io::Result<Vec<u8>> {
    tokio::task::block_in_place(|| {
        let path = path.to_path_buf();
        handle.block_on(async move {
            tokio::task::spawn_blocking(move || std::fs::read(path))
                .await
                .map_err(|error| std::io::Error::other(error.to_string()))?
        })
    })
}

fn response_from_file_bytes(path: &Path, content: Vec<u8>) -> Response {
    let mut res = Response::ok(content);
    if let Some(ext) = path.extension().and_then(|s| s.to_str()) {
        res = res.with_header("Content-Type", mime_for_ext(ext));
    }
    res
}

fn value_from_header(value: HeaderValue) -> String {
    value.to_str().map(str::to_string).unwrap_or_default()
}

fn mime_for_ext(ext: &str) -> &'static str {
    match ext {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "application/javascript; charset=utf-8",
        "html" | "htm" => "text/html; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

#[derive(Debug)]
pub enum ParseError {
    MissingRequestLine,
    InvalidRequestLine,
    InvalidHttpVersion,
    InvalidPath,
    PathTraversal,
    InvalidUtf8,
    InvalidHeaderLine,
    InvalidContentLength,
    InvalidPercentEncoding,
    TooManyHeaders,
    TooManyQueryParams,
    QueryValueTooLong,
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
            ParseError::PathTraversal => write!(f, "request path contains traversal sequences"),
            ParseError::InvalidUtf8 => write!(f, "request headers are not valid utf-8"),
            ParseError::InvalidHeaderLine => write!(f, "request header line is invalid"),
            ParseError::InvalidContentLength => write!(f, "content-length header is invalid"),
            ParseError::InvalidPercentEncoding => {
                write!(f, "request query percent-encoding is invalid")
            }
            ParseError::TooManyHeaders => {
                write!(
                    f,
                    "request contains too many headers (limit: {MAX_HEADERS})"
                )
            }
            ParseError::TooManyQueryParams => write!(
                f,
                "request contains too many query parameters (limit: {MAX_QUERY_PARAMS})"
            ),
            ParseError::QueryValueTooLong => write!(
                f,
                "query key or value exceeds maximum length ({MAX_QUERY_VALUE_LEN} bytes)"
            ),
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
