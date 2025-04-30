use anyhow::Result;
use clap::{arg, Parser};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    net::{SocketAddr, ToSocketAddrs},
    path::{Path, PathBuf},
};

pub const BLOCK_SIZE: usize = 300 * 1024;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    pub subscribers: Vec<Subscriber>,
    pub publisher: Vec<ActiveSubscribers>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Subscriber {
    #[serde(deserialize_with = "deserialize_addr")]
    pub addr: SocketAddr,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ActiveSubscribers {
    #[serde(deserialize_with = "deserialize_addr")]
    pub addr: SocketAddr,
}

#[derive(Parser, Debug, Clone, Serialize)]
pub struct CliArgs {
    #[arg(short, long)]
    pub config: PathBuf,
}

fn deserialize_addr<'de, D>(deserializer: D) -> Result<SocketAddr, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let addr_str: String = String::deserialize(deserializer)?;
    let result = addr_str
        .parse::<SocketAddr>()
        .map_err(serde::de::Error::custom);
    if result.is_err() {
        // try to resolve addr_str as a SockerAddr
        if let Ok(mut sockets) = addr_str.to_socket_addrs() {
            if let Some(socket) = sockets.next() {
                return Ok(socket);
            }
        }
    }
    result
}

pub fn read_yaml<T: DeserializeOwned>(config_path: impl AsRef<Path>) -> anyhow::Result<T> {
    let config_path = config_path.as_ref();
    let Some(path) = config_path.as_os_str().to_str() else {
        anyhow::bail!("Invalid path {:?}", config_path);
    };
    let expanded = PathBuf::from(shellexpand::tilde(path).into_owned());
    let file = std::fs::File::open(&expanded)?;
    let config = serde_yaml::from_reader(file)?;
    Ok(config)
}
