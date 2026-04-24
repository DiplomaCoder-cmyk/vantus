use std::sync::Arc;

use crate::core::errors::FrameworkError;
use crate::core::http::Method;
use crate::middleware::Middleware;

use super::Handler;
use super::path::normalize_path_template;

#[derive(Clone)]
/// Internal route declaration produced by macro-generated module/controller code.
pub struct RouteDefinition {
    pub method: Method,
    pub path: String,
    pub handler: Handler,
    pub middleware: Vec<Arc<dyn Middleware>>,
    pub contract: RouteContract,
}

impl RouteDefinition {
    pub fn new(method: Method, path: impl Into<String>, handler: Handler) -> Self {
        Self {
            method,
            path: path.into(),
            handler,
            middleware: Vec::new(),
            contract: RouteContract::default(),
        }
    }

    pub fn with_middleware(mut self, middleware: Vec<Arc<dyn Middleware>>) -> Self {
        self.middleware = middleware;
        self
    }

    pub fn with_contract(mut self, contract: RouteContract) -> Self {
        self.contract = contract;
        self
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RequestBodyKind {
    #[default]
    None,
    Bytes,
    Text,
    Json,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RouteContract {
    pub body: RequestBodyKind,
}

impl RouteContract {
    pub const fn new(body: RequestBodyKind) -> Self {
        Self { body }
    }

    pub const fn allows_body(self) -> bool {
        !matches!(self.body, RequestBodyKind::None)
    }

    pub const fn required_content_type(self) -> Option<&'static str> {
        match self.body {
            RequestBodyKind::Text => Some("text/plain"),
            RequestBodyKind::Json => Some("application/json"),
            RequestBodyKind::None | RequestBodyKind::Bytes => None,
        }
    }
}

pub(crate) struct Route {
    pub(crate) method: Method,
    pub(crate) path: String,
    pub(crate) route_pattern: Arc<str>,
    pub(crate) handler: Handler,
    pub(crate) middleware: Vec<Arc<dyn Middleware>>,
    pub(crate) contract: RouteContract,
}

impl Route {
    pub(crate) fn from_definition(definition: RouteDefinition) -> Result<Self, FrameworkError> {
        let normalized_path = normalize_path_template(&definition.path)?;
        Ok(Self {
            method: definition.method,
            path: normalized_path.clone(),
            route_pattern: Arc::<str>::from(normalized_path),
            handler: definition.handler,
            middleware: definition.middleware,
            contract: definition.contract,
        })
    }
}
