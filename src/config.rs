use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io::{self, BufRead};
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde_json::{Map, Number, Value};

/// Manual configuration binding contract.
///
/// Implement this when your type needs custom parsing or validation beyond
/// what the serde-backed `Configuration::bind::<T>()` helper provides.
pub trait FromConfiguration: Sized {
    fn from_configuration(configuration: &Configuration) -> Result<Self, ConfigError>;
}

/// Projects a sub-config from an already bound root config.
///
/// This is useful when application code wants a focused nested settings type
/// without rebinding the full layered configuration.
pub trait FromConfig<Root>: Sized {
    fn from_config(root: &Root) -> Result<Self, ConfigError>;
}

/// Layered key-value configuration store used by the framework.
///
/// The framework loads `.properties` files and environment overrides into this
/// structure, then application code can either query raw keys or bind typed
/// config structs from it.
#[derive(Clone, Debug)]
pub struct Configuration {
    profile: String,
    environment: String,
    values: HashMap<String, String>,
}

impl Configuration {
    /// Returns the resolved configuration profile name.
    pub fn profile(&self) -> &str {
        &self.profile
    }

    /// Returns the resolved environment name.
    pub fn environment(&self) -> &str {
        &self.environment
    }

    /// Reads a raw key from the merged key-value store.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    pub fn get_any<'a>(&'a self, keys: &[&str]) -> Option<&'a str> {
        // Later aliases win so environment-derived dotted keys can override
        // hyphenated file keys during publish/deploy configuration.
        keys.iter().rev().find_map(|key| self.get(key))
    }

    /// Reads a string key or returns the provided default.
    pub fn get_string(&self, key: &str, default: &str) -> String {
        self.get(key).unwrap_or(default).to_string()
    }

    /// Reads a typed config struct using serde.
    ///
    /// This works best for app-specific config such as:
    /// `demo.store-name` -> `DemoConfig { demo: DemoSettings { store_name, .. } }`
    pub fn bind<T>(&self) -> Result<T, ConfigError>
    where
        T: DeserializeOwned,
    {
        serde_json::from_value(self.as_json_value()).map_err(|error| ConfigError::Deserialize {
            type_name: std::any::type_name::<T>(),
            message: error.to_string(),
        })
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

    fn as_json_value(&self) -> Value {
        let mut root = Map::new();
        for (key, value) in &self.values {
            insert_json_path(&mut root, key, parse_json_scalar(value));
        }
        Value::Object(root)
    }
}

/// Builder for layered configuration loading.
///
/// Sources are merged in this order:
/// - base properties file
/// - profile-specific properties file
/// - environment variables
pub struct ConfigurationBuilder {
    pub(crate) config_file: Option<PathBuf>,
    profile: Option<String>,
    environment: Option<String>,
    env_prefix: String,
}

impl ConfigurationBuilder {
    /// Creates a layered configuration builder using the default `APP_` env prefix.
    pub fn new() -> Self {
        Self {
            config_file: None,
            profile: None,
            environment: None,
            env_prefix: "APP".to_string(),
        }
    }
    /// Returns true if a configuration file path has been explicitly set.
    pub fn is_file_set(&self) -> bool {
        self.config_file.is_some()
    }

    /// Sets the base configuration file used for layered loading.
    pub fn config_file(&mut self, path: impl Into<PathBuf>) -> &mut Self {
        self.config_file = Some(path.into());
        self
    }

    /// Overrides the active profile used for profile-specific file lookup.
    pub fn profile(&mut self, profile: impl Into<String>) -> &mut Self {
        self.profile = Some(profile.into());
        self
    }

    /// Overrides the active environment name.
    pub fn environment(&mut self, environment: impl Into<String>) -> &mut Self {
        self.environment = Some(environment.into());
        self
    }

    /// Changes the environment variable prefix used for overrides.
    pub fn env_prefix(&mut self, prefix: impl Into<String>) -> &mut Self {
        self.env_prefix = prefix.into();
        self
    }

