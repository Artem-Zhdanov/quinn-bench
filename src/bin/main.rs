use anyhow::{bail, Result};
use clap::{arg, Parser};
use quinn::{ClientConfig, Endpoint, ServerConfig, VarInt};
use rustls::{Certificate, PrivateKey};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr, ToSocketAddrs},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::time::{sleep, Instant};

const BLOCK_SIZE: usize = 200 * 1024; // 200KB

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    pub subscribers: Vec<Subscriber>,
    pub publisher: Vec<ActiveSubscribers>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Subscriber {
    #[serde(deserialize_with = "deserialize_addr", default = "default_addr")]
    pub addr: SocketAddr,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ActiveSubscribers {
    #[serde(deserialize_with = "deserialize_addr", default = "default_addr")]
    pub subscriber_addr: SocketAddr,
}

#[derive(Parser, Debug, Clone, Serialize)]
pub struct CliArgs {
    #[arg(short, long)]
    pub config: PathBuf,
}

#[derive(Debug, Default)]
struct Metrics {
    bytes: AtomicUsize,
    blocks: AtomicUsize,
}

impl Metrics {
    fn new() -> Self {
        Self::default()
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli: CliArgs = CliArgs::parse();
    let config = match read_yaml::<Config>(&cli.config) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("Error parsing config file {:?}: {:?}", cli.config, error);
            std::process::exit(1);
        }
    };

    let metrics: Arc<Metrics> = Arc::new(Metrics::new());

    // Run subscribers
    for Subscriber { addr } in config.subscribers {
        let metrics_clone = metrics.clone();
        let _ = tokio::spawn(async move {
            if let Err(err) = run_subscriber(metrics_clone, addr).await {
                eprintln!("Subscriber error: {}", err);
            }
        });
    }

    sleep(Duration::from_secs(1)).await;

    // Run publisher
    for ActiveSubscribers { subscriber_addr } in config.publisher {
        let _ = tokio::spawn(async move {
            if let Err(err) = publish(subscriber_addr).await {
                eprintln!("Publisher task failed: {}", err);
            }
        });
    }

    let metrics_clone = metrics.clone();
    let _report_handle = tokio::spawn(async {
        if let Err(e) = run_report(metrics_clone).await {
            eprintln!("Report task failed: {}", e);
        }
    });

    tokio::signal::ctrl_c().await?;
    Ok(())
}

fn configure_server() -> Result<ServerConfig> {
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let cert_der = cert.serialize_der().unwrap();
    let priv_key = cert.serialize_private_key_der();
    let priv_key = PrivateKey(priv_key);
    let cert_chain = vec![Certificate(cert_der)];

    let mut server_config = ServerConfig::with_crypto(Arc::new(
        rustls::ServerConfig::builder()
            .with_safe_defaults()
            .with_no_client_auth()
            .with_single_cert(cert_chain, priv_key)?,
    ));

    server_config.transport_config(configure_transport());
    Ok(server_config)
}

fn configure_client() -> Result<ClientConfig> {
    let mut client_config = ClientConfig::new(Arc::new(
        rustls::ClientConfig::builder()
            .with_safe_defaults()
            .with_custom_certificate_verifier(Arc::new(SkipServerVerification))
            .with_no_client_auth(),
    ));

    client_config.transport_config(configure_transport());
    Ok(client_config)
}

fn configure_transport() -> Arc<quinn::TransportConfig> {
    let mut transport_config = quinn::TransportConfig::default();

    transport_config
        .max_concurrent_uni_streams(VarInt::from_u32(1024))
        // .max_idle_timeout(Some(quinn::IdleTimeout::from(Duration::from_secs(30))))
        .keep_alive_interval(Some(Duration::from_secs(5)))
        .stream_receive_window(VarInt::from_u32(10 * 1024 * 1024)) // 10MB
        .receive_window(VarInt::from_u32(10 * 1024 * 1024)) // 10MB
        .send_window(10 * 1024 * 1024) // 10MB
        .max_concurrent_uni_streams(VarInt::from_u32(4096))
        .stream_receive_window(VarInt::from_u32(50 * 1024 * 1024)) // 50MB
        .receive_window(VarInt::from_u32(50 * 1024 * 1024)) // 50MB
        .send_window(50 * 1024 * 1024) // 50MB
        .allow_spin(true);

    Arc::new(transport_config)
}

