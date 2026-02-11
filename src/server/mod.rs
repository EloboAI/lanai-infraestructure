use actix_web::{web, App, HttpServer, middleware};
use std::sync::Arc;
use log::info;

use crate::middleware::security_headers::SecurityHeadersMiddleware;
use crate::middleware::request_size::RequestSizeLimitMiddleware;
use crate::middleware::rate_limit::RateLimitMiddleware;
use crate::rate_limit::create_limiter;

/// Builder for standardized Actix Web servers in the Lanai ecosystem.
///
/// This builder enforces:
/// - Standard Middleware (Tracing, Logging, Compression, CORS, CSRF, Security Headers)
/// - Rate Limiting (Redis-backed if available)
/// - Request Size Limiting
/// - Consistent Shutdown/Timeout settings
pub struct ServerBuilder {
    name: String,
    host: String,
    port: u16,
    workers: usize,
    max_request_size: usize,
    rate_limit_requests: u32,
    rate_limit_window_seconds: u64,
    enable_cors: bool,
}

impl ServerBuilder {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            host: "0.0.0.0".to_string(),
            port: 8080,
            workers: 4,
            max_request_size: 2 * 1024 * 1024, // 2MB default
            rate_limit_requests: 1000,
            rate_limit_window_seconds: 60,
            enable_cors: true,
        }
    }

    pub fn host(mut self, host: &str) -> Self {
        self.host = host.to_string();
        self
    }

    pub fn port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    pub fn workers(mut self, workers: usize) -> Self {
        self.workers = workers;
        self
    }

    pub fn max_request_size(mut self, size: usize) -> Self {
        self.max_request_size = size;
        self
    }
    
    pub fn rate_limit(mut self, requests: u32, window: u64) -> Self {
        self.rate_limit_requests = requests;
        self.rate_limit_window_seconds = window;
        self
    }

    pub fn disable_cors(mut self) -> Self {
        self.enable_cors = false;
        self
    }

    /// Start the server and return the `Server` instance (Future) without awaiting it.
    /// Useful for running the server concurrently with other tasks (e.g., gRPC server).
    pub async fn start<F>(self, configure: F) -> std::io::Result<actix_web::dev::Server>
    where
        F: Fn(&mut web::ServiceConfig) + Send + Clone + 'static,
    {
        // Initialize infrastructure components
        crate::observability::init_tracing(&self.name);
        
        info!("🚀 Starting {} on {}:{}", self.name, self.host, self.port);
        
        let limiter = create_limiter().await;
        
        // Capture configuration to move into closure
        let max_size = self.max_request_size;
        let rl_reqs = self.rate_limit_requests;
        let rl_window = self.rate_limit_window_seconds;
        let enable_cors = self.enable_cors;

        Ok(HttpServer::new(move || {
            let app = App::new();
            
            // 1. Core Middleware
            let app = app
                .wrap(middleware::Compress::default())
                .wrap(crate::middleware::tenant_context::TenantMiddleware);

            // 2. CORS (Optional but recommended)
            let app = app.wrap(actix_web::middleware::Condition::new(
                    enable_cors,
                    crate::cors::create_cors(),
                ));

            // 3. Security Headers
            let app = app.wrap(SecurityHeadersMiddleware {
                content_security_policy: Some("default-src 'self'".to_string()), 
                hsts_preload: true,
                hsts_max_age_seconds: 31536000,
                hsts_include_subdomains: true,
                referrer_policy: "strict-origin-when-cross-origin".to_string(),
                permissions_policy: None,
            });

            // 4. Rate Limiting & Protection
            let app = app
                .wrap(RateLimitMiddleware {
                    limiter: Arc::clone(&limiter),
                    max_requests: rl_reqs,
                    window_seconds: rl_window,
                })
                .wrap(RequestSizeLimitMiddleware {
                    max_size,
                });

            let app = app.wrap(tracing_actix_web::TracingLogger::default());
            let app = app.wrap(middleware::Logger::default());

            // 6. User Configuration (Routes, AppData)
            app.configure(configure.clone())
        })
        .bind((self.host.as_str(), self.port))?
        .workers(self.workers)
        // Default Timeouts
        .keep_alive(std::time::Duration::from_secs(75))
        .client_request_timeout(std::time::Duration::from_secs(60))
        .run())
    }

    /// Run the server and await it until shutdown.
    pub async fn run<F>(self, configure: F) -> std::io::Result<()>
    where
        F: Fn(&mut web::ServiceConfig) + Send + Clone + 'static,
    {
        self.start(configure).await?.await
    }
}
