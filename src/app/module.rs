use async_trait::async_trait;

use crate::app::application::HostContext;
use crate::app::state::ServiceCollection;
use crate::core::errors::FrameworkError;
use crate::middleware::MiddlewareStack;
use crate::routing::RouteRegistrar;

pub trait Module: Send + Sync {
    fn configure_services(&self, _services: &mut ServiceCollection) -> Result<(), FrameworkError> {
        Ok(())
    }

    fn configure_middleware(&self, _middleware: &mut MiddlewareStack) {}

    fn configure_routes(&self, _routes: &mut dyn RouteRegistrar) -> Result<(), FrameworkError> {
        Ok(())
    }
}

#[async_trait]
pub trait RuntimeModule: Module {
    async fn on_start(&self, _host: &HostContext) -> Result<(), FrameworkError> {
        Ok(())
    }

    async fn on_stop(&self, _host: &HostContext) -> Result<(), FrameworkError> {
        Ok(())
    }
}

pub trait Controller: Module {}

impl<T: Module + ?Sized> Controller for T {}
