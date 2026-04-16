use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Request as HyperRequest, Response as HyperResponse};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio::sync::Semaphore;
use tokio::task::JoinHandle;
use tokio::time::timeout;

use crate::app::{HostContext, RuntimeModule, ServiceContainer};
use crate::config::{AppConfig, Configuration};
use crate::core::http::{Method, Request, Response};
use crate::middleware::MiddlewareStack;
use crate::routing::{RequestContext, Router};
use crate::{HostBuildError, HostError};

#[derive(Clone, Copy, Debug)]
pub struct RuntimeSettings {
    pub request_timeout: std::time::Duration,
    pub graceful_shutdown: std::time::Duration,
    pub max_request_bytes: usize,
    pub concurrency_limit: usize,
}

impl RuntimeSettings {
    pub fn merge_from(mut self, config: &AppConfig) -> Self {
        self.request_timeout = config.server.request_timeout;
        self.graceful_shutdown = config.server.graceful_shutdown;
        self.max_request_bytes = config.server.max_request_bytes;
        self.concurrency_limit = config.server.concurrency_limit;
        self
    }
}

impl Default for RuntimeSettings {
    fn default() -> Self {
        let config = AppConfig::default();
        Self {
            request_timeout: config.server.request_timeout,
            graceful_shutdown: config.server.graceful_shutdown,
            max_request_bytes: config.server.max_request_bytes,
            concurrency_limit: config.server.concurrency_limit,
        }
    }
}

pub struct ServerHandle {
    shutdown: tokio_util::sync::CancellationToken,
    join: JoinHandle<Result<(), HostError>>,
    local_addr: SocketAddr,
}

impl ServerHandle {
    pub fn shutdown(&self) {
        self.shutdown.cancel();
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub async fn wait(self) -> Result<(), HostError> {
        match self.join.await {
            Ok(result) => result,
            Err(error) => Err(HostError::Io(std::io::Error::other(error.to_string()))),
        }
    }
}

pub async fn serve(
    router: Arc<Router>,
    middleware: Arc<MiddlewareStack>,
    services: Arc<ServiceContainer>,
    modules: Vec<Arc<dyn RuntimeModule>>,
    configuration: Arc<Configuration>,
    settings: RuntimeSettings,
    context: HostContext,
) -> Result<ServerHandle, HostError> {
    let app_config = services
        .root_scope()
        .resolve::<AppConfig>()
        .map_err(HostBuildError::Service)
        .map_err(HostError::Build)?;
    let listener = TcpListener::bind(&app_config.server.address)
        .await
        .map_err(HostError::Io)?;
    let local_addr = listener.local_addr().map_err(HostError::Io)?;
    let shutdown = context.background_tasks().cancellation_token();
    let semaphore = Arc::new(Semaphore::new(settings.concurrency_limit));

    let shutdown_for_join = shutdown.clone();
    let join = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = shutdown_for_join.cancelled() => break,
                accept = listener.accept() => {
                    let (stream, _) = accept.map_err(HostError::Io)?;
                    let io = TokioIo::new(stream);
                    let router = Arc::clone(&router);
                    let middleware = Arc::clone(&middleware);
                    let services = Arc::clone(&services);
                    let configuration = Arc::clone(&configuration);
                    let semaphore = Arc::clone(&semaphore);
                    let settings = settings;

                    tokio::spawn(async move {
                        let service = service_fn(move |request: HyperRequest<Incoming>| {
                            let router = Arc::clone(&router);
                            let middleware = Arc::clone(&middleware);
                            let services = Arc::clone(&services);
                            let configuration = Arc::clone(&configuration);
                            let semaphore = Arc::clone(&semaphore);
                            async move {
                                let permit = match semaphore.acquire_owned().await {
                                    Ok(permit) => permit,
                                    Err(_) => {
                                        return Ok::<_, Infallible>(into_hyper_response(
                                            Response::internal_server_error(),
                                        ));
                                    }
                                };
                                let _permit = permit;
                                let response = match build_request(request, settings.max_request_bytes).await {
                                    Ok(request) => dispatch_request(
                                        request,
                                        router,
                                        middleware,
                                        services,
                                        configuration,
                                        settings.request_timeout,
                                    ).await,
                                    Err(response) => response,
                                };
                                Ok::<_, Infallible>(into_hyper_response(response))
                            }
                        });

                        let builder = hyper::server::conn::http1::Builder::new();
                        let _ = builder.serve_connection(io, service).await;
                    });
                }
            }
        }

        context.background_tasks().shutdown().await;
        for module in modules.iter().rev() {
            module
                .on_stop(&context)
                .await
                .map_err(HostError::Framework)?;
        }
        Ok(())
    });

    Ok(ServerHandle {
        shutdown,
        join,
        local_addr,
    })
}

async fn build_request(
    request: HyperRequest<Incoming>,
    max_request_bytes: usize,
) -> Result<Request, Response> {
    let (parts, body) = request.into_parts();
    let method = Method::from_http_str(parts.method.as_str());
    let raw_path = parts
        .uri
        .path_and_query()
        .map(|value| value.as_str().to_string())
        .unwrap_or_else(|| "/".to_string());
    let version = format!("{:?}", parts.version);
    let headers = parts
        .headers
        .iter()
        .filter_map(|(key, value)| {
            value
                .to_str()
                .ok()
                .map(|value| (key.to_string(), value.to_string()))
        })
        .collect::<std::collections::HashMap<_, _>>();
    let collected = body
        .collect()
        .await
        .map_err(|_| Response::bad_request("invalid request body"))?;
    let body_bytes = collected.to_bytes();
    if body_bytes.len() > max_request_bytes {
        return Err(Response::bad_request(
            "request exceeds maximum allowed size",
        ));
    }
    let parsed = Request::from_bytes(
        format!(
            "{} {} HTTP/1.1\r\n{}\r\n\r\n",
            method,
            raw_path,
            headers
                .iter()
                .map(|(key, value)| format!("{key}: {value}\r\n"))
                .collect::<String>()
        )
        .into_bytes()
        .into_iter()
        .chain(body_bytes.iter().copied())
        .collect::<Vec<_>>()
        .as_slice(),
    )
    .map_err(|error| Response::bad_request(error.to_string()))?;

    Ok(Request {
        method: parsed.method,
        path: parsed.path,
        version,
        headers,
        body: body_bytes.to_vec(),
        query_params: parsed.query_params,
    })
}

async fn dispatch_request(
    request: Request,
    router: Arc<Router>,
    middleware: Arc<MiddlewareStack>,
    services: Arc<ServiceContainer>,
    configuration: Arc<Configuration>,
    request_timeout: std::time::Duration,
) -> Response {
    let Some(route) = router.route(&request.method, &request.path) else {
        return Response::not_found();
    };

    let ctx = RequestContext::new(request, route.path_params, services, configuration);
    match timeout(
        request_timeout,
        middleware.execute(&route.middleware, ctx, route.handler),
    )
    .await
    {
        Ok(Ok(response)) => response,
        Ok(Err(error)) => error.to_response(),
        Err(_) => Response::from_error(408, "Request Timeout", "408 Request Timeout"),
    }
}

fn into_hyper_response(response: Response) -> HyperResponse<Full<Bytes>> {
    let mut builder = HyperResponse::builder().status(response.status_code);
    for (key, value) in response.headers {
        builder = builder.header(key, value);
    }
    builder
        .body(Full::new(Bytes::from(response.body)))
        .unwrap_or_else(|_| {
            HyperResponse::new(Full::new(Bytes::from_static(b"response build error")))
        })
}
