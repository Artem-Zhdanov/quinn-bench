use anyhow::Result;

use std::sync::atomic::Ordering;
use std::sync::Arc;
use wtransport::Endpoint;

use crate::config::BLOCK_SIZE;
use crate::metrics::{Metrics, OtMetrics};
use crate::now_ms;
use crate::quic_config::configure_server;

pub async fn run(metrics: Arc<Metrics>, ot_metrics: Arc<OtMetrics>, port: u16) -> Result<()> {
    let server_config = configure_server(port)?;
    let server = Endpoint::server(server_config)?;

    let incoming_session = server.accept().await;

    let session_request = incoming_session.await?;

    tracing::info!(
        "New session: Authority: '{}', Path: '{}'",
        session_request.authority(),
        session_request.path()
    );

    let connection = session_request.accept().await?;

    let metrics_clone = metrics.clone();

    while let Ok(mut stream) = connection.accept_uni().await {
        let metrics = metrics_clone.clone();

        let mut buf = vec![0u8; BLOCK_SIZE];
        // loop {
        match stream.read_exact(&mut buf).await {
            Ok(_) => {
                metrics.blocks.fetch_add(1, Ordering::Relaxed);

                let header_bytes = &buf[0..8];

                let sent_timestamp = u64::from_be_bytes(header_bytes.try_into()?);

                let latency = now_ms() - sent_timestamp;
                println!("Latency ms: {}", latency);
                ot_metrics.latency.record(latency, &[]);
            }
            Err(e) => {
                eprintln!("Error reading: {}", e);
                // break;
            }
        }
        //  }
    }
    Ok(())
}
