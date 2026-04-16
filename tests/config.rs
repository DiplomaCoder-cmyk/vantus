use std::fs;
use std::path::Path;

use vantus::{AppConfig, ConfigurationBuilder, FromConfiguration};

#[test]
fn configuration_layers_base_profile_and_env() {
    let base = Path::new("tests/application.properties");
    let profile = Path::new("tests/application.production.properties");
    fs::write(
        base,
        "service.name=base-service\nservice.enable-info=true\nserver.address=127.0.0.1:9000\n",
    )
    .unwrap();
    fs::write(
        profile,
        "service.name=profile-service\nservice.readiness=false\n",
    )
    .unwrap();

    let mut builder = ConfigurationBuilder::new();
    builder.config_file(base).profile("production");
    let config = builder
        .build_with_env([
            ("APP_SERVER_CONCURRENCY_LIMIT", "64"),
            ("APP_ENVIRONMENT", "production"),
        ])
        .unwrap();
    let app = AppConfig::from_configuration(&config).unwrap();

    assert_eq!(app.service_name, "profile-service");
    assert_eq!(app.environment, "production");
    assert!(!app.readiness);
    assert_eq!(app.server.address, "127.0.0.1:9000");
    assert_eq!(app.server.concurrency_limit, 64);

    fs::remove_file(base).unwrap();
    fs::remove_file(profile).unwrap();
}

#[test]
fn invalid_boolean_values_fail_fast() {
    let builder = ConfigurationBuilder::new();
    let config = builder
        .build_with_env([("APP_SERVICE_ENABLE_INFO", "maybe")])
        .unwrap();
    let error = AppConfig::from_configuration(&config).unwrap_err();
    assert!(error.to_string().contains("service.enable-info"));
}
