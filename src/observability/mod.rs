use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Registry};
use opentelemetry::{global, KeyValue, trace::TracerProvider as _};
use opentelemetry_sdk::{Resource, trace::TracerProvider as SdkTracerProvider};
use opentelemetry_otlp::WithExportConfig;

pub fn init_tracing(service_name: &str) {
    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,actix_web=info"));

    // Check if OTLP endpoint is set, otherwise default to localhost
    let otlp_endpoint = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
        .unwrap_or_else(|_| "http://localhost:4317".to_string());

    // Create OTLP exporter using SpanExporter::builder (v0.27+)
    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(&otlp_endpoint)
        .build()
        .expect("Failed to create OTLP exporter");

    // Configure Tracer Provider
    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter, opentelemetry_sdk::runtime::Tokio)
        .with_resource(Resource::new(vec![
            KeyValue::new("service.name", service_name.to_string()),
        ]))
        .build();

    // Set global provider
    global::set_tracer_provider(provider.clone());
    
    // Set global propagator for trace context propagation
    global::set_text_map_propagator(opentelemetry_sdk::propagation::TraceContextPropagator::new());

    // Get tracer
    let tracer = provider.tracer("tracing-otel-subscriber");

    // Create a tracing layer with the configured tracer
    let telemetry_layer = tracing_opentelemetry::layer().with_tracer(tracer);

    // Initialize the subscriber with both stdout formatting and OTLP export
    let _ = Registry::default()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer())
        .with(telemetry_layer)
        .try_init();

    tracing::info!("🔍 Distributed tracing initialized for service: {} -> {}", service_name, otlp_endpoint);
}

pub fn shutdown_tracing() {
    global::shutdown_tracer_provider();
}
