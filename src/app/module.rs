use async_trait::async_trait;

use crate::app::application::HostContext;
use crate::core::errors::FrameworkError;
use crate::middleware::MiddlewareStack;
use crate::routing::RouteRegistrar;

/// The core application building block in Vantus.
///
/// Implement this trait with `#[module]` when you want to:
/// - add middleware
/// - declare routes
///
/// In day-to-day application code, you normally won't implement this
/// manually. The `#[module]` macro generates the trait impl for you.
pub trait Module: Send + Sync {
    /// Add middleware that should run before this module's handlers.
    fn configure_middleware(&self, _middleware: &mut MiddlewareStack) {}

    /// Register the routes owned by this module.
    fn configure_routes(&self, _routes: &mut dyn RouteRegistrar) -> Result<(), FrameworkError> {
        Ok(())
    }

    /// Internal Arc-aware route registration path used by macro-generated modules.
    fn configure_routes_arc(
        self: std::sync::Arc<Self>,
        routes: &mut dyn RouteRegistrar,
    ) -> Result<(), FrameworkError>
    where
        Self: Sized,
    {
        self.configure_routes(routes)
    }
}

#[async_trait]
/// Optional lifecycle hooks for long-lived application modules.
///
/// Use this when a module needs startup or shutdown behavior such as:
/// - warming caches
/// - registering readiness contributors
/// - launching background tasks
pub trait RuntimeModule: Module {
    async fn on_start(&self, _host: &HostContext) -> Result<(), FrameworkError> {
        Ok(())
    }

    async fn on_stop(&self, _host: &HostContext) -> Result<(), FrameworkError> {
        Ok(())
    }
}

/// Marker trait for route-focused types.
///
/// In practice, `#[controller]` types become `Controller`s automatically.
pub trait Controller: Module {}

impl<T: Module + ?Sized> Controller for T {}
