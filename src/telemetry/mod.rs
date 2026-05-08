use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::trace::SdkTracerProvider;
use tracing_subscriber::{EnvFilter, Layer, layer::SubscriberExt, util::SubscriberInitExt};

use crate::{
    config::TelemetrySettings,
    domain::{ChainwatchError, Result},
};

pub const METRIC_LATEST_INDEXED_BLOCK: &str = "chainwatch_latest_indexed_block";
pub const METRIC_BLOCKS_INDEXED_TOTAL: &str = "chainwatch_blocks_indexed_total";
pub const METRIC_TX_INDEXED_TOTAL: &str = "chainwatch_transactions_indexed_total";
pub const METRIC_RPC_ERRORS_TOTAL: &str = "chainwatch_rpc_errors_total";
pub const METRIC_ALERTS_GENERATED_TOTAL: &str = "chainwatch_alerts_generated_total";
pub const METRIC_INDEXING_LAG: &str = "chainwatch_indexing_lag_blocks";

pub struct TelemetryGuard {
    tracer_provider: Option<SdkTracerProvider>,
}

impl Drop for TelemetryGuard {
    fn drop(&mut self) {
        if let Some(provider) = self.tracer_provider.take() {
            let _shutdown_result = provider.shutdown();
        }
    }
}

pub fn init(settings: &TelemetrySettings) -> Result<(PrometheusHandle, TelemetryGuard)> {
    let prometheus = PrometheusBuilder::new()
        .install_recorder()
        .map_err(|err| ChainwatchError::Internal(format!("metrics recorder: {err}")))?;

    let filter = EnvFilter::try_new(&settings.log_level)
        .or_else(|_| EnvFilter::try_new("info"))
        .map_err(|err| ChainwatchError::Internal(format!("log filter: {err}")))?;

    let fmt_layer = if settings.json_logs {
        tracing_subscriber::fmt::layer().json().boxed()
    } else {
        tracing_subscriber::fmt::layer().compact().boxed()
    };

    if let Some(endpoint) = &settings.otlp_endpoint {
        let exporter = opentelemetry_otlp::SpanExporter::builder()
            .with_tonic()
            .with_endpoint(endpoint)
            .build()
            .map_err(|err| ChainwatchError::Internal(format!("otlp exporter: {err}")))?;
        let provider = SdkTracerProvider::builder()
            .with_batch_exporter(exporter)
            .build();
        let tracer = provider.tracer("chainwatch-rs");
        let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);
        tracing_subscriber::registry()
            .with(filter)
            .with(fmt_layer)
            .with(otel_layer)
            .try_init()
            .map_err(|err| ChainwatchError::Internal(format!("tracing subscriber: {err}")))?;
        Ok((
            prometheus,
            TelemetryGuard {
                tracer_provider: Some(provider),
            },
        ))
    } else {
        tracing_subscriber::registry()
            .with(filter)
            .with(fmt_layer)
            .try_init()
            .map_err(|err| ChainwatchError::Internal(format!("tracing subscriber: {err}")))?;
        Ok((
            prometheus,
            TelemetryGuard {
                tracer_provider: None,
            },
        ))
    }
}
