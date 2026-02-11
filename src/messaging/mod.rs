//! NATS Messaging Client for Lanai Services
//!
//! Provides a singleton NATS client with:
//! - Automatic reconnection with backoff
//! - Connection status monitoring
//! - Typed event publishing
//! - Optional JetStream support for durable messaging

use async_nats::{Client, ConnectOptions};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::OnceCell;
use log::{info, warn, error};
use tracing_opentelemetry::OpenTelemetrySpanExt;
use opentelemetry::propagation::Injector;

pub mod events;

/// Environment variable for NATS URL
pub const NATS_URL_ENV: &str = "NATS_URL";
/// Default NATS URL
pub const DEFAULT_NATS_URL: &str = "nats://localhost:4222";

/// Singleton-like NATS client for Lanai services
#[derive(Clone)]
pub struct NatsClient;

static NATS_INSTANCE: OnceCell<Arc<Client>> = OnceCell::const_new();

/// Configuration for NATS connection
#[derive(Debug, Clone)]
pub struct NatsConfig {
    /// NATS server URL(s), comma-separated for clusters
    pub url: String,
    /// Maximum reconnection attempts (0 = infinite)
    pub max_reconnects: usize,
    /// Initial reconnection delay
    pub reconnect_delay: Duration,
    /// Maximum reconnection delay (for exponential backoff)
    pub max_reconnect_delay: Duration,
    /// Connection name for identification
    pub connection_name: String,
}

impl Default for NatsConfig {
    fn default() -> Self {
        Self {
            url: std::env::var(NATS_URL_ENV).unwrap_or_else(|_| DEFAULT_NATS_URL.to_string()),
            max_reconnects: 0, // Infinite
            reconnect_delay: Duration::from_millis(500),
            max_reconnect_delay: Duration::from_secs(30),
            connection_name: "lanai-service".to_string(),
        }
    }
}

impl NatsConfig {
    /// Create a new config with a specific service name
    pub fn for_service(service_name: &str) -> Self {
        Self {
            connection_name: service_name.to_string(),
            ..Default::default()
        }
    }
}

impl NatsClient {
    /// Initialize the global NATS connection with default config
    pub async fn init(url: &str) -> Result<(), async_nats::ConnectError> {
        let config = NatsConfig {
            url: url.to_string(),
            ..Default::default()
        };
        Self::init_with_config(config).await
    }

    /// Initialize the global NATS connection with custom config
    pub async fn init_with_config(config: NatsConfig) -> Result<(), async_nats::ConnectError> {
        let connect_options = ConnectOptions::new()
            .name(&config.connection_name)
            .retry_on_initial_connect()

            .reconnect_delay_callback(move |attempts| {
                // Exponential backoff with jitter
                let base_delay = config.reconnect_delay.as_millis() as u64;
                let max_delay = config.max_reconnect_delay.as_millis() as u64;
                let delay = std::cmp::min(base_delay * 2u64.saturating_pow(attempts as u32), max_delay);
                // Add jitter (up to 25%)
                let jitter = (delay as f64 * 0.25 * rand::random::<f64>()) as u64;
                Duration::from_millis(delay + jitter)
            })
;

        info!("📡 Connecting to NATS at {} as '{}'...", config.url, config.connection_name);
        
        let client = connect_options.connect(&config.url).await?;
        
        info!("✅ NATS Client connected to {} with auto-reconnect enabled", config.url);
        
        let _ = NATS_INSTANCE.set(Arc::new(client));
        Ok(())
    }

    /// Get the shared NATS client instance
    pub fn global() -> Option<Client> {
        NATS_INSTANCE.get().map(|c| (**c).clone())
    }

    /// Check if NATS is connected
    pub fn is_connected() -> bool {
        if let Some(client) = Self::global() {
            // Check connection state
            matches!(client.connection_state(), async_nats::connection::State::Connected)
        } else {
            false
        }
    }

    /// Get the NATS connection state as a string
    pub fn connection_status() -> &'static str {
        if let Some(client) = Self::global() {
            match client.connection_state() {
                async_nats::connection::State::Connected => "connected",
                async_nats::connection::State::Pending => "connecting",
                async_nats::connection::State::Disconnected => "disconnected",
            }
        } else {
            "not_initialized"
        }
    }

    /// Convenience wrapper to publish a JSON event with Trace Context
    pub async fn publish_event<T: serde::Serialize>(subject: &str, event: &T) -> Result<(), NatsError> {
        let client = Self::global().ok_or(NatsError::NotInitialized)?;

        let payload = serde_json::to_vec(event)
            .map_err(|e| NatsError::SerializationError(e.to_string()))?;
        
        // Inject Trace Context
        let mut headers = async_nats::HeaderMap::new();
        let cx = tracing::Span::current().context();
        
        opentelemetry::global::get_text_map_propagator(|propagator| {
            propagator.inject_context(&cx, &mut NatsHeaderInjector(&mut headers));
        });

        client.publish_with_headers(subject.to_string(), headers, payload.into()).await
            .map_err(|e| NatsError::PublishError(e.to_string()))?;
        
        Ok(())
    }

    /// Publish with retry logic
    pub async fn publish_event_with_retry<T: serde::Serialize>(
        subject: &str, 
        event: &T,
        max_retries: u32,
    ) -> Result<(), NatsError> {
        let mut attempts = 0;
        loop {
            match Self::publish_event(subject, event).await {
                Ok(()) => return Ok(()),
                Err(e) if attempts < max_retries => {
                    attempts += 1;
                    warn!("NATS publish failed (attempt {}/{}): {}. Retrying...", attempts, max_retries, e);
                    tokio::time::sleep(Duration::from_millis(100 * 2u64.pow(attempts))).await;
                }
                Err(e) => return Err(e),
            }
        }
    }
}

/// NATS-specific error types
#[derive(Debug, thiserror::Error)]
pub enum NatsError {
    #[error("NATS client not initialized. Call NatsClient::init() first.")]
    NotInitialized,
    
    #[error("Failed to serialize event: {0}")]
    SerializationError(String),
    
    #[error("Failed to publish message: {0}")]
    PublishError(String),
    
    #[error("Connection error: {0}")]
    ConnectionError(String),
}

/// Helper for injecting OTEL context into NATS headers
struct NatsHeaderInjector<'a>(&'a mut async_nats::HeaderMap);

impl<'a> Injector for NatsHeaderInjector<'a> {
    fn set(&mut self, key: &str, value: String) {
        if let Ok(name) = key.parse::<async_nats::header::HeaderName>() {
            if let Ok(val) = value.parse::<async_nats::header::HeaderValue>() {
                self.0.insert(name, val);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = NatsConfig::default();
        assert_eq!(config.max_reconnects, 0);
        assert_eq!(config.reconnect_delay, Duration::from_millis(500));
    }

    #[test]
    fn test_service_config() {
        let config = NatsConfig::for_service("lanai-inventory-service");
        assert_eq!(config.connection_name, "lanai-inventory-service");
    }
}
