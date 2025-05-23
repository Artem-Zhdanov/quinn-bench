use anyhow::Result;
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use wtransport::Endpoint;

use crate::config::BLOCK_SIZE;
use crate::now_ms;
use crate::quic_config::configure_client;

pub async fn run(addr: String, port: u16) -> Result<()> {
    let config = configure_client()?;
    let url = format!("https://{}:{}", addr, port);
    let connection = Endpoint::client(config)?.connect(url).await?;

    let mut data = vec![42u8; BLOCK_SIZE];

    loop {
        let mut stream = connection.open_uni().await?.await?;
        let moment = Instant::now();

        data[0..8].copy_from_slice(&now_ms().to_be_bytes());

        match stream.write_all(&data).await {
            Ok(_) => {
                if let Err(e) = stream.flush().await {
                    tracing::error!("Error closing stream: {}", e);
                }
            }
            Err(e) => {
                tracing::error!("Error send data: {}", e);
                anyhow::bail!(e);
            }
        }
        let elapsed = moment.elapsed().as_millis() as u64;
        if elapsed < 330 {
            tokio::time::sleep(Duration::from_millis(330 - elapsed)).await;
            // tokio::task::yield_now().await;
        } else {
            tracing::error!("Elapsed time is too long: {} ms", elapsed);
        }
    }
}
