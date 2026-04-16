use std::fmt;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::runtime::Runtime;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::app::module::RuntimeModule;
use crate::app::modules::WebPlatformModule;
use crate::app::state::{ServiceCollection, ServiceContainer, ServiceError};
use crate::config::{
    AppConfig, ConfigError, Configuration, ConfigurationBuilder, FromConfiguration,
};
use crate::core::errors::FrameworkError;
use crate::core::http::{Request, Response};
use crate::middleware::MiddlewareStack;
use crate::routing::{RequestContext, RouteDefinition, RouteRegistrar, RouteRegistration, Router};
use crate::runtime::{RuntimeSettings, ServerHandle, serve};

type ConfigBinder =
    Box<dyn Fn(&Configuration, &mut ServiceCollection) -> Result<(), HostBuildError> + Send + Sync>;

#[derive(Clone)]
pub struct BackgroundTasks {
    cancellation: CancellationToken,
    handles: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

impl BackgroundTasks {
    pub fn new(cancellation: CancellationToken) -> Self {
        Self {
            cancellation,
            handles: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    pub async fn spawn<F>(&self, future: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.handles.lock().await.push(tokio::spawn(future));
    }

    pub async fn shutdown(&self) {
        self.cancellation.cancel();
        let handles = {
            let mut guard = self.handles.lock().await;
            std::mem::take(&mut *guard)
        };
        for handle in handles {
            let _ = handle.await;
        }
    }
}

#[derive(Clone)]
pub struct HostContext {
    services: Arc<ServiceContainer>,
    configuration: Arc<Configuration>,
    background_tasks: BackgroundTasks,
}

impl HostContext {
    pub fn configuration(&self) -> &Configuration {
        self.configuration.as_ref()
    }

    pub fn services(&self) -> Arc<ServiceContainer> {
        Arc::clone(&self.services)
    }

    pub fn service_scope(&self) -> crate::app::state::ServiceScope {
        self.services.create_scope()
    }

    pub fn background_tasks(&self) -> &BackgroundTasks {
        &self.background_tasks
    }
}

pub struct ApplicationHost {
    router: Arc<Router>,
    middleware: Arc<MiddlewareStack>,
    services: Arc<ServiceContainer>,
    modules: Vec<Arc<dyn RuntimeModule>>,
    configuration: Arc<Configuration>,
    runtime_settings: RuntimeSettings,
}

impl ApplicationHost {
    pub fn context(&self) -> HostContext {
        HostContext {
            services: Arc::clone(&self.services),
            configuration: Arc::clone(&self.configuration),
            background_tasks: BackgroundTasks::new(CancellationToken::new()),
        }
    }

    pub async fn serve(self) -> Result<ServerHandle, HostError> {
        let cancellation = CancellationToken::new();
        let background_tasks = BackgroundTasks::new(cancellation.clone());
        let context = HostContext {
            services: Arc::clone(&self.services),
            configuration: Arc::clone(&self.configuration),
            background_tasks: background_tasks.clone(),
        };

        for module in &self.modules {
            module
                .on_start(&context)
                .await
                .map_err(HostError::Framework)?;
        }

        serve(
            self.router,
            self.middleware,
            self.services,
            self.modules,
            self.configuration,
            self.runtime_settings,
            context,
        )
        .await
    }

    pub async fn run(self) -> Result<(), HostError> {
        self.serve().await?.wait().await
    }

    pub async fn handle(&self, request: Request) -> Response {
        let Some(route) = self.router.route(&request.method, &request.path) else {
            return Response::not_found();
        };

        let ctx = RequestContext::new(
            request,
            route.path_params,
            Arc::clone(&self.services),
            Arc::clone(&self.configuration),
        );
        match self
            .middleware
            .execute(&route.middleware, ctx, route.handler)
            .await
        {
            Ok(response) => response,
            Err(error) => error.to_response(),
        }
    }

    pub fn blocking_run(self) -> Result<(), HostError> {
        Runtime::new()
            .map_err(HostError::Io)?
            .block_on(async move { self.run().await })
    }
}

pub struct HostBuilder {
    router: Router,
    middleware: MiddlewareStack,
    modules: Vec<Arc<dyn RuntimeModule>>,
    services: ServiceCollection,
    configuration: ConfigurationBuilder,
    binders: Vec<ConfigBinder>,
}

impl HostBuilder {
    pub fn new() -> Self {
        let mut builder = Self {
            router: Router::new(),
            middleware: MiddlewareStack::new(),
            modules: Vec::new(),
            services: ServiceCollection::new(),
            configuration: ConfigurationBuilder::new(),
            binders: Vec::new(),
        };
        builder.bind_config::<AppConfig>();
        builder
    }

    pub fn config_file(&mut self, path: impl Into<PathBuf>) -> &mut Self {
        self.configuration.config_file(path);
        self
    }

    pub fn environment(&mut self, name: impl Into<String>) -> &mut Self {
        self.configuration.environment(name);
        self
    }

    pub fn profile(&mut self, profile: impl Into<String>) -> &mut Self {
        self.configuration.profile(profile);
        self
    }

    pub fn env_prefix(&mut self, prefix: impl Into<String>) -> &mut Self {
        self.configuration.env_prefix(prefix);
        self
    }

    pub fn service_singleton<T>(&mut self, value: T) -> &mut Self
    where
        T: Send + Sync + 'static,
    {
        self.services.add_singleton(value);
        self
    }

    pub fn service_singleton_with<T, F>(&mut self, factory: F) -> &mut Self
    where
        T: Send + Sync + 'static,
        F: Fn(&crate::app::state::ServiceScope) -> Result<T, ServiceError> + Send + Sync + 'static,
    {
        self.services.add_singleton_with(factory);
        self
    }

    pub fn service_scoped<T, F>(&mut self, factory: F) -> &mut Self
    where
        T: Send + Sync + 'static,
        F: Fn(&crate::app::state::ServiceScope) -> Result<T, ServiceError> + Send + Sync + 'static,
    {
        self.services.add_scoped(factory);
        self
    }

    pub fn service_transient<T, F>(&mut self, factory: F) -> &mut Self
    where
        T: Send + Sync + 'static,
        F: Fn(&crate::app::state::ServiceScope) -> Result<T, ServiceError> + Send + Sync + 'static,
    {
        self.services.add_transient(factory);
        self
    }

    pub fn bind_config<T>(&mut self) -> &mut Self
    where
        T: FromConfiguration + Send + Sync + 'static,
    {
        self.binders.push(Box::new(|config, services| {
            services.add_singleton(T::from_configuration(config).map_err(HostBuildError::Config)?);
            Ok(())
        }));
        self
    }

    pub fn module<M>(&mut self, module: M) -> &mut Self
    where
        M: RuntimeModule + 'static,
    {
        let module = Arc::new(module);
        module
            .configure_services(&mut self.services)
            .expect("module service configuration failed");
        module.configure_middleware(&mut self.middleware);
        module
            .configure_routes(self)
            .expect("module route configuration failed");
        self.modules.push(module);
        self
    }

    pub fn group<F>(&mut self, prefix: impl Into<String>, f: F) -> &mut Self
    where
        F: FnOnce(&mut RouteGroup<'_>),
    {
        let mut group = RouteGroup::new(self, prefix.into());
        f(&mut group);
        self
    }

    pub fn with_web_platform(&mut self) -> &mut Self {
        self.module(WebPlatformModule::default())
    }

    pub fn build(mut self) -> Result<ApplicationHost, HostBuildError> {
        let configuration = Arc::new(self.configuration.build().map_err(HostBuildError::Config)?);
        for binder in &self.binders {
            binder(configuration.as_ref(), &mut self.services)?;
        }

        let services = Arc::new(self.services.build());
        let root_scope = services.root_scope();
        let app_config = root_scope
            .resolve::<AppConfig>()
            .map_err(HostBuildError::Service)?;
        let runtime_settings = RuntimeSettings::default().merge_from(app_config.as_ref());

        Ok(ApplicationHost {
            router: Arc::new(self.router),
            middleware: Arc::new(self.middleware),
            services,
            modules: self.modules,
            configuration,
            runtime_settings,
        })
    }
}

impl Default for HostBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl RouteRegistration for HostBuilder {
    fn register_definition(&mut self, definition: RouteDefinition) -> &mut Self {
        self.router.add_definition(definition);
        self
    }
}

impl RouteRegistrar for HostBuilder {
    fn add_route(&mut self, definition: RouteDefinition) -> Result<(), FrameworkError> {
        self.router.add_definition(definition);
        Ok(())
    }
}

#[doc(hidden)]
pub struct RouteGroup<'a> {
    builder: &'a mut HostBuilder,
    prefix: String,
    middleware: Vec<Arc<dyn crate::middleware::Middleware>>,
}

impl<'a> RouteGroup<'a> {
    fn new(builder: &'a mut HostBuilder, prefix: String) -> Self {
        Self {
            builder,
            prefix: normalize_prefix(&prefix),
            middleware: Vec::new(),
        }
    }

    pub fn group<F>(&mut self, prefix: impl Into<String>, f: F) -> &mut Self
    where
        F: FnOnce(&mut RouteGroup<'_>),
    {
        let prefix = join_paths(&self.prefix, &prefix.into());
        let middleware = self.middleware.clone();
        let mut group = RouteGroup {
            builder: self.builder,
            prefix,
            middleware,
        };
        f(&mut group);
        self
    }

    pub fn module<M>(&mut self, module: M) -> &mut Self
    where
        M: RuntimeModule + 'static,
    {
        let module = Arc::new(module);
        module
            .configure_services(&mut self.builder.services)
            .expect("module service configuration failed");
        module.configure_middleware(&mut self.builder.middleware);
        module
            .configure_routes(self)
            .expect("module route configuration failed");
        self.builder.modules.push(module);
        self
    }
}

impl RouteRegistration for RouteGroup<'_> {
    fn register_definition(&mut self, mut definition: RouteDefinition) -> &mut Self {
        definition.path = join_paths(&self.prefix, &definition.path);
        let mut middleware = self.middleware.clone();
        middleware.extend(definition.middleware);
        definition.middleware = middleware;
        self.builder.router.add_definition(definition);
        self
    }
}

impl RouteRegistrar for RouteGroup<'_> {
    fn add_route(&mut self, definition: RouteDefinition) -> Result<(), FrameworkError> {
        self.register_definition(definition);
        Ok(())
    }
}

#[derive(Debug)]
pub enum HostBuildError {
    Config(ConfigError),
    Service(ServiceError),
    Framework(FrameworkError),
}

impl fmt::Display for HostBuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HostBuildError::Config(error) => write!(f, "{error}"),
            HostBuildError::Service(error) => write!(f, "{error}"),
            HostBuildError::Framework(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for HostBuildError {}

#[derive(Debug)]
pub enum HostError {
    Build(HostBuildError),
    Framework(FrameworkError),
    Io(std::io::Error),
}

impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HostError::Build(error) => write!(f, "{error}"),
            HostError::Framework(error) => write!(f, "{error}"),
            HostError::Io(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for HostError {}

impl From<HostBuildError> for HostError {
    fn from(value: HostBuildError) -> Self {
        Self::Build(value)
    }
}

fn normalize_prefix(prefix: &str) -> String {
    if prefix.is_empty() || prefix == "/" {
        String::new()
    } else {
        format!("/{}", prefix.trim_matches('/'))
    }
}

fn join_paths(prefix: &str, path: &str) -> String {
    let prefix = normalize_prefix(prefix);
    let path = path.trim_matches('/');
    match (prefix.is_empty(), path.is_empty()) {
        (true, true) => "/".to_string(),
        (true, false) => format!("/{}", path),
        (false, true) => prefix,
        (false, false) => format!("{}/{}", prefix, path),
    }
}
