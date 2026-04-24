use std::fmt;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

use crate::app::module::RuntimeModule;
use crate::app::state::HostState;
use crate::app::{ObservabilityModule, WebPlatformModule};
use crate::config::{
    AppConfig, ConfigError, Configuration, ConfigurationBuilder, FromConfiguration,
};
use crate::core::errors::{FrameworkError, HttpError};
use crate::core::http::{Request, Response};
use crate::middleware::{Middleware, MiddlewareStack};
use crate::routing::{RequestContext, RouteDefinition, RouteRegistrar, RouteResolution, Router};
use crate::runtime::{
    GlobalRateLimiter, RuntimeSecuritySettings, RuntimeSettings, RuntimeState, ServerHandle,
    enforce_pre_middleware_limits, enforce_route_contract, method_not_allowed_response,
    record_contract_error, record_method_not_allowed, serve,
};
use crate::{IdGenerator, LogSink, StdIoLogSink, UuidIdGenerator};

type CompositionHook = Box<
    dyn Fn(&Configuration, &AppConfig, &mut CompositionContext<'_>) -> Result<(), HostBuildError>
        + Send
        + Sync,
>;

#[derive(Clone)]
/// Handle to framework-owned background tasks.
///
/// Runtime modules can use this through `HostContext` to spawn work that
/// should be cancelled automatically during graceful shutdown.
pub struct BackgroundTasks {
    cancellation: CancellationToken,
    handles: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

impl BackgroundTasks {
    /// Creates a background-task coordinator bound to a cancellation token.
    pub fn new(cancellation: CancellationToken) -> Self {
        Self {
            cancellation,
            handles: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Returns the cancellation token observed by framework-managed tasks.
    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    /// Spawns a task that will be cancelled and awaited during host shutdown.
    pub async fn spawn<F>(&self, future: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.handles
            .lock()
            .await
            .push(crate::runtime::spawn_task(future));
    }

    /// Cancels outstanding tasks and waits for their join handles to resolve.
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
/// Runtime context passed to lifecycle hooks.
///
/// Use this inside `RuntimeModule::on_start` / `on_stop` when you need
/// access to services, configuration, or framework-managed background tasks.
pub struct HostContext {
    host: Arc<HostState>,
    background_tasks: BackgroundTasks,
}

impl HostContext {
    /// Returns the resolved framework application configuration.
    pub fn app_config(&self) -> &AppConfig {
        self.host.app_config()
    }

    /// Returns shared runtime counters and gauges for the current host.
    pub fn runtime_state(&self) -> Arc<RuntimeState> {
        self.host.runtime_state()
    }

    /// Returns the configured log sink used by framework middleware.
    pub fn log_sink(&self) -> Arc<dyn LogSink> {
        self.host.log_sink()
    }

    /// Returns the configured ID generator shared across the host.
    pub fn id_generator(&self) -> Arc<dyn IdGenerator> {
        self.host.id_generator()
    }

    /// Returns the background-task coordinator for lifecycle hooks.
    pub fn background_tasks(&self) -> &BackgroundTasks {
        &self.background_tasks
    }
}

pub struct ApplicationHost {
    router: Arc<Router>,
    middleware: Arc<MiddlewareStack>,
    host: Arc<HostState>,
    modules: Vec<Arc<dyn RuntimeModule>>,
    runtime_settings: RuntimeSettings,
    security_settings: RuntimeSecuritySettings,
    background_tasks: BackgroundTasks,
}

impl ApplicationHost {
    /// Returns a cloneable runtime context for advanced integrations.
    pub fn context(&self) -> HostContext {
        HostContext {
            host: Arc::clone(&self.host),
            background_tasks: self.background_tasks.clone(),
        }
    }

    /// Starts the network server and returns a shutdown handle.
    pub async fn serve(self) -> Result<ServerHandle, HostError> {
        let rollback_host = Arc::clone(&self.host);
        let rollback_tasks = self.background_tasks.clone();
        let context = HostContext {
            host: Arc::clone(&self.host),
            background_tasks: self.background_tasks.clone(),
        };
        let mut started_modules = Vec::new();

        for module in &self.modules {
            if let Err(error) = module.on_start(&context).await {
                rollback_started_modules(&started_modules, &context).await;
                return Err(HostError::Framework(error));
            }
            started_modules.push(Arc::clone(module));
        }

        let server = serve(
            self.router,
            self.middleware,
            self.host,
            self.modules,
            self.runtime_settings,
            self.security_settings,
            context,
        )
        .await;

        if server.is_err() {
            let rollback_context = HostContext {
                host: rollback_host,
                background_tasks: rollback_tasks,
            };
            rollback_started_modules(&started_modules, &rollback_context).await;
        }

        server
    }

    /// Runs the application until shutdown.
    pub async fn run(self) -> Result<(), HostError> {
        self.serve().await?.wait().await
    }

    /// Sends an in-memory request through the router and middleware stack.
    ///
    /// This is especially useful in tests and examples where you want to
    /// exercise application behavior without opening a socket.
    pub async fn handle(&self, request: Request) -> Response {
        let runtime_state = self.host.runtime_state();

        if let Err(error) = enforce_pre_middleware_limits(
            &request,
            &self.security_settings,
            Some(runtime_state.as_ref()),
        ) {
            return error.to_response();
        }

        let route = match self.router.resolve(&request.method, &request.path) {
            RouteResolution::Matched(route) => route,
            RouteResolution::MethodNotAllowed { allow } => {
                record_method_not_allowed(runtime_state.as_ref());
                return method_not_allowed_response(&allow);
            }
            RouteResolution::NotFound => return Response::not_found(),
        };

        if let Err(error) = enforce_route_contract(&request, route.contract) {
            record_contract_error(runtime_state.as_ref(), &error);
            return error.to_response();
        }

        let ctx = RequestContext::new(
            request,
            route.route_pattern.clone(),
            route.path_params,
            Arc::clone(&self.host),
        );
        match timeout(
            self.security_settings.request_timeout,
            self.middleware
                .execute(&route.middleware, ctx, route.handler),
        )
        .await
        {
            Ok(Ok(response)) => response,
            Ok(Err(error)) => error.to_response(),
            Err(_) => HttpError::new(408, "Request Timeout", "408 Request Timeout").to_response(),
        }
    }

    /// Convenience wrapper for synchronous `main` functions.
    pub fn blocking_run(self) {
        self.run_blocking()
    }

    /// Runs the host on a Tokio runtime, prints startup failures, and exits the process.
    pub fn run_blocking(self) {
        let result = crate::runtime::block_on(async move { self.run().await });
        let final_result = match result {
            Ok(inner_result) => inner_result.map_err(|e| e.to_string()),
            Err(runtime_err) => Err(runtime_err.to_string()),
        };
        if let Err(message) = final_result {
            eprintln!("Vantus Framework Error: {}", message);
            std::process::exit(1);
        }
    }
}

/// Fluent application bootstrap API.
///
/// Typical usage:
/// 1. create with `HostBuilder::new()`
/// 2. add configuration sources and modules
/// 3. optionally compose config-aware modules
/// 4. call `build()` and then `run_blocking()` / `serve()`
pub struct HostBuilder {
    router: Router,
    middleware: MiddlewareStack,
    modules: Vec<Arc<dyn RuntimeModule>>,
    configuration: ConfigurationBuilder,
    composition_hooks: Vec<CompositionHook>,
    registration_errors: Vec<HostBuildError>,
    max_body_size_override: Option<usize>,
    request_timeout_override: Option<Duration>,
    rate_limiter: Option<GlobalRateLimiter>,
    log_sink: Arc<dyn LogSink>,
    id_generator: Arc<dyn IdGenerator>,
}

impl HostBuilder {
    /// Creates a builder with the default framework runtime dependencies already registered.
    pub fn new() -> Self {
        Self {
            router: Router::new(),
            middleware: MiddlewareStack::new(),
            modules: Vec::new(),
            configuration: ConfigurationBuilder::new(),
            composition_hooks: Vec::new(),
            registration_errors: Vec::new(),
            max_body_size_override: None,
            request_timeout_override: None,
            rate_limiter: None,
            log_sink: Arc::new(StdIoLogSink),
            id_generator: Arc::new(UuidIdGenerator),
        }
    }

    /// Adds a `.properties` file to the layered configuration sources.
    pub fn config_file(&mut self, path: impl Into<PathBuf>) -> &mut Self {
        self.configuration.config_file(path);
        self
    }

    /// Overrides the active environment name.
    pub fn environment(&mut self, name: impl Into<String>) -> &mut Self {
        self.configuration.environment(name);
        self
    }

    /// Overrides the active profile used for profile-specific config files.
    pub fn profile(&mut self, profile: impl Into<String>) -> &mut Self {
        self.configuration.profile(profile);
        self
    }

    /// Changes the environment variable prefix, defaulting to `APP`.
    pub fn env_prefix(&mut self, prefix: impl Into<String>) -> &mut Self {
        self.configuration.env_prefix(prefix);
        self
    }

    /// Overrides `server.max-request-bytes` after configuration binding.
    pub fn max_body_size(&mut self, bytes: usize) -> &mut Self {
        self.max_body_size_override = Some(bytes);
        self
    }

    /// Overrides the outer request deadline enforced around the full pipeline.
    pub fn request_timeout(&mut self, duration: Duration) -> &mut Self {
        self.request_timeout_override = Some(duration);
        self
    }

    /// Installs a pre-middleware global token-bucket limiter keyed by client IP.
    pub fn rate_limiter(&mut self, rate_limiter: GlobalRateLimiter) -> &mut Self {
        self.rate_limiter = Some(rate_limiter);
        self
    }

    /// Replaces the default log sink used by framework middleware/modules.
    pub fn log_sink<T>(&mut self, sink: T) -> &mut Self
    where
        T: LogSink + 'static,
    {
        self.log_sink = Arc::new(sink);
        self
    }

    /// Replaces the default ID generator used by framework middleware and modules.
    pub fn id_generator<T>(&mut self, generator: T) -> &mut Self
    where
        T: IdGenerator + 'static,
    {
        self.id_generator = Arc::new(generator);
        self
    }

    /// Adds a build-time configuration composition hook.
    pub fn compose_with_config<F>(&mut self, compose: F) -> &mut Self
    where
        F: Fn(
                &Configuration,
                &AppConfig,
                &mut CompositionContext<'_>,
            ) -> Result<(), HostBuildError>
            + Send
            + Sync
            + 'static,
    {
        self.composition_hooks.push(Box::new(compose));
        self
    }

    /// Mounts a module at the application root.
    pub fn module<M>(&mut self, module: M) -> &mut Self
    where
        M: RuntimeModule + 'static,
    {
        self.register_module(module);
        self
    }

    /// Creates a grouped route prefix for nested modules.
    ///
    /// This is the usual way to mount versioned APIs such as `/api/v1`.
    pub fn group<F>(&mut self, prefix: impl Into<String>, f: F) -> &mut Self
    where
        F: FnOnce(&mut RouteGroup<'_>),
    {
        let mut group = RouteGroup::new(self, prefix.into());
        f(&mut group);
        self
    }

    /// Adds the framework's default production-style platform module.
    pub fn with_web_platform(&mut self) -> &mut Self {
        self.module(WebPlatformModule::default())
    }

    /// Adds the first-party observability stack.
    pub fn with_observability(&mut self) -> &mut Self {
        self.module(ObservabilityModule::default())
    }

    /// Finalizes configuration and builds the host, panicking on any configuration errors.
    pub fn build(self) -> ApplicationHost {
        self.try_build().expect("Module registration failed")
    }

    /// Attempts to build the host and returns a Result.
    /// Use this in tests to inspect specific error variants.
    pub fn try_build(mut self) -> Result<ApplicationHost, HostBuildError> {
        // 1. Handle registration errors
        if !self.registration_errors.is_empty() {
            return Err(HostBuildError::combine(self.registration_errors));
        }

        // If the user didn't provide a file, look for defaults automatically
        if self.configuration.config_file.is_none() {
            let defaults = [
                "application.toml",
                "application.yaml",
                "application.json",
                "application.properties",
            ];
            for file in defaults {
                let path = PathBuf::from(file);
                if path.exists() {
                    self.configuration.config_file(path);
                    break; // Stop at the first one found
                }
            }
        }

        // 2. Build configuration
        let configuration = Arc::new(self.configuration.build().map_err(HostBuildError::Config)?);

        // 3. Resolve built-in app config and run config-aware composition hooks.
        let app_config = Arc::new(
            AppConfig::from_configuration(configuration.as_ref())
                .map_err(HostBuildError::Config)?,
        );
        let composition_hooks = std::mem::take(&mut self.composition_hooks);
        for compose in composition_hooks {
            let mut context = CompositionContext { builder: &mut self };
            compose(configuration.as_ref(), app_config.as_ref(), &mut context)?;
        }

        if !self.registration_errors.is_empty() {
            return Err(HostBuildError::combine(self.registration_errors));
        }

        // 4. Build host state and runtime settings.
        let runtime_settings = RuntimeSettings::from_config(app_config.as_ref())?;
        let max_body_size = self
            .max_body_size_override
            .unwrap_or(app_config.as_ref().server.max_request_bytes);
        if max_body_size == 0 {
            return Err(HostBuildError::Config(ConfigError::InvalidValue {
                key: "builder.max-body-size",
                value: max_body_size.to_string(),
                expected: "positive usize",
            }));
        }
        let request_timeout = self
            .request_timeout_override
            .unwrap_or(app_config.as_ref().server.request_timeout);
        if request_timeout.is_zero() {
            return Err(HostBuildError::Config(ConfigError::InvalidValue {
                key: "builder.request-timeout",
                value: "0".to_string(),
                expected: "positive duration",
            }));
        }
        let security_settings = RuntimeSecuritySettings::new(
            max_body_size,
            request_timeout,
            app_config.as_ref().server.trusted_proxies.clone(),
            self.rate_limiter,
        );
        let host = Arc::new(HostState::new(
            Arc::clone(&configuration),
            Arc::clone(&app_config),
            Arc::new(RuntimeState::default()),
            Arc::clone(&self.log_sink),
            Arc::clone(&self.id_generator),
        ));

        // 5. Return host
        Ok(ApplicationHost {
            router: Arc::new(self.router),
            middleware: Arc::new(self.middleware),
            host,
            modules: self.modules,
            runtime_settings,
            security_settings,
            background_tasks: BackgroundTasks::new(CancellationToken::new()),
        })
    }

    fn register_module<M>(&mut self, module: M)
    where
        M: RuntimeModule + 'static,
    {
        let module_name = std::any::type_name::<M>();
        let module = Arc::new(module);
        module.configure_middleware(&mut self.middleware);
        if let Err(error) = Arc::clone(&module).configure_routes_arc(self) {
            self.registration_errors.push(HostBuildError::module(
                module_name,
                ModulePhase::ConfigureRoutes,
                error,
            ));
        }
        self.modules.push(module);
    }
}

impl Default for HostBuilder {
    fn default() -> Self {
        Self::new()
    }
}

async fn rollback_started_modules(modules: &[Arc<dyn RuntimeModule>], context: &HostContext) {
    // Startup rollback mirrors steady-state shutdown so partially started
    // hosts do not leave background work or module resources behind.
    context.background_tasks().shutdown().await;
    for module in modules.iter().rev() {
        let _ = module.on_stop(context).await;
    }
}

impl RouteRegistrar for HostBuilder {
    fn add_route(&mut self, definition: RouteDefinition) -> Result<(), FrameworkError> {
        self.router.add_definition(definition)
    }
}

#[doc(hidden)]
pub struct RouteGroup<'a> {
    builder: &'a mut HostBuilder,
    prefix: String,
    middleware: Vec<Arc<dyn Middleware>>,
}

impl<'a> RouteGroup<'a> {
    fn new(builder: &'a mut HostBuilder, prefix: String) -> Self {
        Self {
            builder,
            prefix: normalize_prefix(&prefix),
            middleware: Vec::new(),
        }
    }

    /// Creates a nested route group under the current prefix.
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

    /// Mounts a module underneath this group's prefix.
    pub fn module<M>(&mut self, module: M) -> &mut Self
    where
        M: RuntimeModule + 'static,
    {
        let module_name = std::any::type_name::<M>();
        let module = Arc::new(module);
        module.configure_middleware(&mut self.builder.middleware);
        if let Err(error) = Arc::clone(&module).configure_routes_arc(self) {
            self.builder
                .registration_errors
                .push(HostBuildError::module(
                    module_name,
                    ModulePhase::ConfigureRoutes,
                    error,
                ));
        }
        self.builder.modules.push(module);
        self
    }
}

impl RouteRegistrar for RouteGroup<'_> {
    fn add_route(&mut self, definition: RouteDefinition) -> Result<(), FrameworkError> {
        let mut definition = definition;
        definition.path = join_paths(&self.prefix, &definition.path);
        let mut middleware = self.middleware.clone();
        middleware.extend(definition.middleware);
        definition.middleware = middleware;
        self.builder.router.add_definition(definition)
    }
}

#[derive(Debug, Clone, Copy)]
pub enum ModulePhase {
    ConfigureRoutes,
}

impl fmt::Display for ModulePhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModulePhase::ConfigureRoutes => write!(f, "configure_routes"),
        }
    }
}

#[derive(Debug)]
pub enum HostBuildError {
    Config(ConfigError),
    Framework(FrameworkError),
    Module {
        module: &'static str,
        phase: ModulePhase,
        source: FrameworkError,
    },
    Combined(Vec<HostBuildError>),
}

impl HostBuildError {
    fn module(module: &'static str, phase: ModulePhase, source: FrameworkError) -> Self {
        Self::Module {
            module,
            phase,
            source,
        }
    }

    fn combine(errors: Vec<HostBuildError>) -> Self {
        match errors.len() {
            0 => Self::Framework(FrameworkError::Startup {
                context: "build failed".to_string(),
            }),
            1 => errors
                .into_iter()
                .next()
                .unwrap_or(Self::Framework(FrameworkError::Startup {
                    context: "build failed".to_string(),
                })),
            _ => Self::Combined(errors),
        }
    }
}

impl fmt::Display for HostBuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HostBuildError::Config(error) => write!(f, "{error}"),
            HostBuildError::Framework(error) => write!(f, "{error}"),
            HostBuildError::Module {
                module,
                phase,
                source,
            } => write!(f, "module {module} failed during {phase}: {source}"),
            HostBuildError::Combined(errors) => {
                let summary = errors
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join("; ");
                write!(f, "{summary}")
            }
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

pub struct CompositionContext<'a> {
    builder: &'a mut HostBuilder,
}

impl CompositionContext<'_> {
    pub fn module<M>(&mut self, module: M) -> &mut Self
    where
        M: RuntimeModule + 'static,
    {
        self.builder.module(module);
        self
    }

    pub fn group<F>(&mut self, prefix: impl Into<String>, f: F) -> &mut Self
    where
        F: FnOnce(&mut RouteGroup<'_>),
    {
        self.builder.group(prefix, f);
        self
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
