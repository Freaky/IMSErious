use anyhow::Result;
use serde::Deserialize;
use strum::{Display, EnumString};
use tokio::process::Command;

use std::{
    num::{NonZeroU16, NonZeroU32},
    str::FromStr,
    time::Duration,
};

use crate::message::ImseEvent;

#[derive(Clone, Debug, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub listen: Option<std::net::SocketAddr>,
    #[serde(default)]
    pub allow: Vec<ipnet::IpNet>,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub max_connections: Option<NonZeroU16>,
    #[serde(default)]
    pub timeout: Option<NonZeroDuration>,
    #[serde(default)]
    pub auth: Option<Auth>,
    #[serde(default)]
    pub tls: Option<TlsConfig>,
    #[serde(default)]
    pub log: Logging,
    pub handler: Vec<Handler>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Logging {
    #[serde(default)]
    pub max_level: LoggingLevel,
    #[serde(default)]
    pub level: bool,
    #[serde(default)]
    pub timestamp: bool,
    #[serde(default)]
    pub target: bool,
    #[serde(default)]
    pub ansi: bool,
    #[serde(default)]
    pub format: LoggingFormat,
}

#[derive(Copy, Clone, Debug, Display, Deserialize, Hash, PartialEq, Eq, EnumString)]
#[strum(ascii_case_insensitive)]
#[serde(try_from = "&str")]
pub enum LoggingFormat {
    Full,
    Compact,
    Pretty,
    Json,
}

impl Default for LoggingFormat {
    fn default() -> Self {
        Self::Compact
    }
}

#[derive(Copy, Clone, Debug, Deserialize, Hash, PartialEq, Eq)]
#[serde(try_from = "&str")]
pub struct LoggingLevel(tracing::Level);

impl TryFrom<&str> for LoggingLevel {
    type Error = tracing::metadata::ParseLevelError;

    fn try_from(string: &str) -> Result<Self, Self::Error> {
        Ok(Self(tracing::Level::from_str(string)?))
    }
}

impl Default for LoggingLevel {
    fn default() -> Self {
        Self(tracing::Level::INFO)
    }
}

impl LoggingLevel {
    pub fn inner(self) -> tracing::Level {
        self.0
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Auth {
    pub user: String,
    pub pass: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct TlsConfig {
    pub cert: String,
    pub key: String,
    #[serde(default)]
    pub periodic_reload: Option<NonZeroDuration>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Handler {
    pub user: String,
    #[serde(default)]
    pub event: ImseEvent,
    #[serde(default)]
    pub delay: Option<NonZeroDuration>,
    #[serde(default)]
    pub limit_period: Option<NonZeroDuration>,
    #[serde(default)]
    pub limit_burst: Option<NonZeroU32>,
    #[serde(default)]
    pub periodic: Option<NonZeroDuration>,
    pub command: SplitCommand,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(try_from = "&str")]
pub struct NonZeroDuration(Duration);

impl TryFrom<&str> for NonZeroDuration {
    type Error = &'static str;

    fn try_from(string: &str) -> Result<Self, Self::Error> {
        let d = humantime::parse_duration(string).map_err(|_| "Error parsing Duration")?;
        if d.is_zero() {
            Err("Duration is zero")
        } else {
            Ok(Self(d))
        }
    }
}

impl From<NonZeroDuration> for Duration {
    fn from(dur: NonZeroDuration) -> Duration {
        dur.0
    }
}

impl NonZeroDuration {
    pub fn into_std(self) -> Duration {
        self.0
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(try_from = "&str")]
pub struct SplitCommand(Vec<String>);

impl TryFrom<&str> for SplitCommand {
    type Error = &'static str;

    fn try_from(string: &str) -> Result<Self, Self::Error> {
        let command = shell_words::split(string).map_err(|_| "missing closing quote")?;
        if command.is_empty() {
            return Err("command is empty");
        }
        Ok(Self(command))
    }
}

impl SplitCommand {
    pub fn as_tokio_command(&self) -> Command {
        let mut command = Command::new(&self.0[0]);
        if self.0.len() > 1 {
            command.args(&self.0[1..]);
        }
        command
    }

    pub fn get_prog(&self) -> &str {
        &self.0[0]
    }
}

impl Config {
    pub fn from_path<P>(path: P) -> Result<Config>
    where
        P: AsRef<std::path::Path>,
    {
        Ok(toml::from_str(&read_restrict::read_to_string(
            path,
            1024 * 1024,
        )?)?)
    }
}
