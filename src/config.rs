use std::{
    env::{self, VarError},
    path::{Path, PathBuf},
};

use indexmap::IndexMap;
use serde::{de::Error, Deserialize, Deserializer};
use tracing_subscriber::filter::FilterId;

pub const DEFAULT_LOG_LEVEL: &str = "info";
pub const DEFAULT_LOG_FILENAME: &str = "app.log";

#[derive(Debug, Copy, Clone, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    Full,
    Pretty,
    Compact,
    System,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConsoleTarget {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize)]
#[serde(default)]
pub struct ConsoleLogConfig {
    pub color: bool,
    pub level: Option<String>,
    pub format: Option<LogFormat>,
    pub target: ConsoleTarget,
}

impl Default for ConsoleLogConfig {
    fn default() -> Self {
        Self {
            color: true,
            level: None,
            format: None,
            target: ConsoleTarget::Stdout,
        }
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileWritingMode {
    Append,
    Overwrite,
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize)]
#[serde(default)]
pub struct FileLogConfig {
    pub color: bool,
    pub level: Option<String>,
    pub format: Option<LogFormat>,
    pub path: PathBuf,
    pub mode: FileWritingMode,
}

impl Default for FileLogConfig {
    fn default() -> Self {
        Self {
            color: false,
            level: None,
            format: None,
            path: DEFAULT_LOG_FILENAME.to_owned().into(),
            mode: FileWritingMode::Append,
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AppenderLogConfig {
    Console(ConsoleLogConfig),
    File(FileLogConfig),
}

#[derive(Debug, Default, Clone, Eq, PartialEq, Deserialize)]
#[serde(default)]
pub struct LogConfigs {
    pub appenders: IndexMap<String, AppenderLogConfig>,
}

fn deserialize_log_configs<'de, D>(deserializer: D) -> Result<LogConfigs, D::Error>
where
    D: Deserializer<'de>,
{
    let mut log_configs = LogConfigs::deserialize(deserializer)?;

    if log_configs.appenders.is_empty() {
        log_configs.appenders.insert(
            "stdout".into(),
            AppenderLogConfig::Console(ConsoleLogConfig::default()),
        );
    }

    // `tracing` limits the number of simultaneous filters
    let max_appenders = FilterId::MAX_ID;
    if log_configs.appenders.len() > max_appenders as usize {
        let msg = format!("cannot have more than {max_appenders} appenders");
        return Err(D::Error::custom(msg));
    }

    Ok(log_configs)
}

/// Global log configuration
#[derive(Debug, Clone, Eq, PartialEq, Deserialize)]
#[serde(default)]
pub struct GlobalLogConfig {
    #[serde(skip)]
    pub level_from_env: Option<String>,
    pub level: String,
    pub format: LogFormat,
}

impl Default for GlobalLogConfig {
    fn default() -> Self {
        Self {
            level_from_env: None,
            level: DEFAULT_LOG_LEVEL.to_owned(),
            format: LogFormat::Full,
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Deserialize)]
pub struct Log {
    #[serde(flatten)]
    pub global: GlobalLogConfig,
    #[serde(flatten, deserialize_with = "deserialize_log_configs")]
    pub configs: LogConfigs,
}

impl Log {
    pub fn parse(file_contents: &str, data_dir: &Path) -> eyre::Result<Self> {
        let level_from_env = match env::var("RUST_LOG") {
            Ok(level) => Some(level),
            Err(VarError::NotPresent) => None,
            Err(err) => return Err(err.into()),
        };

        #[derive(Deserialize)]
        struct LogSection {
            log: Log,
        }

        let mut log = toml::from_str::<LogSection>(file_contents)?.log;
        log.global.level_from_env = level_from_env;

        for appender in log.configs.appenders.values_mut() {
            let path = match appender {
                AppenderLogConfig::Console(_) => continue,
                AppenderLogConfig::File(file) => &mut file.path,
            };
            *path = data_dir.join(&path);
        }

        Ok(log)
    }
}

/// Common methods for a log configuration
pub trait LogConfig {
    fn color(&self) -> bool;
    fn level(&self) -> Option<&str>;
    fn format(&self) -> Option<LogFormat>;
}

macro_rules! impl_log_config {
    ($struct_name:ident) => {
        impl LogConfig for $struct_name {
            fn color(&self) -> bool {
                self.color
            }
            fn level(&self) -> Option<&str> {
                self.level.as_deref()
            }
            fn format(&self) -> Option<LogFormat> {
                self.format
            }
        }
    };
}

impl_log_config!(ConsoleLogConfig);
impl_log_config!(FileLogConfig);
