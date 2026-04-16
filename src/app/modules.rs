use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use futures_util::FutureExt;
use serde::Serialize;

use crate::app::application::HostContext;
use crate::app::module::{Module, RuntimeModule};
use crate::app::state::{Service, ServiceCollection};
use crate::config::AppConfig;
use crate::core::errors::{FrameworkError, HttpError};
use crate::core::http::{Method, Response};
use crate::middleware::{Middleware, Next};
use crate::routing::{RequestContext, RouteDefinition, RouteRegistrar};

pub struct RequestLogger;

#[async_trait]
impl Middleware for RequestLogger {
    async fn handle(&self, ctx: RequestContext, next: Next) -> crate::routing::HandlerResult {
        println!("{} {}", ctx.request().method, ctx.request().path);
        next.run(ctx).await
    }
}

pub struct PanicRecovery;

#[async_trait]
impl Middleware for PanicRecovery {
    async fn handle(&self, ctx: RequestContext, next: Next) -> crate::routing::HandlerResult {
        match std::panic::AssertUnwindSafe(next.run(ctx))
            .catch_unwind()
            .await
        {
            Ok(result) => result,
            Err(_) => Ok(Response::internal_server_error()),
        }
    }
}

#[derive(Default)]
pub struct HealthModule;

#[derive(Default)]
struct ReadinessState {
    ready: AtomicBool,
}

impl Module for HealthModule {
    fn configure_services(&self, services: &mut ServiceCollection) -> Result<(), FrameworkError> {
        if !services.contains::<ReadinessState>() {
            services.add_singleton(ReadinessState::default());
        }
        Ok(())
    }

    fn configure_routes(&self, routes: &mut dyn RouteRegistrar) -> Result<(), FrameworkError> {
        routes.add_route(RouteDefinition::new(
            Method::Get,
            "/health",
            crate::routing::Handler::new(|ctx| async move {
                #[derive(Serialize)]
                struct HealthPayload<'a> {
                    status: &'a str,
                }

                let state: Service<ReadinessState> = ctx.service()?;
                let status = if state.as_ref().ready.load(Ordering::SeqCst) {
                    "ok"
                } else {
                    "starting"
                };

                Response::json_serialized(&HealthPayload { status })
                    .map_err(|error| FrameworkError::Internal(error.to_string()))
            }),
        ))
    }
}

#[async_trait]
impl RuntimeModule for HealthModule {
    async fn on_start(&self, host: &HostContext) -> Result<(), FrameworkError> {
        let scope = host.service_scope();
        let state = scope
            .resolve::<ReadinessState>()
            .map_err(FrameworkError::from)?;
        let config = scope.resolve::<AppConfig>().map_err(FrameworkError::from)?;
        state.ready.store(config.readiness, Ordering::SeqCst);
        Ok(())
    }

    async fn on_stop(&self, host: &HostContext) -> Result<(), FrameworkError> {
        let scope = host.service_scope();
        let state = scope
            .resolve::<ReadinessState>()
            .map_err(FrameworkError::from)?;
        state.ready.store(false, Ordering::SeqCst);
        Ok(())
    }
}

#[derive(Default)]
pub struct InfoModule;

#[derive(Default)]
struct InfoState {
    enabled: AtomicBool,
}

impl Module for InfoModule {
    fn configure_services(&self, services: &mut ServiceCollection) -> Result<(), FrameworkError> {
        if !services.contains::<InfoState>() {
            services.add_singleton(InfoState::default());
        }
        Ok(())
    }

    fn configure_routes(&self, routes: &mut dyn RouteRegistrar) -> Result<(), FrameworkError> {
        routes.add_route(RouteDefinition::new(
            Method::Get,
            "/info",
            crate::routing::Handler::new(|ctx| async move {
                #[derive(Serialize)]
                struct InfoPayload {
                    service: String,
                    path: String,
                    time: u64,
                }

                let info: Service<InfoState> = ctx.service()?;
                if !info.as_ref().enabled.load(Ordering::SeqCst) {
                    return Err(FrameworkError::Http(HttpError::new(
                        404,
                        "Not Found",
                        "404 Not Found",
                    )));
                }

                let config = ctx.config::<AppConfig>()?;
                let payload = InfoPayload {
                    service: config.as_ref().service_name.clone(),
                    path: ctx.request().path.clone(),
                    time: SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map(|duration| duration.as_secs())
                        .unwrap_or(0),
                };
                Response::json_serialized(&payload)
                    .map_err(|error| FrameworkError::Internal(error.to_string()))
            }),
        ))
    }
}

#[async_trait]
impl RuntimeModule for InfoModule {
    async fn on_start(&self, host: &HostContext) -> Result<(), FrameworkError> {
        let scope = host.service_scope();
        let state = scope.resolve::<InfoState>().map_err(FrameworkError::from)?;
        let config = scope.resolve::<AppConfig>().map_err(FrameworkError::from)?;
        state.enabled.store(config.enable_info, Ordering::SeqCst);
        Ok(())
    }

    async fn on_stop(&self, host: &HostContext) -> Result<(), FrameworkError> {
        let scope = host.service_scope();
        let state = scope.resolve::<InfoState>().map_err(FrameworkError::from)?;
        state.enabled.store(false, Ordering::SeqCst);
        Ok(())
    }
}

pub struct WebPlatformModule {
    health: HealthModule,
    info: InfoModule,
}

impl WebPlatformModule {
    pub fn new() -> Self {
        Self {
            health: HealthModule,
            info: InfoModule,
        }
    }
}

impl Default for WebPlatformModule {
    fn default() -> Self {
        Self::new()
    }
}

impl Module for WebPlatformModule {
    fn configure_services(&self, services: &mut ServiceCollection) -> Result<(), FrameworkError> {
        self.health.configure_services(services)?;
        self.info.configure_services(services)?;
        Ok(())
    }

    fn configure_middleware(&self, middleware: &mut crate::middleware::MiddlewareStack) {
        middleware.add(RequestLogger);
        middleware.add(PanicRecovery);
    }

    fn configure_routes(&self, routes: &mut dyn RouteRegistrar) -> Result<(), FrameworkError> {
        self.health.configure_routes(routes)?;
        self.info.configure_routes(routes)?;
        Ok(())
    }
}

#[async_trait]
impl RuntimeModule for WebPlatformModule {
    async fn on_start(&self, host: &HostContext) -> Result<(), FrameworkError> {
        self.health.on_start(host).await?;
        self.info.on_start(host).await
    }

    async fn on_stop(&self, host: &HostContext) -> Result<(), FrameworkError> {
        self.info.on_stop(host).await?;
        self.health.on_stop(host).await
    }
}
