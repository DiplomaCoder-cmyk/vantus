use std::time::Duration;

use vantus::{GlobalRateLimiter, HostBuilder, RequestContext, Response, module};

#[derive(Clone)]
struct DemoModule {
    service_name: String,
}

#[module]
impl DemoModule {
    #[vantus::get("/")]
    fn index(&self, ctx: RequestContext) -> Response {
        Response::json_value(serde_json::json!({
            "service": self.service_name,
            "environment": ctx.app_config().environment,
            "profile": ctx.app_config().profile,
        }))
    }
}

fn main() {
    let mut builder = HostBuilder::new();
    builder.config_file("examples/application.properties");

    builder.with_observability();

    // 2. Global Security Layer
    builder.max_body_size(32 * 1024);
    builder.request_timeout(Duration::from_secs(3));
    builder.rate_limiter(GlobalRateLimiter::new(60, 60, Duration::from_secs(60)));

    // 3. Web Platform Setup
    builder.with_web_platform();

    builder.compose_with_config(|_configuration, app, context| {
        context.module(DemoModule {
            service_name: app.service_name.clone(),
        });
        Ok(())
    });

    builder.build().run_blocking();
}