async fn run_subscriber(metrics: Arc<Metrics>, server_addr: SocketAddr) -> Result<()> {
    let server_config = configure_server()?;
    let endpoint = Endpoint::server(server_config, server_addr)?;

    let accept = endpoint.accept().await;

    let connecting = match accept {
        Some(connecting) => connecting,
        None => {
            println!("endpoint closed");
            anyhow::bail!("endpoint closed");
        }
    };

    let connection = match connecting.await {
        Ok(connection) => {
            println!("new conection {:?}", connection.remote_address());
            connection
        }
        Err(e) => {
            bail!("Error: {}", e);
        }
    };

    let metrics_clone = metrics.clone();

    while let Ok(mut stream) = connection.accept_uni().await {
        let metrics = metrics_clone.clone();

        let mut buf = vec![0u8; BLOCK_SIZE]; // twice as expected data
        let mut pos = 0; // how much valid data we have
        loop {
            match stream.read_exact(&mut buf).await {
                Ok(_) => {
                    metrics.blocks.fetch_add(1, Ordering::Relaxed);

                    let header_bytes = &buf[0..8];

                    let sent_timestamp = u64::from_be_bytes(header_bytes.try_into().unwrap());

                    println!("Delay ms: {}", now_ms() - sent_timestamp);
                }
                Err(e) => {
                    eprintln!("Error reading: {}", e);
                    break;
                }
            }
        }
    }
    Ok(())
}
async fn publish(server_addr: SocketAddr) -> Result<()> {
    let client_config = configure_client()?;

    let mut endpoint = Endpoint::client(SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), 0))?;
    endpoint.set_default_client_config(client_config);

    let connection = endpoint.connect(server_addr, "localhost")?.await?;

    let mut data = vec![42u8; BLOCK_SIZE];
    let mut stream: quinn::SendStream = connection.open_uni().await?;

    loop {
        let moment = Instant::now();

        data[0..8].copy_from_slice(&now_ms().to_be_bytes());

        match stream.write_all(&data).await {
            Ok(_) => {
                // if let Err(e) = stream.finish().await {
                //     eprintln!("Error closing stream: {}", e);
                // }
            }
            Err(e) => {
                eprintln!("Error send data: {}", e);
                anyhow::bail!(e);
            }
        }
        // tokio::task::yield_now().await;
        let elapsed = moment.elapsed().as_millis() as u64;
        if elapsed < 300 {
            tokio::time::sleep(Duration::from_millis(300 - elapsed)).await;
        } else {
            eprintln!("Elapsed time is too long: {} ms", elapsed);
        }
    }
}

async fn run_report(metrics: Arc<Metrics>) -> Result<()> {
    let moment = Instant::now();
    let mut prev_bytes = 0;
    let mut prev_blocks = 0;
    let mut prev_micros = 0;
    loop {
        sleep(Duration::from_secs(1)).await;

        let bytes = metrics.bytes.load(Ordering::Relaxed);
        let blocks = metrics.blocks.load(Ordering::Relaxed);
        let micros = moment.elapsed().as_micros();

        let delta_bytes = bytes - prev_bytes;
        let delta_blocks = blocks - prev_blocks;

        let delta_micros = micros - prev_micros;
        let moment_speed_bytes = ((delta_bytes * 1_000_000) / delta_micros as usize) as f64;
        let moment_speed_blocks = ((delta_blocks * 1_000_000) / delta_micros as usize) as f64;

        println!(
            "Speed {:.2} GB, ({}) blocks",
            moment_speed_bytes / (1024.0 * 1024.0 * 1024.0),
            moment_speed_blocks
        );

        println!(
            "Totals: bytes {}, blocks: {}\n-------------------------------",
            bytes, blocks
        );

        prev_bytes = bytes;
        prev_blocks = blocks;
        prev_micros = micros;
    }
}

struct SkipServerVerification;

impl rustls::client::ServerCertVerifier for SkipServerVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &Certificate,
        _intermediates: &[Certificate],
        _server_name: &rustls::ServerName,
        _scts: &mut dyn Iterator<Item = &[u8]>,
        _ocsp_response: &[u8],
        _now: std::time::SystemTime,
    ) -> Result<rustls::client::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::ServerCertVerified::assertion())
    }
}

fn default_addr() -> SocketAddr {
    "0.0.0.0:5000".parse().expect("Invalid default address")
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

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
