//! Public entrypoint for the Vantus framework.
//!
//! Most application code only needs the re-exports from this file:
//! `HostBuilder` for bootstrapping, `#[module]` / `#[controller]`
//! for structure, and the typed request extractors for handlers.
//!
//! ```no_run
//! use vantus::{HostBuilder, RequestContext, Response, module};
//!
//! #[derive(Clone)]
//! struct HealthModule;
//!
//! #[module]
//! impl HealthModule {
//!     #[vantus::get("/health")]
//!     fn health(&self, ctx: RequestContext) -> Response {
//!         Response::json_value(serde_json::json!({
//!             "service": ctx.app_config().service_name,
//!             "status": "ok",
//!         }))
//!     }
//! }
//!
//! fn main() {
//!     let mut builder = HostBuilder::new();
//!     builder.compose_with_config(|_configuration, _app, context| {
//!         context.module(HealthModule);
//!         Ok(())
//!     });
//!     builder.build().run_blocking();
//! }
//! ```

extern crate self as vantus;

mod app;
#[cfg(feature = "cli")]
mod cli;
mod config;
mod core;
mod di;
mod error_conversions;
mod id;
mod logging;
mod middleware;
mod routing;
mod runtime;

pub use app::{
    ApplicationHost, Controller, HealthModule, HostBuildError, HostBuilder, HostContext, HostError,
    InfoModule, Module, ObservabilityModule, PanicRecovery, ReadinessCheck, ReadinessContributor,
    ReadinessRegistry, RequestId, RequestLogger, RuntimeModule, SecurityHeaders, WebPlatformModule,
};
pub use async_trait::async_trait;
#[cfg(feature = "cli")]
pub use cli::{CliError, CliFeatureDefaults, IdGeneratorKind, RuntimeMode, ServerCli};
pub use config::{
    AppConfig, ConfigError, Configuration, ConfigurationBuilder, FromConfig, FromConfiguration,
    ServerConfig, ServerOptions, ServerProtocol,
};
pub use core::{FrameworkError, HttpError, Method, ParseError, Request, Response};
pub use di::{
    BodyBytes, ExtractorError, FromRequest, Header, IdentityState, IntoHandlerResult, IntoResponse,
    JsonBody, NamedFromRequest, NamedOptionalFromRequest, OptionalFromRequest, Path, Query,
    QueryMap, RequestState, TextBody,
};
pub use id::{AtomicIdGenerator, IdGenerator, UuidIdGenerator};
pub use logging::{
    LogLevel, LogSink, RequestLogEvent, StdIoLogSink, emit_default_log, redact_headers,
    sanitize_log_message, sanitize_path_for_logs,
};
pub use middleware::{Middleware, MiddlewareFuture, MiddlewareStage};
pub use routing::{Identity, RequestContext};
pub use runtime::{GlobalRateLimiter, RuntimeSnapshot, RuntimeState, ServerHandle};

/// Attribute macros used to define route-bearing modules and controllers.
pub use vantus_macros::{
    controller, delete, get, head, middleware, module, options, patch, post, put,
};

#[doc(hidden)]
pub mod __private {
    pub use crate::async_trait;
    pub use crate::di::{
        IntoHandlerResult, IntoResponse, NamedFromRequest, NamedOptionalFromRequest,
        OptionalFromRequest,
    };
    pub use crate::middleware::{MiddlewareStack, Next};
    pub use crate::routing::{
        Handler, HandlerResult, RequestBodyKind, RequestContext, RouteContract, RouteDefinition,
        RouteRegistrar, Router,
    };
}
