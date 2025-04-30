use opentelemetry::metrics::Gauge;
use opentelemetry::metrics::Histogram;
use opentelemetry::KeyValue;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;
use std::time::Duration;

use opentelemetry_sdk::metrics::PeriodicReader;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::runtime::Tokio;

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
    std::env::set_var(
        "OTEL_RESOURCE_ATTRIBUTES",
        "service.namespace=abc,service.instance.id=my_instance_0",
    );

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
        latency: meter.u64_histogram("quic_latency").build(),
    });
    ot_metrics
}
