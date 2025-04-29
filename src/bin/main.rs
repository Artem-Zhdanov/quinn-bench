use anyhow::Result;
use clap::{arg, Parser};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::time::Duration;
use std::time::Instant;
use std::{
    net::{SocketAddr, ToSocketAddrs},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};
use tokio::time::sleep;
use tracing::info;
use wtransport::ClientConfig;
use wtransport::Endpoint;
use wtransport::Identity;
use wtransport::ServerConfig;
use wtransport::{
    config::QuicTransportConfig,
    quinn::{AckFrequencyConfig, VarInt},
};

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
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_ansi(false)
        .init();

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

pub fn get_transport_config() -> std::sync::Arc<wtransport::quinn::TransportConfig> {
    let mut ack_freq_conf = AckFrequencyConfig::default();
    ack_freq_conf.max_ack_delay(Some(Duration::from_millis(1)));
    ack_freq_conf.ack_eliciting_threshold(VarInt::from_u32(0));

    let mut quic_transport_config = QuicTransportConfig::default();
    quic_transport_config.ack_frequency_config(Some(ack_freq_conf));
    quic_transport_config.max_concurrent_uni_streams(VarInt::from_u32(10000));

    quic_transport_config.send_window(4 * 1024 * 1024);
    quic_transport_config.receive_window(VarInt::from_u32(4 * 1024 * 1024));
    quic_transport_config.stream_receive_window(VarInt::from_u32(2 * 1024 * 1024));

    // quic_transport_config.congestion_controller_factory(std::sync::Arc::new(
    //     wtransport::quinn::congestion::BbrConfig::default(),
    // ));

    let trans_conf = std::sync::Arc::new(quic_transport_config);
    // let trans_conf = std::sync::Arc::new(QuicTransportConfig::default());

    trans_conf
}

fn configure_server(port: u16) -> Result<ServerConfig> {
    let mut config = ServerConfig::builder()
        .with_bind_default(port)
        .with_identity(Identity::self_signed(["server"]).unwrap())
        .keep_alive_interval(Some(Duration::from_secs(3)))
        .build();

    config
        .quic_config_mut()
        .transport_config(get_transport_config());
    println!("CONFIG: {:#?}", config);
    Ok(config)
}

fn configure_client() -> Result<ClientConfig> {
    let mut config = ClientConfig::builder()
        .with_bind_default()
        .with_no_cert_validation()
        .build();

    config
        .quic_config_mut()
        .transport_config(get_transport_config());
    Ok(config)
}

async fn run_subscriber(metrics: Arc<Metrics>, server_addr: SocketAddr) -> Result<()> {
    let server_config = configure_server(server_addr.port())?;
    let server = Endpoint::server(server_config)?;

    let incoming_session = server.accept().await;

    let session_request = incoming_session.await?;

    info!(
        "New session: Authority: '{}', Path: '{}'",
        session_request.authority(),
        session_request.path()
    );

    let connection = session_request.accept().await?;

    let metrics_clone = metrics.clone();

    while let Ok(mut stream) = connection.accept_uni().await {
        let metrics = metrics_clone.clone();

        let mut buf = vec![0u8; BLOCK_SIZE];
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
    let config = configure_client()?;
    let url = format!("https://{}", server_addr.to_string());
    //let url = format!("https://[::1]:{}", server_addr.port());
    let connection = Endpoint::client(config)
        .unwrap()
        .connect(url)
        .await
        .unwrap();

    let mut data = vec![42u8; BLOCK_SIZE];
    let mut stream = connection.open_uni().await.unwrap().await.unwrap();

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
