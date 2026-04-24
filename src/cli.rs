use std::path::PathBuf;
use std::time::Duration;

use clap::{ArgAction, Parser, ValueEnum};

use crate::{AtomicIdGenerator, GlobalRateLimiter, HostBuilder, UuidIdGenerator};

const DEFAULT_ENVIRONMENT: &str = "development";
const DEFAULT_ENV_PREFIX: &str = "APP";
const DEFAULT_REQUEST_TIMEOUT_MS: u64 = 5_000;
const DEFAULT_MAX_BODY_BYTES: usize = 64 * 1024;

/// First-party CLI bootstrap for Vantus applications.
///
/// This layer is intentionally runtime-focused. It controls configuration
/// selection, module toggles, runtime limits, and startup behavior.
/// Cargo build profiles remain a build-time concern:
/// - use `cargo run` for debug/dev builds
/// - use `cargo run --release` for optimized release builds
#[derive(Debug, Clone, Parser)]
#[command(
    author,
    version,
    about = "Bootstrap a Vantus application with runtime CLI toggles"
)]
pub struct ServerCli {
    /// Base configuration file to load instead of auto-discovery.
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Overrides the runtime environment name.
    #[arg(long, value_name = "NAME")]
    pub environment: Option<String>,

    /// Overrides the active profile used for profile-specific config files.
    #[arg(long, value_name = "NAME")]
    pub profile: Option<String>,

    /// Overrides the environment variable prefix used for config loading.
    #[arg(long, value_name = "PREFIX")]
    pub env_prefix: Option<String>,

    /// Applies runtime defaults for development or production behavior.
    #[arg(long, value_enum)]
    pub mode: Option<RuntimeMode>,

    /// Enables the built-in web platform module.
    #[arg(long, action = ArgAction::SetTrue, overrides_with = "no_web_platform")]
    pub web_platform: bool,

    /// Disables the built-in web platform module.
    #[arg(long = "no-web-platform", action = ArgAction::SetTrue, overrides_with = "web_platform")]
    pub no_web_platform: bool,

    /// Enables the first-party observability module.
    #[arg(long, action = ArgAction::SetTrue, overrides_with = "no_observability")]
    pub observability: bool,

    /// Disables the first-party observability module.
    #[arg(long = "no-observability", action = ArgAction::SetTrue, overrides_with = "observability")]
    pub no_observability: bool,

    /// Overrides the maximum accepted request body size in bytes.
    #[arg(long, value_name = "BYTES")]
    pub max_body_bytes: Option<usize>,

    /// Overrides the outer request timeout in milliseconds.
    #[arg(long, value_name = "MS")]
    pub request_timeout_ms: Option<u64>,

    /// Token bucket capacity for the global rate limiter.
    #[arg(long, value_name = "COUNT")]
    pub rate_limit_capacity: Option<usize>,

    /// Number of tokens restored per refill interval.
    #[arg(long, value_name = "COUNT")]
    pub rate_limit_refill_tokens: Option<usize>,

    /// Rate-limiter refill interval in seconds.
    #[arg(long, value_name = "SECONDS")]
    pub rate_limit_refill_seconds: Option<u64>,

    /// Chooses the shared request-ID generator implementation.
    #[arg(long, value_enum)]
    pub id_generator: Option<IdGeneratorKind>,

    /// Prints the resolved startup plan before booting.
    #[arg(long)]
    pub print_startup_plan: bool,

    /// Prints the startup plan and exits without starting the server.
    #[arg(long)]
    pub dry_run: bool,
}

/// Default states for first-party modules when the CLI does not override them.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CliFeatureDefaults {
    pub web_platform: bool,
    pub observability: bool,
}

/// Runtime preset applied by the CLI.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum RuntimeMode {
    Development,
    Production,
}

impl RuntimeMode {
    fn default_environment(self) -> &'static str {
        match self {
            Self::Development => "development",
            Self::Production => "production",
        }
    }

    fn default_request_timeout(self) -> Duration {
        match self {
            Self::Development => Duration::from_secs(10),
            Self::Production => Duration::from_secs(5),
        }
    }

    fn default_max_body_bytes(self) -> usize {
        match self {
            Self::Development => 128 * 1024,
            Self::Production => 64 * 1024,
        }
    }
}

/// Shared ID generator choices exposed by the CLI.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum IdGeneratorKind {
    Uuid,
    Atomic,
}

