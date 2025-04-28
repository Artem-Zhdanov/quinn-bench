use anyhow::Result;
use quinn::{ClientConfig, Endpoint, ServerConfig, VarInt};
use rustls::{Certificate, PrivateKey};
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::time::{sleep, Instant};

const SERVER_ADDR: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 5000);
const BLOCK_SIZE: usize = 200 * 1024; // 200KB

#[derive(Debug, Default)]
struct Metrics {
    bytes: AtomicU64,
    blocks: AtomicU64,
}

impl Metrics {
    fn new() -> Self {
        Self::default()
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let metrics: Arc<Metrics> = Arc::new(Metrics::new());

    let metrics_clone = metrics.clone();
    let _server_handle = tokio::spawn(async move {
        if let Err(e) = run_server(metrics_clone).await {
            eprintln!("Server task failed: {}", e);
        }
    });

    sleep(Duration::from_secs(1)).await;

    let _client_handle = tokio::spawn(async {
        if let Err(e) = run_client().await {
            eprintln!("Client task failed: {}", e);
        }
    });

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
        //     .max_idle_timeout(Some(quinn::IdleTimeout::from(Duration::from_secs(30))))
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

async fn run_server(metrics: Arc<Metrics>) -> Result<()> {
    let server_config = configure_server()?;
    let endpoint = Endpoint::server(server_config, SERVER_ADDR)?;

    loop {
        let accept = endpoint.accept().await;

        let connecting = match accept {
            Some(connecting) => connecting,
            None => {
                println!("endpoint closed");
                continue;
            }
        };

        let connection = match connecting.await {
            Ok(connection) => {
                println!("new conection {:?}", connection.remote_address());
                connection
            }
            Err(e) => {
                eprintln!("Error: {}", e);
                continue;
            }
        };

        let metrics_clone = metrics.clone();

        tokio::spawn(async move {
            while let Ok(mut stream) = connection.accept_uni().await {
                let metrics = metrics_clone.clone();

                tokio::spawn(async move {
                    let mut buf = vec![0u8; BLOCK_SIZE * 2]; // twice as expected data
                    loop {
                        let result = stream.read(&mut buf).await;
                        match result {
                            Ok(Some(0)) => {
                                tokio::task::yield_now().await;
                            }
                            Ok(Some(n)) => {
                                metrics.bytes.fetch_add(n as u64, Ordering::Relaxed);
                                metrics.blocks.fetch_add(1, Ordering::Relaxed);
                            }
                            Ok(None) => {
                                break;
                            }
                            Err(e) => {
                                eprintln!("Error reading: {}", e);
                                break;
                            }
                        }
                    }
                });
            } // While loop for stream
        });
    }
}

async fn run_client() -> Result<()> {
    let client_config = configure_client()?;

    // Создаем эндпоинт для клиента с рандомным локальным адресом
    let mut endpoint = Endpoint::client(SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), 0))?;
    endpoint.set_default_client_config(client_config);

    let connection = endpoint.connect(SERVER_ADDR, "localhost")?.await?;

    let data = vec![0u8; BLOCK_SIZE];

    loop {
        let mut stream = connection.open_uni().await?;

        match stream.write_all(&data).await {
            Ok(_) => {
                if let Err(e) = stream.finish().await {
                    eprintln!("Error closing stream: {}", e);
                }
            }
            Err(e) => {
                eprintln!("Error send data: {}", e);
                anyhow::bail!(e);
            }
        }
        tokio::task::yield_now().await;
    }
}

// Запуск клиента
async fn run_report(metrics: Arc<Metrics>) -> Result<()> {
    let moment = Instant::now();
    let mut prev_bytes = 0;
    let mut prev_micros = 0;
    loop {
        sleep(Duration::from_secs(1)).await;

        let bytes = metrics.bytes.load(Ordering::Relaxed);

        let _blocks = metrics.blocks.load(Ordering::Relaxed);

        let micros = moment.elapsed().as_micros();
        println!("micros: {}", micros);

        let speed = ((bytes * 1_000_000) / micros as u64) as f64;
        let delta_bytes = bytes - prev_bytes;
        let delta_micros = micros - prev_micros;
        let moment_speed = ((delta_bytes * 1_000_000) / delta_micros as u64) as f64;
        println!(
            "Speed: {:.2} GB, at moment {:.2} GB",
            speed / (1024.0 * 1024.0 * 1024.0),
            moment_speed / (1024.0 * 1024.0 * 1024.0)
        );

        prev_bytes = bytes;
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

//  Что в нем можно настроить чтобы скорость передачи возросла? Сейчас она равна 0,25 GB
