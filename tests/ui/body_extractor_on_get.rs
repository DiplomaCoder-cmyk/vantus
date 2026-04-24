use vantus::module;

#[derive(Default)]
struct InvalidGetBodyModule;

#[module]
impl InvalidGetBodyModule {
    #[vantus::get("/bad")]
    fn bad(&self, _body: vantus::TextBody) -> vantus::Response {
        vantus::Response::text("bad")
    }
}

fn main() {}
