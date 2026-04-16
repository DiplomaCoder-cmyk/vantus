use vantus::{Response, controller, get};

#[derive(Clone, Default)]
struct BadController;

#[controller]
impl BadController {
    #[get("/broken")]
    fn broken(&self, (value, _other): (String, String)) -> Response {
        Response::text(value)
    }
}

fn main() {}