#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub enum CliError {
    #[error(
        "rate limiting requires --rate-limit-capacity, --rate-limit-refill-tokens, and --rate-limit-refill-seconds together"
    )]
    IncompleteRateLimit,
    #[error("request timeout must be positive")]
    InvalidRequestTimeout,
    #[error("max body size must be positive")]
    InvalidMaxBodyBytes,
    #[error("rate-limit-capacity must be positive")]
    InvalidRateLimitCapacity,
    #[error("rate-limit-refill-tokens must be positive")]
    InvalidRateLimitRefillTokens,
    #[error("rate-limit-refill-seconds must be positive")]
    InvalidRefillSeconds,
}

impl ServerCli {
    /// Applies CLI-selected defaults, module toggles, and runtime limits.
    pub fn apply_to_builder(
        &self,
        builder: &mut HostBuilder,
        defaults: CliFeatureDefaults,
    ) -> Result<(), CliError> {
        if let Some(mode) = self.mode {
            if self.environment.is_none() {
                builder.environment(mode.default_environment());
            }
            if self.profile.is_none() {
                builder.profile(mode.default_environment());
            }
            if self.request_timeout_ms.is_none() {
                builder.request_timeout(mode.default_request_timeout());
            }
            if self.max_body_bytes.is_none() {
                builder.max_body_size(mode.default_max_body_bytes());
            }
        }

        if let Some(path) = &self.config {
            builder.config_file(path.clone());
        }
        if let Some(environment) = &self.environment {
            builder.environment(environment.clone());
        }
        if let Some(profile) = &self.profile {
            builder.profile(profile.clone());
        }
        if let Some(prefix) = &self.env_prefix {
            builder.env_prefix(prefix.clone());
        }

        if self.web_platform_enabled(defaults.web_platform) {
            builder.with_web_platform();
        }
        if self.observability_enabled(defaults.observability) {
            builder.with_observability();
        }

        if let Some(bytes) = self.max_body_bytes {
            if bytes == 0 {
                return Err(CliError::InvalidMaxBodyBytes);
            }
            builder.max_body_size(bytes);
        }

        if let Some(timeout_ms) = self.request_timeout_ms {
            if timeout_ms == 0 {
                return Err(CliError::InvalidRequestTimeout);
            }
            builder.request_timeout(Duration::from_millis(timeout_ms));
        }

        if let Some(rate_limiter) = self.rate_limiter()? {
            builder.rate_limiter(rate_limiter);
        }

        match self.id_generator_kind() {
            IdGeneratorKind::Uuid => builder.id_generator(UuidIdGenerator),
            IdGeneratorKind::Atomic => builder.id_generator(AtomicIdGenerator::with_prefix("req")),
        };

        Ok(())
    }

    /// Returns the resolved runtime mode, defaulting to development.
    pub fn effective_mode(&self) -> RuntimeMode {
        self.mode.unwrap_or(RuntimeMode::Development)
    }

    /// Returns the resolved ID generator choice, defaulting to UUIDs.
    pub fn id_generator_kind(&self) -> IdGeneratorKind {
        self.id_generator.unwrap_or(IdGeneratorKind::Uuid)
    }

    /// Returns whether the built-in web platform module should be installed.
    pub fn web_platform_enabled(&self, default: bool) -> bool {
        resolve_toggle(self.web_platform, self.no_web_platform, default)
    }

    /// Returns whether the first-party observability module should be installed.
    pub fn observability_enabled(&self, default: bool) -> bool {
        resolve_toggle(self.observability, self.no_observability, default)
    }

