use std::sync::Arc;

use crate::routing::{Handler, HandlerFuture, HandlerResult, RequestContext};

pub type MiddlewareFuture = HandlerFuture;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum MiddlewareStage {
    Logging,
    Recovery,
    Auth,
    Validation,
    Response,
}

#[derive(Clone)]
pub struct Next {
    middlewares: Arc<[PipelineMiddleware]>,
    index: usize,
    handler: Handler,
}

impl Next {
    pub(crate) fn new(
        middlewares: Arc<[PipelineMiddleware]>,
        index: usize,
        handler: Handler,
    ) -> Self {
        Self {
            middlewares,
            index,
            handler,
        }
    }

    pub fn run(&self, ctx: RequestContext) -> MiddlewareFuture {
        if let Some(middleware) = self.middlewares.get(self.index).cloned() {
            let next = Self {
                middlewares: Arc::clone(&self.middlewares),
                index: self.index + 1,
                handler: self.handler.clone(),
            };
            middleware.inner.handle(ctx, next)
        } else {
            let handler = self.handler.clone();
            Box::pin(async move { handler.call(ctx).await })
        }
    }
}

pub trait Middleware: Send + Sync {
    fn stage(&self) -> MiddlewareStage {
        MiddlewareStage::Validation
    }

    fn handle(&self, ctx: RequestContext, next: Next) -> MiddlewareFuture;
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
        let mut combined = self
            .stack
            .iter()
            .cloned()
            .enumerate()
            .map(|(index, middleware)| {
                PipelineMiddleware::new(middleware, MiddlewareSource::Global, index)
            })
            .chain(
                route_stack
                    .iter()
                    .cloned()
                    .enumerate()
                    .map(|(index, middleware)| {
                        PipelineMiddleware::new(middleware, MiddlewareSource::Route, index)
                    }),
            )
            .collect::<Vec<_>>();

        combined.sort_by_key(|middleware| {
            (
                middleware.stage,
                middleware.source,
                middleware.registration_index,
            )
        });

        let combined: Arc<[PipelineMiddleware]> = combined.into();

        Next::new(combined, 0, handler).run(ctx).await
    }
}

#[derive(Clone)]
pub struct PipelineMiddleware {
    inner: Arc<dyn Middleware>,
    stage: MiddlewareStage,
    source: MiddlewareSource,
    registration_index: usize,
}

impl PipelineMiddleware {
    fn new(
        inner: Arc<dyn Middleware>,
        source: MiddlewareSource,
        registration_index: usize,
    ) -> Self {
        Self {
            stage: inner.stage(),
            inner,
            source,
            registration_index,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum MiddlewareSource {
    Global,
    Route,
}
