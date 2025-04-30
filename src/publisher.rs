use anyhow::Result;
use std::time::Duration;
use std::time::Instant;
use wtransport::Endpoint;

use crate::config::BLOCK_SIZE;
use crate::now_ms;
use crate::quic_config::configure_client;

pub async fn run(addr: String, port: u16) -> Result<()> {
    for _ in 0..2 {
        let config = configure_client()?;
        let url = format!("https://{}:{}", addr, port);
        let connection = Endpoint::client(config)?.connect(url).await?;

        tokio::spawn(async move {
            let mut data = vec![42u8; BLOCK_SIZE];

            loop {
                let mut stream = connection.open_uni().await.unwrap().await.unwrap();

                data[0..8].copy_from_slice(&now_ms().to_be_bytes());

                match stream.write_all(&data).await {
                    Ok(_) => {
                        // if let Err(e) = stream.finish().await {
                        //     eprintln!("Error closing stream: {}", e);
                        // }
                    }
                    Err(e) => {
                        eprintln!("Error send data: {}", e);
                        //  anyhow::bail!(e);
                    }
                }
            }
        });
    }
    Ok(())
}
