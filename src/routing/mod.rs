use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::app::{Config, Service, ServiceContainer, ServiceScope};
use crate::config::{Configuration, FromConfiguration};
use crate::core::errors::FrameworkError;
use crate::core::http::{Method, Request, Response};
use crate::middleware::Middleware;

pub type HandlerFuture = Pin<Box<dyn Future<Output = HandlerResult> + Send>>;
pub type HandlerResult = Result<Response, FrameworkError>;

#[derive(Clone)]
pub struct Handler(Arc<dyn Fn(RequestContext) -> HandlerFuture + Send + Sync>);

impl Handler {
    pub fn new<F, Fut>(handler: F) -> Self
    where
        F: Fn(RequestContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = HandlerResult> + Send + 'static,
    {
        Self(Arc::new(move |ctx| Box::pin(handler(ctx))))
    }

    pub async fn call(&self, ctx: RequestContext) -> HandlerResult {
        (self.0)(ctx).await
    }
}

pub trait RouteRegistrar {
    fn add_route(&mut self, definition: RouteDefinition) -> Result<(), FrameworkError>;
}

pub trait RouteRegistration {
    fn register_definition(&mut self, definition: RouteDefinition) -> &mut Self;
}

#[derive(Clone)]
pub struct RequestContext {
    request: Request,
    path_params: HashMap<String, String>,
    scope: Arc<ServiceScope>,
    configuration: Arc<Configuration>,
}

impl RequestContext {
    pub fn new(
        request: Request,
        path_params: HashMap<String, String>,
        services: Arc<ServiceContainer>,
        configuration: Arc<Configuration>,
    ) -> Self {
        let scope = Arc::new(services.create_scope());
        Self {
            request,
            path_params,
            scope,
            configuration,
        }
    }

    pub fn request(&self) -> &Request {
        &self.request
    }

    pub fn path_params(&self) -> &HashMap<String, String> {
        &self.path_params
    }

    pub fn scope(&self) -> Arc<ServiceScope> {
        Arc::clone(&self.scope)
    }

    pub fn service<T>(&self) -> Result<Service<T>, FrameworkError>
    where
        T: Send + Sync + 'static,
    {
        self.scope
            .resolve::<T>()
            .map(Service::from)
            .map_err(FrameworkError::from)
    }

    pub fn config<T>(&self) -> Result<Config<T>, FrameworkError>
    where
        T: Send + Sync + 'static,
    {
        self.scope
            .resolve::<T>()
            .map(Config::from)
            .map_err(FrameworkError::from)
    }

    pub fn configuration(&self) -> &Configuration {
        self.configuration.as_ref()
    }

    pub fn bind_config<T>(&self) -> Result<T, FrameworkError>
    where
        T: FromConfiguration,
    {
        T::from_configuration(self.configuration.as_ref()).map_err(FrameworkError::from)
    }
}

pub struct Router {
    routes: Vec<Route>,
}

impl Router {
    pub fn new() -> Self {
        Self { routes: Vec::new() }
    }

    pub fn add_definition(&mut self, definition: RouteDefinition) {
        self.ensure_unique_route(&definition.method, &definition.path);
        self.routes.push(Route::from_definition(definition));
    }

    pub fn route(&self, method: &Method, path: &str) -> Option<RouteMatch> {
        for route in &self.routes {
            if &route.method != method {
                continue;
            }

            if let Some(path_params) = route.template.match_path(path) {
                return Some(RouteMatch {
                    handler: route.handler.clone(),
                    middleware: route.middleware.clone(),
                    path_params,
                });
            }
        }
        None
    }

    fn ensure_unique_route(&self, method: &Method, template: &str) {
        if self
            .routes
            .iter()
            .any(|route| &route.method == method && route.template.raw == template)
        {
            panic!("duplicate route registration for {method} {template}");
        }
    }
}

impl Default for Router {
    fn default() -> Self {
        Self::new()
    }
}

impl RouteRegistrar for Router {
    fn add_route(&mut self, definition: RouteDefinition) -> Result<(), FrameworkError> {
        self.add_definition(definition);
        Ok(())
    }
}

#[derive(Clone)]
pub struct RouteDefinition {
    pub method: Method,
    pub path: String,
    pub handler: Handler,
    pub middleware: Vec<Arc<dyn Middleware>>,
}

impl RouteDefinition {
    pub fn new(method: Method, path: impl Into<String>, handler: Handler) -> Self {
        Self {
            method,
            path: path.into(),
            handler,
            middleware: Vec::new(),
        }
    }

    pub fn with_middleware(mut self, middleware: Vec<Arc<dyn Middleware>>) -> Self {
        self.middleware = middleware;
        self
    }
}

#[derive(Clone)]
pub struct RouteMatch {
    pub handler: Handler,
    pub middleware: Vec<Arc<dyn Middleware>>,
    pub path_params: HashMap<String, String>,
}

struct Route {
    method: Method,
    template: PathTemplate,
    handler: Handler,
    middleware: Vec<Arc<dyn Middleware>>,
}

impl Route {
    fn from_definition(definition: RouteDefinition) -> Self {
        Self {
            method: definition.method,
            template: PathTemplate::new(definition.path),
            handler: definition.handler,
            middleware: definition.middleware,
        }
    }
}

struct PathTemplate {
    raw: String,
    segments: Vec<PathSegment>,
}

impl PathTemplate {
    fn new(raw: String) -> Self {
        let segments = raw
            .trim_matches('/')
            .split('/')
            .filter(|segment| !segment.is_empty())
            .map(PathSegment::from)
            .collect();
        Self { raw, segments }
    }

    fn match_path(&self, candidate: &str) -> Option<HashMap<String, String>> {
        let candidate_segments: Vec<&str> = candidate
            .trim_matches('/')
            .split('/')
            .filter(|segment| !segment.is_empty())
            .collect();

        if self.segments.len() != candidate_segments.len() {
            return None;
        }

        let mut params = HashMap::new();
        for (segment, candidate) in self.segments.iter().zip(candidate_segments.iter()) {
            match segment {
                PathSegment::Literal(value) if value != candidate => return None,
                PathSegment::Literal(_) => {}
                PathSegment::Param(name) => {
                    params.insert(name.clone(), (*candidate).to_string());
                }
            }
        }

        Some(params)
    }
}

enum PathSegment {
    Literal(String),
    Param(String),
}

impl From<&str> for PathSegment {
    fn from(value: &str) -> Self {
        if value.starts_with('{') && value.ends_with('}') {
            Self::Param(value.trim_matches(|c| c == '{' || c == '}').to_string())
        } else {
            Self::Literal(value.to_string())
        }
    }
}
