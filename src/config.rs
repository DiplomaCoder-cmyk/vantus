use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io::{self, BufRead};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub trait FromConfiguration: Sized {
    fn from_configuration(configuration: &Configuration) -> Result<Self, ConfigError>;
}

#[derive(Clone, Debug)]
pub struct Configuration {
    profile: String,
    environment: String,
    values: HashMap<String, String>,
}

impl Configuration {
    pub fn profile(&self) -> &str {
        &self.profile
    }

    pub fn environment(&self) -> &str {
        &self.environment
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    pub fn get_any<'a>(&'a self, keys: &[&str]) -> Option<&'a str> {
        keys.iter().find_map(|key| self.get(key))
    }

    pub fn get_string(&self, key: &str, default: &str) -> String {
        self.get(key).unwrap_or(default).to_string()
    }

    pub fn get_bool(&self, key: &'static str, default: bool) -> Result<bool, ConfigError> {
        match self.get(key) {
            None => Ok(default),
            Some("true" | "1" | "yes" | "on") => Ok(true),
            Some("false" | "0" | "no" | "off") => Ok(false),
            Some(value) => Err(ConfigError::InvalidValue {
                key,
                value: value.to_string(),
                expected: "boolean",
            }),
        }
    }

    pub fn get_u64(&self, key: &'static str, default: u64) -> Result<u64, ConfigError> {
        self.get(key)
            .map(|value| {
                value.parse::<u64>().map_err(|_| ConfigError::InvalidValue {
                    key,
                    value: value.to_string(),
                    expected: "u64",
                })
            })
            .unwrap_or(Ok(default))
    }

    pub fn get_usize(&self, key: &'static str, default: usize) -> Result<usize, ConfigError> {
        self.get(key)
            .map(|value| {
                value
                    .parse::<usize>()
                    .map_err(|_| ConfigError::InvalidValue {
                        key,
                        value: value.to_string(),
                        expected: "usize",
                    })
            })
            .unwrap_or(Ok(default))
    }
}

pub struct ConfigurationBuilder {
    config_file: Option<PathBuf>,
    profile: Option<String>,
    environment: Option<String>,
    env_prefix: String,
}

impl ConfigurationBuilder {
    pub fn new() -> Self {
        Self {
            config_file: None,
            profile: None,
            environment: None,
            env_prefix: "APP".to_string(),
        }
    }

    pub fn config_file(&mut self, path: impl Into<PathBuf>) -> &mut Self {
        self.config_file = Some(path.into());
        self
    }

    pub fn profile(&mut self, profile: impl Into<String>) -> &mut Self {
        self.profile = Some(profile.into());
        self
    }

    pub fn environment(&mut self, environment: impl Into<String>) -> &mut Self {
        self.environment = Some(environment.into());
        self
    }

    pub fn env_prefix(&mut self, prefix: impl Into<String>) -> &mut Self {
        self.env_prefix = prefix.into();
        self
    }

    pub fn build(&self) -> Result<Configuration, ConfigError> {
        self.build_with_env(std::env::vars())
    }

    pub fn build_with_env<I, K, V>(&self, env: I) -> Result<Configuration, ConfigError>
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        let env_map = env
            .into_iter()
            .map(|(key, value)| (key.as_ref().to_string(), value.as_ref().to_string()))
            .collect::<HashMap<_, _>>();

        let environment = self
            .environment
            .clone()
            .or_else(|| {
                env_map
                    .get(&format!("{}_ENVIRONMENT", self.env_prefix))
                    .cloned()
            })
            .unwrap_or_else(|| "development".to_string());
        let profile = self
            .profile
            .clone()
            .or_else(|| {
                env_map
                    .get(&format!("{}_PROFILE", self.env_prefix))
                    .cloned()
            })
            .unwrap_or_else(|| environment.clone());

        let mut values = HashMap::new();
        let base_path = self
            .config_file
            .clone()
            .unwrap_or_else(|| PathBuf::from("application.properties"));
        if base_path.exists() {
            values.extend(parse_properties(&base_path)?);
        }

        let profile_path = profile_config_path(&base_path, &profile);
        if profile_path.exists() {
            values.extend(parse_properties(&profile_path)?);
        }

        apply_env_overrides(&mut values, env_map.iter(), &self.env_prefix);

        Ok(Configuration {
            profile,
            environment,
            values,
        })
    }
}

impl Default for ConfigurationBuilder {
    fn default() -> Self {
        Self::new()
    }
}

fn profile_config_path(base_path: &Path, profile: &str) -> PathBuf {
    let stem = base_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("application");
    let extension = base_path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("properties");
    let file_name = format!("{stem}.{profile}.{extension}");
    base_path.with_file_name(file_name)
}

fn parse_properties(path: &Path) -> Result<HashMap<String, String>, ConfigError> {
    let file = fs::File::open(path)?;
    let reader = io::BufReader::new(file);
    let mut map = HashMap::new();

    for line in reader.lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = trimmed.split_once('=') {
            map.insert(key.trim().to_string(), value.trim().to_string());
        }
    }

    Ok(map)
}