    /// Builds the merged configuration from files and process environment.
    pub fn build(&self) -> Result<Configuration, ConfigError> {
        self.build_with_env(std::env::vars())
    }

    /// Builds the merged configuration from files and a caller-provided env source.
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

        // 1. Resolve Environment & Profile
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

        // 2. SMART DISCOVERY: Probes for the best available file
        let base_path = self.config_file.clone().unwrap_or_else(|| {
            let extensions = supported_config_extensions();
            let search_paths = ["", "examples/"];

            for folder in search_paths {
                for ext in extensions {
                    let path = PathBuf::from(format!("{}application.{}", folder, ext));
                    if path.exists() {
                        return path;
                    }
                }
            }
            PathBuf::from("application.properties")
        });

        // 3. Load Base File (using your dynamic parse_file)
        if base_path.exists() {
            values.extend(self.parse_file(&base_path)?);
        }

        // 4. Load Profile-Specific File (e.g. application-dev.json)
        let profile_path = profile_config_path(&base_path, &profile);
        if profile_path.exists() {
            values.extend(self.parse_file(&profile_path)?);
        }

        // 5. Layer Environment Overrides (Last word)
        apply_env_overrides(&mut values, env_map.iter(), &self.env_prefix);
        Ok(Configuration {
            profile,
            environment,
            values,
        })
    }
    /// New helper to dispatch parsing based on file extension
    fn parse_file(&self, path: &PathBuf) -> Result<HashMap<String, String>, ConfigError> {
        let extension = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("properties");

        match extension {
            "toml" => parse_toml(path),
            "yaml" | "yml" => parse_yaml(path),
            "json" => parse_json(path),
            _ => parse_properties(path),
        }
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

fn flatten_value(value: &serde_json::Value, prefix: String, map: &mut HashMap<String, String>) {
    match value {
        serde_json::Value::Object(obj) => {
            for (k, v) in obj {
                let new_prefix = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{}.{}", prefix, k)
                };
                flatten_value(v, new_prefix, map);
            }
        }
        serde_json::Value::Array(arr) => {
            for (i, v) in arr.iter().enumerate() {
                let new_prefix = format!("{}[{}]", prefix, i);
                flatten_value(v, new_prefix, map);
            }
        }
        _ => {
            map.insert(prefix, value.to_string().trim_matches('"').to_string());
        }
    }
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
fn parse_toml(path: &PathBuf) -> Result<HashMap<String, String>, ConfigError> {
    let content = std::fs::read_to_string(path).map_err(|_| ConfigError::FileNotFound)?;
    let value: serde_json::Value =
        toml::from_str(&content).map_err(|_| ConfigError::InvalidFormat)?;

    let mut map = HashMap::new();
    flatten_value(&value, String::new(), &mut map);
    Ok(map)
}
fn parse_json(path: &PathBuf) -> Result<HashMap<String, String>, ConfigError> {
    let content = std::fs::read_to_string(path).map_err(|_| ConfigError::FileNotFound)?;
    let value: serde_json::Value =
        serde_json::from_str(&content).map_err(|_| ConfigError::InvalidFormat)?;

    let mut map = HashMap::new();
    flatten_value(&value, String::new(), &mut map);
    Ok(map)
}

#[cfg(feature = "yaml-config")]
fn parse_yaml(path: &PathBuf) -> Result<HashMap<String, String>, ConfigError> {
    let content = std::fs::read_to_string(path).map_err(|_| ConfigError::FileNotFound)?;
    let value: serde_json::Value =
        serde_yaml::from_str(&content).map_err(|_| ConfigError::InvalidFormat)?;

    let mut map = HashMap::new();
    flatten_value(&value, String::new(), &mut map);
    Ok(map)
}

#[cfg(not(feature = "yaml-config"))]
fn parse_yaml(_path: &PathBuf) -> Result<HashMap<String, String>, ConfigError> {
    Err(ConfigError::InvalidFormat)
}

#[cfg(feature = "yaml-config")]
fn supported_config_extensions() -> &'static [&'static str] {
    &["toml", "json", "yaml", "yml", "properties"]
}

