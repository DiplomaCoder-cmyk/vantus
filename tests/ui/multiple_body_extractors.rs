use vantus::module;

#[derive(serde::Deserialize)]
struct Payload {
    value: String,
}

#[derive(Default)]
struct InvalidMultipleBodiesModule;

#[module]
impl InvalidMultipleBodiesModule {
    #[vantus::post("/bad")]
    fn bad(
        &self,
        _text: vantus::TextBody,
        _json: vantus::JsonBody<Payload>,
    ) -> vantus::Response {
        vantus::Response::text("bad")
    }
}

fn main() {}
