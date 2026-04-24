use thiserror::Error;

use crate::config::ConfigError;
use crate::core::http::Response;
use crate::di::ExtractorError;

#[derive(Debug, Error)]
pub enum FrameworkError {
    #[error(transparent)]
    Http(#[from] HttpError),
    #[error("{context}: {source}")]
    Config {
        context: &'static str,
        #[source]
        source: ConfigError,
    },
    #[error(transparent)]
    Extractor(#[from] ExtractorError),
    #[error(
        "duplicate route registration for {method} {new_path} (conflicts with {existing_path})"
    )]
    RouteConflict {
        method: String,
        existing_path: String,
        new_path: String,
    },
    #[error("{context}")]
    Startup { context: String },
    #[error("{message}")]
    Internal { message: String },
}

impl FrameworkError {
    pub fn to_response(&self) -> Response {
        match self {
            FrameworkError::Http(error) => error.to_response(),
            FrameworkError::Extractor(source) => {
                HttpError::bad_request(source.to_string()).to_response()
            }
            FrameworkError::Config { .. }
            | FrameworkError::RouteConflict { .. }
            | FrameworkError::Startup { .. }
            | FrameworkError::Internal { .. } => Response::internal_server_error(),
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal {
            message: message.into(),
        }
    }

    pub fn startup(context: impl Into<String>) -> Self {
        Self::Startup {
            context: context.into(),
        }
    }

    pub fn config_context(context: &'static str, source: ConfigError) -> Self {
        Self::Config { context, source }
    }

    pub fn route_conflict(
        method: impl Into<String>,
        existing_path: impl Into<String>,
        new_path: impl Into<String>,
    ) -> Self {
        Self::RouteConflict {
            method: method.into(),
            existing_path: existing_path.into(),
            new_path: new_path.into(),
        }
    }
}

#[derive(Debug, Clone, Error)]
#[error("{status_code} {status_text}")]
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

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(404, "Not Found", message)
    }

    pub fn method_not_allowed(message: impl Into<String>) -> Self {
        Self::new(405, "Method Not Allowed", message)
    }

    pub fn payload_too_large(message: impl Into<String>) -> Self {
        Self::new(413, "Payload Too Large", message)
    }

    pub fn unsupported_media_type(message: impl Into<String>) -> Self {
        Self::new(415, "Unsupported Media Type", message)
    }

    pub fn too_many_requests(message: impl Into<String>) -> Self {
        Self::new(429, "Too Many Requests", message)
    }

    pub fn internal_server_error(message: impl Into<String>) -> Self {
        Self::new(500, "Internal Server Error", message)
    }

    pub fn to_response(&self) -> Response {
        Response::from_error(self.status_code, &self.status_text, self.message.clone())
    }
}
