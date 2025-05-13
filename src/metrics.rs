use opentelemetry::metrics::Histogram;
use opentelemetry_sdk::metrics::PeriodicReader;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::runtime::Tokio;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Default)]
pub struct Metrics {
    pub bytes: AtomicUsize,
    pub blocks: AtomicUsize,
}

impl Metrics {
    pub fn new() -> Self {
        Self::default()
    }
}

pub struct OtMetrics {
    pub latency: Histogram<u64>,
}

pub fn init_metrics() -> Arc<OtMetrics> {
    // Don't forget to set and export `OTEL_EXPORTER_OTLP_METRICS_ENDPOINT` envar.
    std::env::set_var("OTEL_EXPORTER_OTLP_METRICS_PROTOCOL", "grpc");
    std::env::set_var("OTEL_EXPORTER_OTLP_INSECURE", "true");
    std::env::set_var("OTEL_SERVICE_NAME", "quic-test");

    let resource = opentelemetry_sdk::Resource::default();

    let metric_exporter = opentelemetry_otlp::MetricExporter::builder()
        .with_tonic()
        .build()
        .expect("Failed to build OTLP metrics exporter");

    let meter_provider = SdkMeterProvider::builder()
        .with_reader(
            PeriodicReader::builder(metric_exporter, Tokio)
                .with_interval(Duration::from_secs(30))
                .with_timeout(Duration::from_secs(5))
                .build(),
        )
        .with_resource(resource)
        .build();

    opentelemetry::global::set_meter_provider(meter_provider.clone());
    let meter = &opentelemetry::global::meter("quic");

    let ot_metrics = Arc::new(OtMetrics {
        latency: meter
            .u64_histogram("quic_latency")
            .with_boundaries(vec![
                0.0, 5.0, 10.0, 20.0, 40.0, 50.0, 60.0, 70.0, 80.0, 90.0, 100.0, 110.0, 120.0,
                130.0, 140.0, 150.0, 160.0, 170.0, 180.0, 190.0, 200.0, 210.0, 220.0, 230.0, 240.0,
                250.0, 280.0, 300.0, 1000.0, 5000.0, 10000.0,
            ])
            .build(),
    });
    ot_metrics
}
