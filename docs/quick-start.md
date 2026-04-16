# Quick Start

1. Create a module with `#[module]`.
2. Register services in `configure_services`.
3. Add route methods with `#[mini_backend::get]` / `#[mini_backend::post]`.
4. Mount the module with `HostBuilder`.
5. Build and run the host on `tokio`.

See [../examples/macro_controller.rs](../examples/macro_controller.rs) for a minimal runnable example.
