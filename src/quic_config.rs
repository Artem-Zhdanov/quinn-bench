use anyhow::Result;
use std::time::Duration;
use wtransport::quinn::MtuDiscoveryConfig;
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
    tracing::info!("CONFIG: {:#?}", config);
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
    ack_freq_conf.ack_eliciting_threshold(VarInt::from_u32(1)); // Быстрая генерация ACK

    let mut quic_transport_config = QuicTransportConfig::default();

    quic_transport_config.ack_frequency_config(Some(ack_freq_conf));

    // 🔥 Максимально широкие окна
    quic_transport_config.send_window(64 * 1024 * 1024); // 64MB
    quic_transport_config.receive_window(VarInt::from_u32(64 * 1024 * 1024));
    quic_transport_config.stream_receive_window(VarInt::from_u32(16 * 1024 * 1024));

    // 🔄 Отключение справедливости для более агрессивной отправки
    quic_transport_config.send_fairness(false);

    // 💡 Повышение лимита количества одновременных uni-потоков
    quic_transport_config.max_concurrent_uni_streams(VarInt::from_u32(1000));

    // 🧠 Быстрая и агрессивная congestion control
    let mut bbr = wtransport::quinn::congestion::BbrConfig::default();
    bbr.initial_window(48_000); // 48KB стартовое окно

    quic_transport_config.congestion_controller_factory(std::sync::Arc::new(bbr));

    // 🚀 Настройка MTU discovery
    let mut mtu_disc_conf = MtuDiscoveryConfig::default();
    mtu_disc_conf.interval(Duration::from_secs(5)); // Проверяем почаще
    mtu_disc_conf.upper_bound(1400); // Стабильный MTU, избегаем IP-фрагментации

    quic_transport_config.mtu_discovery_config(Some(mtu_disc_conf));
    quic_transport_config.initial_mtu(1400);
    quic_transport_config.min_mtu(1200); // Базовый минимум

    std::sync::Arc::new(quic_transport_config)
}