fn apply_env_overrides<'a, I>(values: &mut HashMap<String, String>, env: I, prefix: &str)
where
    I: IntoIterator<Item = (&'a String, &'a String)>,
{
    let prefix = format!("{prefix}_");
    for (key, value) in env {
        if let Some(stripped) = key.strip_prefix(&prefix) {
            if matches!(stripped, "PROFILE" | "ENVIRONMENT") {
                continue;
            }
            let normalized = stripped.to_ascii_lowercase().replace('_', ".");
            values.insert(normalized, value.clone());
        }
    }
}

#[derive(Debug)]
pub enum ConfigError {
    Io(io::Error),
    MissingKey(&'static str),
    InvalidValue {
        key: &'static str,
        value: String,
        expected: &'static str,
    },
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Io(error) => write!(f, "config IO error: {error}"),
            ConfigError::MissingKey(key) => write!(f, "missing config key {key}"),
            ConfigError::InvalidValue {
                key,
                value,
                expected,
            } => {
                write!(f, "invalid value for {key}: {value} ({expected})")
            }
        }
    }
}

impl std::error::Error for ConfigError {}

impl From<io::Error> for ConfigError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

#[derive(Clone, Debug)]
pub struct ServerConfig {
    pub address: String,
    pub request_timeout: Duration,
    pub graceful_shutdown: Duration,
    pub max_request_bytes: usize,
    pub concurrency_limit: usize,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            address: "127.0.0.1:8080".to_string(),
            request_timeout: Duration::from_secs(5),
            graceful_shutdown: Duration::from_secs(5),
            max_request_bytes: 64 * 1024,
            concurrency_limit: 1024,
        }
    }
}

#[derive(Clone, Debug)]
pub struct AppConfig {
    pub service_name: String,
    pub environment: String,
    pub profile: String,
    pub enable_info: bool,
    pub readiness: bool,
    pub server: ServerConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            service_name: "mini-backend".to_string(),
            environment: "development".to_string(),
            profile: "development".to_string(),
            enable_info: true,
            readiness: true,
            server: ServerConfig::default(),
        }
    }
}

impl FromConfiguration for AppConfig {
    fn from_configuration(configuration: &Configuration) -> Result<Self, ConfigError> {
        let service_name = configuration
            .get_any(&["service.name"])
            .unwrap_or("mini-backend")
            .to_string();
        if service_name.trim().is_empty() {
            return Err(ConfigError::MissingKey("service.name"));
        }

        let enable_info = parse_bool_alias(
            configuration,
            &["service.enable-info", "service.enable.info"],
            true,
        )?;
        let readiness =
            parse_bool_alias(configuration, &["service.readiness", "service.ready"], true)?;
        let address = configuration
            .get_any(&["server.address"])
            .unwrap_or("127.0.0.1:8080")
            .to_string();
        let request_timeout = Duration::from_secs(parse_u64_alias(
            configuration,
            &[
                "server.request-timeout-seconds",
                "server.request.timeout.seconds",
            ],
            5,
        )?);
        let graceful_shutdown = Duration::from_secs(parse_u64_alias(
            configuration,
            &[
                "server.graceful-shutdown-seconds",
                "server.graceful.shutdown.seconds",
            ],
            5,
        )?);
        let max_request_bytes = parse_usize_alias(
            configuration,
            &["server.max-request-bytes", "server.max.request.bytes"],
            64 * 1024,
        )?;
        let concurrency_limit = parse_usize_alias(
            configuration,
            &["server.concurrency-limit", "server.concurrency.limit"],
            1024,
        )?;

        Ok(Self {
            service_name,
            environment: configuration.environment().to_string(),
            profile: configuration.profile().to_string(),
            enable_info,
            readiness,
            server: ServerConfig {
                address,
                request_timeout,
                graceful_shutdown,
                max_request_bytes,
                concurrency_limit,
            },
        })
    }
}

fn parse_bool_alias(
    configuration: &Configuration,
    keys: &[&'static str],
    default: bool,
) -> Result<bool, ConfigError> {
    match configuration.get_any(keys) {
        None => Ok(default),
        Some("true" | "1" | "yes" | "on") => Ok(true),
        Some("false" | "0" | "no" | "off") => Ok(false),
        Some(value) => Err(ConfigError::InvalidValue {
            key: keys[0],
            value: value.to_string(),
            expected: "boolean",
        }),
    }
}

fn parse_u64_alias(
    configuration: &Configuration,
    keys: &[&'static str],
    default: u64,
) -> Result<u64, ConfigError> {
    configuration
        .get_any(keys)
        .map(|value| {
            value.parse::<u64>().map_err(|_| ConfigError::InvalidValue {
                key: keys[0],
                value: value.to_string(),
                expected: "u64",
            })
        })
        .unwrap_or(Ok(default))
}

fn parse_usize_alias(
    configuration: &Configuration,
    keys: &[&'static str],
    default: usize,
) -> Result<usize, ConfigError> {
    configuration
        .get_any(keys)
        .map(|value| {
            value
                .parse::<usize>()
                .map_err(|_| ConfigError::InvalidValue {
                    key: keys[0],
                    value: value.to_string(),
                    expected: "usize",
                })
        })
        .unwrap_or(Ok(default))
}