#[cfg(not(feature = "yaml-config"))]
fn supported_config_extensions() -> &'static [&'static str] {
    &["toml", "json", "properties"]
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

fn insert_json_path(root: &mut Map<String, Value>, key: &str, value: Value) {
    let mut current_map = root;
    let parts: Vec<String> = key.split('.').map(normalize_bind_key).collect();
    let len = parts.len();
    let normalized_key = parts.join(".");

    for (i, part) in parts.iter().enumerate() {
        let normalized_part = part.clone();

        if i == len - 1 {
            current_map.insert(normalized_part, value);
            return;
        }

        let entry = current_map
            .entry(normalized_part)
            .or_insert_with(|| Value::Object(Map::new()));

        // Nested serde binding expects both the current segment and the full
        // normalized dotted key to be available while descending.
        if !entry.is_object() {
            *entry = Value::Object(Map::new());
        }
        current_map = entry.as_object_mut().unwrap();
        current_map.insert(normalized_key.clone(), value.clone());
    }
}

fn parse_json_scalar(value: &str) -> Value {
    match value {
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        _ => {
            if let Ok(int) = value.parse::<i64>() {
                return Value::Number(Number::from(int));
            }
            if let Ok(uint) = value.parse::<u64>() {
                return Value::Number(Number::from(uint));
            }
            if let Ok(float) = value.parse::<f64>() {
                if let Some(number) = Number::from_f64(float) {
                    return Value::Number(number);
                }
            }
            Value::String(value.to_string())
        }
    }
}

fn normalize_bind_key(segment: &str) -> String {
    segment.replace('-', "_")
}

#[derive(Debug)]
/// Errors produced while loading or binding configuration.
pub enum ConfigError {
    Io(io::Error),
    FileNotFound,
    InvalidFormat,
    MissingKey(&'static str),
    Deserialize {
        type_name: &'static str,
        message: String,
    },
    InvalidValue {
        key: &'static str,
        value: String,
        expected: &'static str,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// Server protocol mode selected from configuration.
pub enum ServerProtocol {
    Http1,
    Http2,
    Auto,
}

impl ServerProtocol {
    fn parse(value: &str) -> Result<Self, ConfigError> {
        match value.trim().to_ascii_lowercase().as_str() {
            "http1" | "http/1" | "http/1.1" => Ok(Self::Http1),
            "http2" | "h2" | "http/2" => Ok(Self::Http2),
            "auto" => Ok(Self::Auto),
            other => Err(ConfigError::InvalidValue {
                key: "server.protocol",
                value: other.to_string(),
                expected: "http1 | http2 | auto",
            }),
        }
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::Io(error) => write!(f, "config IO error: {error}"),
            ConfigError::MissingKey(key) => write!(f, "missing config key {key}"),
            ConfigError::Deserialize { type_name, message } => {
                write!(
                    f,
                    "failed to bind configuration into {type_name}: {message}"
                )
            }
            ConfigError::InvalidValue {
                key,
                value,
                expected,
            } => {
                write!(f, "invalid value for {key}: {value} ({expected})")
            }
            ConfigError::FileNotFound => write!(f, "Configuration file not found"),
            ConfigError::InvalidFormat => write!(f, "Configuration file has an invalid format"),
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
/// Parsed server settings from `AppConfig`.
pub struct ServerConfig {
    pub address: String,
    pub bind_host: String,
    pub bind_port: u16,
    pub protocol: ServerProtocol,
    pub request_timeout: Duration,
    pub read_timeout: Duration,
    pub handler_timeout: Duration,
    pub graceful_shutdown: Duration,
    pub max_request_bytes: usize,
    pub max_header_bytes: usize,
    pub max_header_count: usize,
    pub concurrency_limit: usize,
    pub keep_alive: bool,
    pub tcp_nodelay: bool,
    pub trusted_proxies: Vec<IpAddr>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            address: "127.0.0.1:8080".to_string(),
            bind_host: "127.0.0.1".to_string(),
            bind_port: 8080,
            protocol: ServerProtocol::Auto,
            request_timeout: Duration::from_secs(5),
            read_timeout: Duration::from_secs(5),
            handler_timeout: Duration::from_secs(5),
            graceful_shutdown: Duration::from_secs(5),
            max_request_bytes: 64 * 1024,
            max_header_bytes: 8 * 1024,
            max_header_count: 64,
            concurrency_limit: 1024,
            keep_alive: true,
            tcp_nodelay: true,
            trusted_proxies: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
/// Runtime-ready server settings after validation.
pub struct ServerOptions {
    pub bind_address: SocketAddr,
    pub protocol: ServerProtocol,
    pub request_timeout: Duration,
    pub read_timeout: Duration,
    pub handler_timeout: Duration,
    pub graceful_shutdown: Duration,
    pub max_request_bytes: usize,
    pub max_header_bytes: usize,
    pub max_header_count: usize,
    pub concurrency_limit: usize,
    pub keep_alive: bool,
    pub tcp_nodelay: bool,
    pub trusted_proxies: Vec<IpAddr>,
}

impl TryFrom<&ServerConfig> for ServerOptions {
    type Error = ConfigError;

    fn try_from(value: &ServerConfig) -> Result<Self, Self::Error> {
        if value.max_request_bytes == 0 {
            return Err(ConfigError::InvalidValue {
                key: "server.max-request-bytes",
                value: value.max_request_bytes.to_string(),
                expected: "positive usize",
            });
        }
        if value.max_header_bytes == 0 {
            return Err(ConfigError::InvalidValue {
                key: "server.max-header-bytes",
                value: value.max_header_bytes.to_string(),
                expected: "positive usize",
            });
        }
        if value.max_header_count == 0 {
            return Err(ConfigError::InvalidValue {
                key: "server.max-header-count",
                value: value.max_header_count.to_string(),
                expected: "positive usize",
            });
        }
        if value.concurrency_limit == 0 {
            return Err(ConfigError::InvalidValue {
                key: "server.concurrency-limit",
                value: value.concurrency_limit.to_string(),
                expected: "positive usize",
            });
        }

        let ip = value
            .bind_host
            .parse::<IpAddr>()
            .map_err(|_| ConfigError::InvalidValue {
                key: "server.bind-host",
                value: value.bind_host.clone(),
                expected: "IP address",
            })?;

        Ok(Self {
            bind_address: SocketAddr::new(ip, value.bind_port),
            protocol: value.protocol,
            request_timeout: value.request_timeout,
            read_timeout: value.read_timeout,
            handler_timeout: value.handler_timeout,
            graceful_shutdown: value.graceful_shutdown,
            max_request_bytes: value.max_request_bytes,
            max_header_bytes: value.max_header_bytes,
            max_header_count: value.max_header_count,
            concurrency_limit: value.concurrency_limit,
            keep_alive: value.keep_alive,
            tcp_nodelay: value.tcp_nodelay,
            trusted_proxies: value.trusted_proxies.clone(),
        })
    }
}

#[derive(Clone, Debug)]
/// Built-in framework configuration model.
///
/// Applications can use this directly, embed it in larger config models,
/// or register their own typed config alongside it.
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

        let is_development = configuration.environment() == "development";
        let enable_info = parse_bool_alias(
            configuration,
            &["service.enable-info", "service.enable.info"],
            is_development,
        )?;
        let readiness =
            parse_bool_alias(configuration, &["service.readiness", "service.ready"], true)?;
        let default_server = ServerConfig::default();
        let address = configuration
            .get_any(&["server.address"])
            .unwrap_or(default_server.address.as_str())
            .to_string();
        let (derived_host, derived_port) = parse_socket_parts(&address)
            .unwrap_or((default_server.bind_host, default_server.bind_port));
        let bind_host = configuration
            .get_any(&["server.bind-host", "server.bind.host"])
            .unwrap_or(derived_host.as_str())
            .to_string();
        let bind_port = parse_u16_alias(
            configuration,
            &["server.bind-port", "server.bind.port"],
            derived_port,
        )?;
        let request_timeout = Duration::from_secs(parse_u64_alias(
            configuration,
            &[
                "server.request-timeout-seconds",
                "server.request.timeout.seconds",
            ],
            5,
        )?);
        let read_timeout = Duration::from_secs(parse_u64_alias(
            configuration,
            &["server.read-timeout-seconds", "server.read.timeout.seconds"],
            request_timeout.as_secs(),
        )?);
        let handler_timeout = Duration::from_secs(parse_u64_alias(
            configuration,
            &[
                "server.handler-timeout-seconds",
                "server.handler.timeout.seconds",
            ],
            request_timeout.as_secs(),
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
            default_server.max_request_bytes,
        )?;
        let max_header_bytes = parse_usize_alias(
            configuration,
            &["server.max-header-bytes", "server.max.header.bytes"],
            default_server.max_header_bytes,
        )?;
        let max_header_count = parse_usize_alias(
            configuration,
            &["server.max-header-count", "server.max.header.count"],
            default_server.max_header_count,
        )?;
        let concurrency_limit = parse_usize_alias(
            configuration,
            &["server.concurrency-limit", "server.concurrency.limit"],
            default_server.concurrency_limit,
        )?;
        let keep_alive = parse_bool_alias(
            configuration,
            &["server.keep-alive", "server.keep.alive"],
            default_server.keep_alive,
        )?;
        let tcp_nodelay = parse_bool_alias(
            configuration,
            &["server.tcp-nodelay", "server.tcp.nodelay"],
            default_server.tcp_nodelay,
        )?;
        let protocol = configuration
            .get_any(&["server.protocol"])
            .map(ServerProtocol::parse)
            .transpose()?
            .unwrap_or(default_server.protocol);
        let trusted_proxies = parse_ip_list_alias(
            configuration,
            &["server.trusted-proxies", "server.trusted.proxies"],
        )?;

        let server = ServerConfig {
            address: format!("{bind_host}:{bind_port}"),
            bind_host,
            bind_port,
            protocol,
            request_timeout,
            read_timeout,
            handler_timeout,
            graceful_shutdown,
            max_request_bytes,
            max_header_bytes,
            max_header_count,
            concurrency_limit,
            keep_alive,
            tcp_nodelay,
            trusted_proxies,
        };
        let _ = ServerOptions::try_from(&server)?;

        Ok(Self {
            service_name,
            environment: configuration.environment().to_string(),
            profile: configuration.profile().to_string(),
            enable_info,
            readiness,
            server,
        })
    }
}

fn parse_socket_parts(value: &str) -> Option<(String, u16)> {
    value
        .parse::<SocketAddr>()
        .ok()
        .map(|address| (address.ip().to_string(), address.port()))
}

fn parse_ip_list_alias(
    configuration: &Configuration,
    keys: &[&'static str],
) -> Result<Vec<IpAddr>, ConfigError> {
    match configuration.get_any(keys) {
        None => Ok(Vec::new()),
        Some(value) => value
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(|item| {
                item.parse::<IpAddr>()
                    .map_err(|_| ConfigError::InvalidValue {
                        key: keys[0],
                        value: item.to_string(),
                        expected: "comma-separated IP addresses",
                    })
            })
            .collect(),
    }
}

fn parse_u16_alias(
    configuration: &Configuration,
    keys: &[&'static str],
    default: u16,
) -> Result<u16, ConfigError> {
    configuration
        .get_any(keys)
        .map(|value| {
            value.parse::<u16>().map_err(|_| ConfigError::InvalidValue {
                key: keys[0],
                value: value.to_string(),
                expected: "u16",
            })
        })
        .unwrap_or(Ok(default))
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
