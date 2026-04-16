use std::fmt;

use crate::app::ServiceError;
use crate::config::ConfigError;
use crate::core::http::Response;
use crate::di::ExtractorError;

#[derive(Debug, Clone)]
pub enum FrameworkError {
    Http(HttpError),
    Internal(String),
}

impl FrameworkError {
    pub fn to_response(&self) -> Response {
        match self {
            FrameworkError::Http(error) => error.to_response(),
            FrameworkError::Internal(_) => Response::internal_server_error(),
        }
    }
}

impl fmt::Display for FrameworkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FrameworkError::Http(error) => write!(f, "{error}"),
            FrameworkError::Internal(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for FrameworkError {}

impl From<HttpError> for FrameworkError {
    fn from(value: HttpError) -> Self {
        Self::Http(value)
    }
}

impl From<ExtractorError> for FrameworkError {
    fn from(value: ExtractorError) -> Self {
        Self::Http(HttpError::bad_request(value.to_string()))
    }
}

impl From<ServiceError> for FrameworkError {
    fn from(value: ServiceError) -> Self {
        Self::Internal(value.to_string())
    }
}

impl From<ConfigError> for FrameworkError {
    fn from(value: ConfigError) -> Self {
        Self::Internal(value.to_string())
    }
}

#[derive(Debug, Clone)]
pub struct HttpError {
    pub status_code: u16,
    pub status_text: String,
    pub message: String,
}

impl HttpError {
    pub fn new(
        status_code: u16,
        status_text: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            status_code,
            status_text: status_text.into(),
            message: message.into(),
        }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(400, "Bad Request", message)
    }

    pub fn to_response(&self) -> Response {
        Response::from_error(self.status_code, &self.status_text, self.message.clone())
    }
}

impl fmt::Display for HttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.status_code, self.status_text)
    }
}
