extern crate self as vantus;

mod app;
mod config;
mod core;
mod di;
mod middleware;
mod routing;
mod runtime;

pub use app::{
    ApplicationHost, Config, Controller, HealthModule, HostBuildError, HostBuilder, HostContext,
    HostError, InfoModule, Module, PanicRecovery, RequestLogger, RuntimeModule, Service,
    ServiceCollection, ServiceContainer, ServiceError, ServiceLifetime, ServiceScope,
    WebPlatformModule,
};
pub use async_trait::async_trait;
pub use config::{AppConfig, ConfigError, Configuration, ConfigurationBuilder, FromConfiguration};
pub use core::{FrameworkError, HttpError, Method, ParseError, Request, Response};
pub use di::{
    BodyBytes, ExtractorError, FromRequest, IntoHandlerResult, JsonBody, NamedFromRequest, Path,
    Query, QueryMap, TextBody,
};
pub use middleware::Middleware;
pub use routing::RequestContext;
pub use runtime::ServerHandle;

pub use vantus_macros::{controller, delete, get, module, post, put};

#[doc(hidden)]
pub mod __private {
    pub use crate::async_trait;
    pub use crate::di::{IntoHandlerResult, NamedFromRequest};
    pub use crate::middleware::{MiddlewareStack, Next};
    pub use crate::routing::{
        Handler, HandlerResult, RequestContext, RouteDefinition, RouteRegistrar,
    };
}
