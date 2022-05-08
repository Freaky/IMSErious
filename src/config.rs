use anyhow::Result;
use serde::Deserialize;
use tokio::process::Command;

use crate::handler::Handler;

#[derive(Deserialize, Debug, Clone)]
pub struct Config {
    #[serde(default)]
    pub listen: Option<std::net::SocketAddr>,
    #[serde(default)]
    pub allow: Vec<ipnet::IpNet>,
    #[serde(default)]
    pub auth: Option<Auth>,
    pub handler: Vec<Handler>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Auth {
    pub user: String,
    pub pass: String,
}

#[derive(Debug, Clone, Deserialize)]
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
}

impl Config {
    pub fn from_path<P>(path: P) -> Result<Config>
    where
        P: AsRef<std::path::Path>,
    {
        tracing::debug!("loading config from {}", path.as_ref().display());
        Ok(toml::from_str(&read_restrict::read_to_string(
            path,
            1024 * 1024,
        )?)?)
    }
}