    /// Renders a human-readable startup plan suitable for `--dry-run`.
    pub fn startup_plan(&self, defaults: CliFeatureDefaults) -> Result<String, CliError> {
        let (environment, environment_source) = self.startup_environment();
        let (profile, profile_source) = self.startup_profile();
        let (env_prefix, env_prefix_source) = self.startup_env_prefix();
        let (web_platform, web_platform_source) = self.startup_web_platform(defaults.web_platform);
        let (observability, observability_source) =
            self.startup_observability(defaults.observability);
        let (max_body_bytes, max_body_bytes_source) = self.startup_max_body_bytes();
        let (request_timeout_ms, request_timeout_source) = self.startup_request_timeout_ms();
        let (id_generator, id_generator_source) = self.startup_id_generator();
        let rate_limit = match self.rate_limiter()? {
            Some(_) => format!(
                "capacity={}, refill_tokens={}, refill_seconds={} (cli)",
                self.rate_limit_capacity.unwrap_or_default(),
                self.rate_limit_refill_tokens.unwrap_or_default(),
                self.rate_limit_refill_seconds.unwrap_or_default()
            ),
            None => "disabled".to_string(),
        };

        Ok(format!(
            "mode={mode}\nconfig={config}\nenvironment={environment}\nprofile={profile}\nenv_prefix={env_prefix}\nweb_platform={web_platform}\nobservability={observability}\nmax_body_bytes={max_body}\nrequest_timeout_ms={timeout_ms}\nid_generator={id_generator}\nrate_limit={rate_limit}\nprint_startup_plan={print_startup_plan}\ndry_run={dry_run}",
            mode = match self.effective_mode() {
                RuntimeMode::Development => "development",
                RuntimeMode::Production => "production",
            },
            config = self
                .config
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "<auto-discover>".to_string()),
            environment = format!("{environment} ({environment_source})"),
            profile = format!("{profile} ({profile_source})"),
            env_prefix = format!("{env_prefix} ({env_prefix_source})"),
            web_platform = format!("{web_platform} ({web_platform_source})"),
            observability = format!("{observability} ({observability_source})"),
            max_body = format!("{max_body_bytes} ({max_body_bytes_source})"),
            timeout_ms = format!("{request_timeout_ms} ({request_timeout_source})"),
            id_generator = format!("{id_generator} ({id_generator_source})"),
            print_startup_plan = self.print_startup_plan,
            dry_run = self.dry_run,
        ))
    }

    fn rate_limiter(&self) -> Result<Option<GlobalRateLimiter>, CliError> {
        match (
            self.rate_limit_capacity,
            self.rate_limit_refill_tokens,
            self.rate_limit_refill_seconds,
        ) {
            (None, None, None) => Ok(None),
            (Some(capacity), Some(refill_tokens), Some(refill_seconds)) => {
                if capacity == 0 {
                    return Err(CliError::InvalidRateLimitCapacity);
                }
                if refill_tokens == 0 {
                    return Err(CliError::InvalidRateLimitRefillTokens);
                }
                if refill_seconds == 0 {
                    return Err(CliError::InvalidRefillSeconds);
                }
                Ok(Some(GlobalRateLimiter::new(
                    capacity,
                    refill_tokens,
                    Duration::from_secs(refill_seconds),
                )))
            }
            _ => Err(CliError::IncompleteRateLimit),
        }
    }

    fn startup_environment(&self) -> (&str, &'static str) {
        if let Some(environment) = self.environment.as_deref() {
            (environment, "cli")
        } else if let Some(mode) = self.mode {
            (mode.default_environment(), "mode")
        } else {
            (DEFAULT_ENVIRONMENT, "env/default")
        }
    }

    fn startup_profile(&self) -> (&str, &'static str) {
        if let Some(profile) = self.profile.as_deref() {
            (profile, "cli")
        } else if let Some(mode) = self.mode {
            (mode.default_environment(), "mode")
        } else {
            (DEFAULT_ENVIRONMENT, "env/default")
        }
    }

    fn startup_env_prefix(&self) -> (&str, &'static str) {
        if let Some(prefix) = self.env_prefix.as_deref() {
            (prefix, "cli")
        } else {
            (DEFAULT_ENV_PREFIX, "default")
        }
    }

    fn startup_web_platform(&self, default: bool) -> (bool, &'static str) {
        if self.web_platform || self.no_web_platform {
            (self.web_platform_enabled(default), "cli")
        } else {
            (default, "default")
        }
    }

    fn startup_observability(&self, default: bool) -> (bool, &'static str) {
        if self.observability || self.no_observability {
            (self.observability_enabled(default), "cli")
        } else {
            (default, "default")
        }
    }

    fn startup_max_body_bytes(&self) -> (String, &'static str) {
        if let Some(bytes) = self.max_body_bytes {
            (bytes.to_string(), "cli")
        } else if let Some(mode) = self.mode {
            (mode.default_max_body_bytes().to_string(), "mode")
        } else {
            (
                format!("<config/default: {DEFAULT_MAX_BODY_BYTES}>"),
                "pending",
            )
        }
    }

    fn startup_request_timeout_ms(&self) -> (String, &'static str) {
        if let Some(timeout_ms) = self.request_timeout_ms {
            (timeout_ms.to_string(), "cli")
        } else if let Some(mode) = self.mode {
            (
                mode.default_request_timeout().as_millis().to_string(),
                "mode",
            )
        } else {
            (
                format!("<config/default: {DEFAULT_REQUEST_TIMEOUT_MS}>"),
                "pending",
            )
        }
    }

    fn startup_id_generator(&self) -> (&'static str, &'static str) {
        match self.id_generator {
            Some(IdGeneratorKind::Uuid) => ("uuid", "cli"),
            Some(IdGeneratorKind::Atomic) => ("atomic", "cli"),
            None => ("uuid", "default"),
        }
    }
}

fn resolve_toggle(enabled: bool, disabled: bool, default: bool) -> bool {
    if enabled {
        true
    } else if disabled {
        false
    } else {
        default
    }
}
