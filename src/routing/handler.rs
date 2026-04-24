use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::core::errors::FrameworkError;
use crate::core::http::Response;

use super::RequestContext;

pub type HandlerFuture = Pin<Box<dyn Future<Output = HandlerResult> + Send>>;
pub type HandlerResult = Result<Response, FrameworkError>;

/// Boxed request handler used internally by the router.
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
