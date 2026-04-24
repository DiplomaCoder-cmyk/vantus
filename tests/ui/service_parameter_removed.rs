use vantus::module;

struct Service<T>(T);

#[derive(Default)]
struct InvalidServiceModule;

#[module]
impl InvalidServiceModule {
    #[vantus::get("/bad")]
    fn bad(
        &self,
        _generator: Service<std::sync::Arc<dyn vantus::IdGenerator>>,
    ) -> vantus::Response {
        vantus::Response::text("bad")
    }
}

fn main() {}
