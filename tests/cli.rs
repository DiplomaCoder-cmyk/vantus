#![cfg(feature = "cli")]

use clap::Parser;
use vantus::{CliError, CliFeatureDefaults, IdGeneratorKind, RuntimeMode, ServerCli};

#[test]
fn cli_parses_runtime_mode_and_feature_toggles() {
    let cli = ServerCli::parse_from([
        "vantus-demo",
        "--mode",
        "production",
        "--web-platform",
        "--no-observability",
        "--id-generator",
        "atomic",
    ]);

    assert_eq!(cli.effective_mode(), RuntimeMode::Production);
    assert!(cli.web_platform_enabled(false));
    assert!(!cli.observability_enabled(true));
    assert_eq!(cli.id_generator_kind(), IdGeneratorKind::Atomic);
}

#[test]
fn cli_requires_complete_rate_limit_configuration() {
    let cli = ServerCli::parse_from(["vantus-demo", "--rate-limit-capacity", "100"]);
    let error = cli
        .startup_plan(CliFeatureDefaults::default())
        .expect_err("rate limiting should require all fields");

    assert_eq!(error, CliError::IncompleteRateLimit);
}

#[test]
fn cli_renders_human_readable_startup_plan() {
    let cli = ServerCli::parse_from([
        "vantus-demo",
        "--mode",
        "development",
        "--observability",
        "--request-timeout-ms",
        "2500",
        "--dry-run",
    ]);
    let plan = cli
        .startup_plan(CliFeatureDefaults {
            web_platform: true,
            observability: false,
        })
        .expect("plan should render");

    assert!(plan.contains("mode=development"), "{plan}");
    assert!(plan.contains("environment=development (mode)"), "{plan}");
    assert!(plan.contains("web_platform=true (default)"), "{plan}");
    assert!(plan.contains("observability=true (cli)"), "{plan}");
    assert!(plan.contains("request_timeout_ms=2500 (cli)"), "{plan}");
    assert!(plan.contains("dry_run=true"), "{plan}");
}

#[test]
fn cli_marks_config_backed_runtime_values_as_pending() {
    let cli = ServerCli::parse_from(["vantus-demo"]);
    let plan = cli
        .startup_plan(CliFeatureDefaults::default())
        .expect("plan should render");

    assert!(
        plan.contains("environment=development (env/default)"),
        "{plan}"
    );
    assert!(plan.contains("env_prefix=APP (default)"), "{plan}");
    assert!(
        plan.contains("max_body_bytes=<config/default: 65536> (pending)"),
        "{plan}"
    );
    assert!(
        plan.contains("request_timeout_ms=<config/default: 5000> (pending)"),
        "{plan}"
    );
}

#[test]
fn cli_rejects_zero_rate_limit_capacity() {
    let cli = ServerCli::parse_from([
        "vantus-demo",
        "--rate-limit-capacity",
        "0",
        "--rate-limit-refill-tokens",
        "10",
        "--rate-limit-refill-seconds",
        "60",
    ]);
    let error = cli
        .startup_plan(CliFeatureDefaults::default())
        .expect_err("zero capacity should be rejected");

    assert_eq!(error, CliError::InvalidRateLimitCapacity);
}

#[test]
fn cli_rejects_zero_rate_limit_refill_tokens() {
    let cli = ServerCli::parse_from([
        "vantus-demo",
        "--rate-limit-capacity",
        "10",
        "--rate-limit-refill-tokens",
        "0",
        "--rate-limit-refill-seconds",
        "60",
    ]);
    let error = cli
        .startup_plan(CliFeatureDefaults::default())
        .expect_err("zero refill tokens should be rejected");

    assert_eq!(error, CliError::InvalidRateLimitRefillTokens);
}
