use anyhow::Result;
use std::time::Duration;
use wtransport::ClientConfig;
use wtransport::Identity;
use wtransport::ServerConfig;
use wtransport::{
    config::QuicTransportConfig,
    quinn::{AckFrequencyConfig, VarInt},
};

pub fn configure_server(port: u16) -> Result<ServerConfig> {
    let mut config = ServerConfig::builder()
        .with_bind_default(port)
        .with_identity(Identity::self_signed(["server"])?)
        .keep_alive_interval(Some(Duration::from_secs(3)))
        .build();

    config
        .quic_config_mut()
        .transport_config(get_transport_config());
    println!("CONFIG: {:#?}", config);
    Ok(config)
}

pub fn configure_client() -> Result<ClientConfig> {
    let mut config = ClientConfig::builder()
        .with_bind_default()
        .with_no_cert_validation()
        .build();

    config
        .quic_config_mut()
        .transport_config(get_transport_config());
    Ok(config)
}

fn get_transport_config() -> std::sync::Arc<wtransport::quinn::TransportConfig> {
    let mut ack_freq_conf = AckFrequencyConfig::default();
    ack_freq_conf.max_ack_delay(Some(Duration::from_millis(1)));
    ack_freq_conf.ack_eliciting_threshold(VarInt::from_u32(0));

    let mut quic_transport_config = QuicTransportConfig::default();
    quic_transport_config.ack_frequency_config(Some(ack_freq_conf));
    quic_transport_config.max_concurrent_uni_streams(VarInt::from_u32(10000));

    quic_transport_config.send_window(4 * 1024 * 1024);
    quic_transport_config.receive_window(VarInt::from_u32(4 * 1024 * 1024));
    quic_transport_config.stream_receive_window(VarInt::from_u32(2 * 1024 * 1024));
    quic_transport_config.initial_rtt(Duration::from_millis(1));

    // quic_transport_config.initial_max_stream_data_uni(1024 * 1024); // 1MB per unidirectional stream
    // quic_transport_config.initial_max_data(10 * 1024 * 1024); // 10MB per connection

    quic_transport_config.congestion_controller_factory(std::sync::Arc::new(
        wtransport::quinn::congestion::BbrConfig::default(),
    ));

    std::sync::Arc::new(quic_transport_config)
    // std::sync::Arc::new(QuicTransportConfig::default())
}
