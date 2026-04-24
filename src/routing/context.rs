use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::Arc;

use dashmap::DashMap;

use crate::app::HostState;
use crate::config::{AppConfig, Configuration};
use crate::core::errors::FrameworkError;
use crate::core::http::Request;
use crate::di::ExtractorError;
use crate::id::IdGenerator;
use crate::logging::LogSink;
use crate::runtime::RuntimeState;

/// Marker trait for request-scoped caller identity payloads inserted by middleware.
pub trait Identity: Send + Sync + 'static {}

#[derive(Clone)]
/// Per-request framework context passed to handlers and middleware.
///
/// This is the main bridge between incoming HTTP data and framework features:
/// - request inspection
/// - path params
/// - typed request state
/// - explicit host state access
pub struct RequestContext {
    request: Request,
    route_pattern: Arc<str>,
    path_params: HashMap<String, String>,
    host: Arc<HostState>,
    extensions: Arc<DashMap<TypeId, Arc<dyn Any + Send + Sync>>>,
}

impl RequestContext {
    pub(crate) fn new(
        request: Request,
        route_pattern: impl Into<Arc<str>>,
        path_params: HashMap<String, String>,
        host: Arc<HostState>,
    ) -> Self {
        Self {
            request,
            route_pattern: route_pattern.into(),
            path_params,
            host,
            extensions: Arc::new(DashMap::new()),
        }
    }

    pub fn request(&self) -> &Request {
        &self.request
    }

    pub fn route_pattern(&self) -> &str {
        self.route_pattern.as_ref()
    }

    pub fn path_params(&self) -> &HashMap<String, String> {
        &self.path_params
    }

    pub fn configuration(&self) -> &Configuration {
        self.host.configuration()
    }

    pub fn app_config(&self) -> &AppConfig {
        self.host.app_config()
    }

    pub fn runtime_state(&self) -> Arc<RuntimeState> {
        self.host.runtime_state()
    }

    pub fn log_sink(&self) -> Arc<dyn LogSink> {
        self.host.log_sink()
    }

    pub fn id_generator(&self) -> Arc<dyn IdGenerator> {
        self.host.id_generator()
    }

    /// Inserts request-local data for downstream middleware/handlers.
    pub fn insert_extension<T>(&self, value: T)
    where
        T: Send + Sync + 'static,
    {
        self.extensions.insert(TypeId::of::<T>(), Arc::new(value));
    }

    /// Preferred helper for storing typed request state.
    pub fn insert_state<T>(&self, value: T)
    where
        T: Send + Sync + 'static,
    {
        self.insert_extension(value);
    }

    /// Stores identity payload for downstream handlers and middleware.
    pub fn insert_identity<T>(&self, value: T)
    where
        T: Identity,
    {
        self.insert_extension(value);
    }

    /// Reads request-local data if it has been inserted earlier in the pipeline.
    pub fn extension<T>(&self) -> Option<Arc<T>>
    where
        T: Send + Sync + 'static,
    {
        self.extensions
            .get(&TypeId::of::<T>())
            .and_then(|value| Arc::clone(value.value()).downcast::<T>().ok())
    }

    /// Preferred helper for reading typed request state.
    pub fn state<T>(&self) -> Option<Arc<T>>
    where
        T: Send + Sync + 'static,
    {
        self.extension::<T>()
    }

    /// Reads identity payload if it has been inserted earlier in the pipeline.
    pub fn identity<T>(&self) -> Option<Arc<T>>
    where
        T: Identity,
    {
        self.extension::<T>()
    }

    /// Reads typed request state and returns a framework error if it is missing.
    pub fn require_state<T>(&self) -> Result<Arc<T>, FrameworkError>
    where
        T: Send + Sync + 'static,
    {
        self.state::<T>().ok_or_else(|| {
            ExtractorError::Missing(format!("request state {}", std::any::type_name::<T>())).into()
        })
    }

    /// Reads identity payload and returns a framework error if it is missing.
    pub fn require_identity<T>(&self) -> Result<Arc<T>, FrameworkError>
    where
        T: Identity,
    {
        self.identity::<T>().ok_or_else(|| {
            ExtractorError::Missing(format!("request identity {}", std::any::type_name::<T>()))
                .into()
        })
    }
}
