use crate::HostError;
use crate::config::ConfigError;
use crate::core::FrameworkError;
use crate::core::http::ParseError;

impl From<ParseError> for FrameworkError {
    fn from(source: ParseError) -> Self {
        Self::Http(crate::HttpError::bad_request(source.to_string()))
    }
}

impl From<ConfigError> for FrameworkError {
    fn from(value: ConfigError) -> Self {
        Self::Startup {
            context: format!("configuration binding failed: {}", value),
        }
    }
}

impl From<HostError> for FrameworkError {
    fn from(value: HostError) -> Self {
        Self::internal(value.to_string())
    }
}
