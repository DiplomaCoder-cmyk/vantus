use http::HeaderMap;
use serde::Serialize;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// Coarse log level used by the framework's default sink abstraction.
pub enum LogLevel {
    Info,
    Warn,
    Error,
}

impl LogLevel {
    fn as_str(self) -> &'static str {
        match self {
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
/// Structured request log payload emitted by framework middleware.
pub struct RequestLogEvent {
    /// Request identifier added by request-ID middleware when available.
    pub request_id: Option<String>,
    /// HTTP method after request normalization.
    pub method: String,
    /// Sanitized request path suitable for logs and metrics labels.
    pub path: String,
    /// Final response status code observed by middleware.
    pub status_code: u16,
    /// End-to-end middleware/handler latency in milliseconds.
    pub duration_ms: u128,
    /// Best-effort resolved client IP, honoring trusted proxy settings.
    pub client_ip: Option<String>,
    /// Redacted headers when a caller chooses to include them.
    pub headers: Vec<(String, String)>,
}

/// Destination for text and structured request logs emitted by the framework.
pub trait LogSink: Send + Sync {
    /// Writes a framework text log after secret-scrubbing the message.
    fn log_text(&self, level: LogLevel, target: &str, message: &str);

    /// Writes a structured request event.
    fn log_request(&self, target: &str, event: &RequestLogEvent);
}

#[derive(Default)]
/// Default sink that writes JSON to stdout/stderr without installing tracing globals.
pub struct StdIoLogSink;

impl LogSink for StdIoLogSink {
    fn log_text(&self, level: LogLevel, target: &str, message: &str) {
        let event = TextLogEvent {
            level: level.as_str(),
            target,
            message: sanitize_log_message(message),
        };
        if let Ok(serialized) = serde_json::to_string(&event) {
            match level {
                LogLevel::Error => eprintln!("{serialized}"),
                LogLevel::Info | LogLevel::Warn => println!("{serialized}"),
            }
        } else {
            let line = format!("[{}] {}: {}", level.as_str(), target, event.message);
            match level {
                LogLevel::Error => eprintln!("{line}"),
                LogLevel::Info | LogLevel::Warn => println!("{line}"),
            }
        }
    }

    fn log_request(&self, target: &str, event: &RequestLogEvent) {
        if let Ok(serialized) = serde_json::to_string(event) {
            println!("[INFO] {target}: {serialized}");
        } else {
            println!(
                "[INFO] {target}: {} {} -> {}",
                event.method, event.path, event.status_code
            );
        }
    }
}

pub fn emit_default_log(level: LogLevel, target: &str, message: &str) {
    StdIoLogSink.log_text(level, target, message);
}

/// Redacts sensitive header values before they are copied into logs.
pub fn redact_headers(headers: &HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(key, value)| {
            let key_name = key.as_str();
            let redacted = if is_sensitive_header(key_name) {
                "<redacted>".to_string()
            } else {
                value.to_str().unwrap_or_default().to_string()
            };
            (key_name.to_string(), redacted)
        })
        .collect()
}

/// Scrubs path segments that look like identifiers before logging.
pub fn sanitize_path_for_logs(path: &str) -> String {
    let segments = path
        .split('/')
        .map(|segment| {
            if segment.is_empty() {
                String::new()
            } else if segment.len() > 32
                || segment.chars().all(|ch| ch.is_ascii_digit())
                || looks_like_identifier(segment)
            {
                ":redacted".to_string()
            } else {
                segment.to_string()
            }
        })
        .collect::<Vec<_>>();
    let sanitized = segments.join("/");
    if sanitized.len() > 128 {
        format!("{}...", &sanitized[..128])
    } else {
        sanitized
    }
}

/// Best-effort secret scrubbing for ad-hoc log messages.
pub fn sanitize_log_message(message: &str) -> String {
    let mut sanitized = message.to_string();
    for key in [
        "token",
        "secret",
        "passwd",
        "password",
        "cookie",
        "authorization",
    ] {
        sanitized = redact_assignment_value(&sanitized, key);
    }
    sanitized
}

#[derive(Serialize)]
struct TextLogEvent<'a> {
    level: &'a str,
    target: &'a str,
    message: String,
}

fn is_sensitive_header(header_name: &str) -> bool {
    let header_name = header_name.to_ascii_lowercase();
    header_name.contains("authorization")
        || header_name.contains("cookie")
        || header_name.contains("token")
        || header_name.contains("secret")
        || matches!(header_name.as_str(), "x-api-key" | "api-key")
}

fn looks_like_identifier(segment: &str) -> bool {
    let compact = segment.replace('-', "");
    compact.len() >= 16 && compact.chars().all(|ch| ch.is_ascii_hexdigit())
}

fn redact_assignment_value(input: &str, key: &str) -> String {
    let lowercase = input.to_ascii_lowercase();
    let mut sanitized = String::with_capacity(input.len());
    let mut cursor = 0usize;

    while let Some(relative_index) = lowercase[cursor..].find(key) {
        let key_start = cursor + relative_index;
        let separator_index = key_start + key.len();
        let separator = lowercase[separator_index..].chars().next();
        if !matches!(separator, Some('=') | Some(':')) {
            sanitized.push_str(&input[cursor..separator_index]);
            cursor = separator_index;
            continue;
        }

        sanitized.push_str(&input[cursor..=separator_index]);
        let mut value_start = separator_index + 1;
        while let Some(ch) = input[value_start..].chars().next() {
            if ch.is_whitespace() {
                sanitized.push(ch);
                value_start += ch.len_utf8();
            } else {
                break;
            }
        }

        let mut value_end = value_start;
        while let Some(ch) = input[value_end..].chars().next() {
            if matches!(ch, ',' | ';' | '&') {
                break;
            }
            value_end += ch.len_utf8();
        }

        sanitized.push_str("<redacted>");
        cursor = value_end;
    }

    sanitized.push_str(&input[cursor..]);
    sanitized
}

#[cfg(test)]
mod tests {
    use super::{redact_headers, sanitize_log_message, sanitize_path_for_logs};

    #[test]
    fn redacts_sensitive_headers() {
        let mut headers = http::HeaderMap::new();
        headers.insert("authorization", "Bearer secret-token".parse().unwrap());
        headers.insert("x-api-key", "super-secret".parse().unwrap());
        headers.insert("accept", "application/json".parse().unwrap());

        let redacted = redact_headers(&headers);
        assert!(
            redacted
                .iter()
                .any(|(key, value)| key == "authorization" && value == "<redacted>")
        );
        assert!(
            redacted
                .iter()
                .any(|(key, value)| key == "x-api-key" && value == "<redacted>")
        );
        assert!(
            redacted
                .iter()
                .any(|(key, value)| key == "accept" && value == "application/json")
        );
    }

    #[test]
    fn sanitizes_sensitive_log_message_values() {
        let sanitized = sanitize_log_message(
            "login failed authorization: Bearer abc123 password=hunter2 token=secret",
        );
        assert!(!sanitized.contains("abc123"));
        assert!(!sanitized.contains("hunter2"));
        assert!(!sanitized.contains("secret"));
        assert!(sanitized.contains("<redacted>"));
    }

    #[test]
    fn sanitizes_log_paths() {
        let sanitized =
            sanitize_path_for_logs("/users/123456/orders/0123456789abcdef0123456789abcdef");
        assert_eq!(sanitized, "/users/:redacted/orders/:redacted");
    }
}
