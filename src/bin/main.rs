use anyhow::Result;
use clap::Parser;
use quic_rust_test::{
    config::{read_yaml, ActiveSubscribers, CliArgs, Config, Subscriber},
    metrics::{init_metrics, Metrics},
    ports_string_to_vec, publisher, subscriber,
};
use std::{sync::atomic::Ordering, time::Instant};
use std::{sync::Arc, time::Duration};
use tokio::time::sleep;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_ansi(false)
        .init();

    let ot_metrics = init_metrics();
    let cli: CliArgs = CliArgs::parse();
    let config = match read_yaml::<Config>(&cli.config) {
        Ok(config) => config,
        Err(error) => {
            tracing::error!("Error parsing config file {:?}: {:?}", cli.config, error);
            std::process::exit(1);
        }
    };

    let metrics: Arc<Metrics> = Arc::new(Metrics::new());

    // Run subscribers
    for Subscriber { ports } in config.subscriber {
        for port in ports_string_to_vec(&ports)? {
            let metrics_clone = metrics.clone();
            let ot_metrics_clone = ot_metrics.clone();
            let _ = tokio::spawn(async move {
                if let Err(err) = subscriber::run(metrics_clone, ot_metrics_clone, port).await {
                    tracing::error!("Subscriber error: {}", err);
                }
            });
        }
    }

    sleep(Duration::from_secs(1)).await;

    // Run publisher
    for ActiveSubscribers { addr, ports } in config.publisher {
        let ports = ports_string_to_vec(&ports)?;
        let delta = 330000 / ports.len() as u64;

        for (i, port) in ports.into_iter().enumerate() {
            let addr_clone = addr.clone();
            let _ = tokio::spawn(async move {
                tokio::time::sleep(Duration::from_micros(i as u64 * delta)).await;
                if let Err(err) = publisher::run(addr_clone, port).await {
                    tracing::error!("Publisher task failed: {}", err);
                }
            });
        }
    }

    tokio::signal::ctrl_c().await?;
    Ok(())
}
