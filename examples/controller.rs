use vantus::{
    AppConfig, Config, FrameworkError, HostBuilder, Response, Service, TextBody, module,
};
use serde::Serialize;

#[derive(Default)]
struct GreetingService;

impl GreetingService {
    fn greet(&self, value: &str) -> String {
        format!("hello {value}")
    }
}

#[derive(Clone, Default)]
struct GreetingModule;

#[module]
impl GreetingModule {
    fn configure_services(
        &self,
        services: &mut vantus::ServiceCollection,
    ) -> Result<(), FrameworkError> {
        services.add_singleton(GreetingService);
        Ok(())
    }

    #[vantus::post("/greet")]
    fn greet(
        &self,
        config: Config<AppConfig>,
        service: Service<GreetingService>,
        body: TextBody,
    ) -> Result<Response, FrameworkError> {
        #[derive(Serialize)]
        struct Payload {
            service: String,
            message: String,
        }

        Response::json_serialized(&Payload {
            service: config.as_ref().service_name.clone(),
            message: service.as_ref().greet(body.as_str()),
        })
        .map_err(|error| FrameworkError::Internal(error.to_string()))
    }
}

#[derive(Clone, Default)]
struct RootModule;

#[module]
impl RootModule {
    #[vantus::get("/")]
    fn home(&self) -> Response {
        Response::text("macro-first vantus")
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut builder = HostBuilder::new();
    builder.module(RootModule);
    builder.group("/api", |api| {
        api.module(GreetingModule);
    });

    let host = builder.build()?;
    host.run().await?;
    Ok(())
}
