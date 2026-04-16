use std::sync::Arc;

use async_trait::async_trait;

use crate::routing::{Handler, HandlerFuture, HandlerResult, RequestContext};

#[derive(Clone)]
pub struct Next {
    handler: Handler,
}

impl Next {
    pub(crate) fn new(handler: Handler) -> Self {
        Self { handler }
    }

    pub async fn run(&self, ctx: RequestContext) -> HandlerResult {
        self.handler.call(ctx).await
    }
}

#[async_trait]
pub trait Middleware: Send + Sync {
    async fn handle(&self, ctx: RequestContext, next: Next) -> HandlerResult;
}

#[derive(Clone, Default)]
pub struct MiddlewareStack {
    stack: Vec<Arc<dyn Middleware>>,
}

impl MiddlewareStack {
    pub fn new() -> Self {
        Self { stack: Vec::new() }
    }

    pub fn len(&self) -> usize {
        self.stack.len()
    }

    pub fn is_empty(&self) -> bool {
        self.stack.is_empty()
    }

    pub fn add<M>(&mut self, middleware: M)
    where
        M: Middleware + 'static,
    {
        self.stack.push(Arc::new(middleware));
    }

    pub async fn execute(
        &self,
        route_stack: &[Arc<dyn Middleware>],
        ctx: RequestContext,
        handler: Handler,
    ) -> HandlerResult {
        let mut combined = self.stack.clone();
        combined.extend_from_slice(route_stack);
        execute_chain(combined, ctx, handler).await
    }
}

fn execute_chain(
    middlewares: Vec<Arc<dyn Middleware>>,
    ctx: RequestContext,
    handler: Handler,
) -> HandlerFuture {
    Box::pin(async move {
        if let Some((first, rest)) = middlewares.split_first() {
            let rest = rest.to_vec();
            let next_handler = Handler::new(move |ctx| {
                let rest = rest.clone();
                let handler = handler.clone();
                execute_chain(rest, ctx, handler)
            });
            first.handle(ctx, Next::new(next_handler)).await
        } else {
            handler.call(ctx).await
        }
    })
}
