use vantus::module;

#[derive(Default)]
struct InvalidPathParameterModule;

#[module]
impl InvalidPathParameterModule {
    #[vantus::get("/users/{id}")]
    fn show(&self, name: vantus::Path<String>) -> vantus::Response {
        vantus::Response::text(name.into_inner())
    }
}

fn main() {}
