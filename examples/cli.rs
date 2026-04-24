#[cfg(feature = "cli")]
use clap::Parser;
#[cfg(feature = "cli")]
use vantus::{CliFeatureDefaults, HostBuilder, RequestContext, Response, ServerCli, module};

#[cfg(feature = "cli")]
#[derive(Clone)]
struct DemoModule {
    service_name: String,
}

#[cfg(feature = "cli")]
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

#[cfg(feature = "cli")]
fn main() {
    let cli = ServerCli::parse();
    let defaults = CliFeatureDefaults {
        web_platform: true,
        observability: false,
    };

    let plan = match cli.startup_plan(defaults) {
        Ok(plan) => plan,
        Err(error) => {
            eprintln!("Vantus CLI Error: {error}");
            std::process::exit(2);
        }
    };

    if cli.print_startup_plan || cli.dry_run {
        println!("{plan}");
    }

    if cli.dry_run {
        return;
    }

    let mut builder = HostBuilder::new();
    if let Err(error) = cli.apply_to_builder(&mut builder, defaults) {
        eprintln!("Vantus CLI Error: {error}");
        std::process::exit(2);
    }

    builder.compose_with_config(|_configuration, app, context| {
        context.module(DemoModule {
            service_name: app.service_name.clone(),
        });
        Ok(())
    });

    builder.build().run_blocking();
}

#[cfg(not(feature = "cli"))]
fn main() {
    eprintln!(
        "Enable the `cli` feature to run this example: cargo run --example cli --features cli -- --help"
    );
    std::process::exit(1);
}
