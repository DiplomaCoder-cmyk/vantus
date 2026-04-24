use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use vantus::{
    AppConfig, ConfigError, ConfigurationBuilder, FrameworkError, FromConfig, FromConfiguration,
    ServerProtocol,
};

fn temp_config_path(name: &str, extension: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("vantus-config-{name}-{unique}.{extension}"))
}

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
fn environment_variables_override_profile_and_base_values() {
    let base = temp_config_path("env-override-base", "properties");
    let profile = base.with_file_name(format!(
        "{}.production.properties",
        base.file_stem().unwrap().to_string_lossy()
    ));
    fs::write(
        &base,
        "service.name=base-service\nserver.bind-port=7000\nserver.concurrency-limit=16\n",
    )
    .unwrap();
    fs::write(
        &profile,
        "service.name=profile-service\nserver.bind-port=7100\nserver.concurrency-limit=24\n",
    )
    .unwrap();

    let mut builder = ConfigurationBuilder::new();
    builder.config_file(&base).profile("production");
    let config = builder
        .build_with_env([
            ("APP_SERVICE_NAME", "env-service"),
            ("APP_SERVER_BIND_PORT", "7200"),
            ("APP_SERVER_CONCURRENCY_LIMIT", "64"),
        ])
        .unwrap();
    let app = AppConfig::from_configuration(&config).unwrap();

    assert_eq!(app.service_name, "env-service");
    assert_eq!(app.server.bind_port, 7200);
    assert_eq!(app.server.concurrency_limit, 64);

    fs::remove_file(base).unwrap();
    fs::remove_file(profile).unwrap();
}

#[test]
fn profile_specific_toml_files_are_loaded() {
    let base = temp_config_path("profile-toml", "toml");
    let profile = base.with_file_name(format!(
        "{}.staging.toml",
        base.file_stem().unwrap().to_string_lossy()
    ));
    fs::write(
        &base,
        r#"
[service]
name = "base-service"

[server]
"bind-port" = 8080
"tcp-nodelay" = true
"#,
    )
    .unwrap();
    fs::write(
        &profile,
        r#"
[service]
name = "staging-service"

[server]
"bind-port" = 9090
"tcp-nodelay" = false
"#,
    )
    .unwrap();

    let mut builder = ConfigurationBuilder::new();
    builder.config_file(&base).profile("staging");
    let config = builder
        .build_with_env(std::iter::empty::<(&str, &str)>())
        .unwrap();
    let app = AppConfig::from_configuration(&config).unwrap();

    assert_eq!(app.profile, "staging");
    assert_eq!(app.service_name, "staging-service");
    assert_eq!(app.server.bind_port, 9090);
    assert!(!app.server.tcp_nodelay);

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

#[test]
fn production_environment_disables_info_by_default() {
    let base = temp_config_path("production-defaults", "properties");
    fs::write(&base, "service.name=prod-service\n").unwrap();
    let mut builder = ConfigurationBuilder::new();
    builder.config_file(&base);
    let config = builder
        .build_with_env([("APP_ENVIRONMENT", "production")])
        .unwrap();
    let app = AppConfig::from_configuration(&config).unwrap();
    assert!(!app.enable_info);
    fs::remove_file(base).unwrap();
}

#[test]
fn invalid_bind_host_fails_fast() {
    let builder = ConfigurationBuilder::new();
    let config = builder
        .build_with_env([("APP_SERVER_BIND_HOST", "localhost")])
        .unwrap();
    let error = AppConfig::from_configuration(&config).unwrap_err();
    assert!(error.to_string().contains("server.bind-host"));
}

#[test]
fn runtime_tuning_config_is_bound() {
    let builder = ConfigurationBuilder::new();
    let config = builder
        .build_with_env([
            ("APP_SERVER_PROTOCOL", "http2"),
            ("APP_SERVER_READ_TIMEOUT_SECONDS", "7"),
            ("APP_SERVER_HANDLER_TIMEOUT_SECONDS", "9"),
            ("APP_SERVER_TCP_NODELAY", "false"),
        ])
        .unwrap();
    let app = AppConfig::from_configuration(&config).unwrap();

    assert_eq!(app.server.protocol, ServerProtocol::Http2);
    assert_eq!(app.server.read_timeout.as_secs(), 7);
    assert_eq!(app.server.handler_timeout.as_secs(), 9);
    assert!(!app.server.tcp_nodelay);
}

#[test]
fn framework_error_preserves_config_error_text() {
    let error = FrameworkError::from(ConfigError::MissingKey("service.name"));
    assert!(matches!(&error, FrameworkError::Startup { .. }));
    assert!(
        error
            .to_string()
            .contains("missing config key service.name")
    );
}

#[test]
fn framework_error_wraps_parse_errors_as_bad_request() {
    let error = FrameworkError::from(vantus::ParseError::InvalidPercentEncoding);

    match error {
        FrameworkError::Http(http) => assert_eq!(http.status_code, 400),
        other => panic!("expected http error, got {other:?}"),
    }
}

#[test]
fn framework_error_exposes_http_source() {
    let error = FrameworkError::from(vantus::HttpError::bad_request("bad input"));
    assert!(matches!(&error, FrameworkError::Http(_)));
    assert_eq!(error.to_string(), "400 Bad Request");
}
#[derive(Debug, Deserialize)]
struct BoundServiceConfig {
    service: BoundServiceMetadata,
    server: BoundServerConfig,
}

#[derive(Debug, Deserialize)]
struct BoundServiceMetadata {
    #[serde(rename = "service.name")]
    name: String,

    #[serde(rename = "service.enable.info")]
    enable_info: bool,
}

#[derive(Debug, Deserialize)]
struct BoundServerConfig {
    #[serde(rename = "server.concurrency.limit")]
    concurrency_limit: u32,

    #[serde(rename = "server.tcp.nodelay")]
    tcp_nodelay: bool,
}
#[test]
fn configuration_bind_supports_serde_structs() {
    let builder = ConfigurationBuilder::new();
    let config = builder
        .build_with_env([
            ("APP_SERVICE_NAME", "serde-service"),
            ("APP_SERVICE_ENABLE_INFO", "false"),
            ("APP_SERVER_CONCURRENCY_LIMIT", "32"),
            ("APP_SERVER_TCP_NODELAY", "true"),
        ])
        .unwrap();

    let bound = config.bind::<BoundServiceConfig>().unwrap();
    assert_eq!(bound.service.name, "serde-service");
    assert!(!bound.service.enable_info);
    assert_eq!(bound.server.concurrency_limit, 32);
    assert!(bound.server.tcp_nodelay);
}

#[derive(Clone, Debug, Deserialize)]
struct ProjectionRoot {
    feature: ProjectionLeaf,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
struct ProjectionLeaf {
    name: String,
}

impl FromConfiguration for ProjectionRoot {
    fn from_configuration(configuration: &vantus::Configuration) -> Result<Self, ConfigError> {
        configuration.bind()
    }
}

impl FromConfig<ProjectionRoot> for ProjectionLeaf {
    fn from_config(root: &ProjectionRoot) -> Result<Self, ConfigError> {
        Ok(root.feature.clone())
    }
}

#[test]
fn from_config_projects_nested_settings() {
    let builder = ConfigurationBuilder::new();
    let config = builder
        .build_with_env([("APP_FEATURE_NAME", "lens-ready")])
        .unwrap();

    let root = ProjectionRoot::from_configuration(&config).unwrap();
    let leaf = ProjectionLeaf::from_config(&root).unwrap();

    assert_eq!(
        leaf,
        ProjectionLeaf {
            name: "lens-ready".to_string(),
        }
    );
}
