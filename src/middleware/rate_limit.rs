use actix_web::{
    body::{BoxBody, MessageBody},
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    Error, HttpMessage, HttpResponse,
};
use futures_util::future::LocalBoxFuture;
use std::future::{ready, Ready};
use std::sync::Arc;
use crate::rate_limit::RateLimiterBackend;

/// Rate limiting middleware
pub struct RateLimitMiddleware {
    pub limiter: Arc<dyn RateLimiterBackend>,
    pub max_requests: u32,
    pub window_seconds: u64,
}

impl<S, B> Transform<S, ServiceRequest> for RateLimitMiddleware
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    S: 'static,
    B: MessageBody + 'static,
{
    type Response = ServiceResponse<BoxBody>;
    type Error = Error;
    type InitError = ();
    type Transform = RateLimitMiddlewareService<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(RateLimitMiddlewareService {
            service: Arc::new(service),
            limiter: Arc::clone(&self.limiter),
            max_requests: self.max_requests,
            window_seconds: self.window_seconds,
        }))
    }
}

pub struct RateLimitMiddlewareService<S> {
    service: Arc<S>,
    limiter: Arc<dyn RateLimiterBackend>,
    max_requests: u32,
    window_seconds: u64,
}

impl<S, B> Service<ServiceRequest> for RateLimitMiddlewareService<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    S: 'static,
    B: MessageBody + 'static,
{
    type Response = ServiceResponse<BoxBody>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let service = Arc::clone(&self.service);
        let limiter = Arc::clone(&self.limiter);
        let max_requests = self.max_requests;
        let window_seconds = self.window_seconds;

        Box::pin(async move {
            // Skip rate limiting for internal and health routes
            let path = req.path();
            if path.starts_with("/internal") 
                || path.starts_with("/health")
                || path.starts_with("/api/v1/health") 
                || path.starts_with("/metrics")
            {
                return service.call(req).await.map(|res| res.map_body(|_, body| body.boxed()));
            }

            // Get client IP for rate limiting
            let ip = req
                .connection_info()
                .peer_addr()
                .unwrap_or("unknown")
                .to_string();

            // Try to extract identifying key
            let mut key_parts: Vec<String> = Vec::new();

            // API key header if present
            if let Some(api_val) = req.headers().get("x-api-key") {
                if let Ok(api_str) = api_val.to_str() {
                    key_parts.push(format!("api:{}", api_str));
                }
            }

            // Auth Header (simple extraction, no validation here to avoid overhead/coupling)
            if let Some(auth_val) = req.headers().get("authorization") {
                if let Ok(auth_str) = auth_val.to_str() {
                    if auth_str.starts_with("Bearer ") {
                        let token = &auth_str[7..];
                        // Use a short hash or prefix of the token
                        let short = &token[..std::cmp::min(16, token.len())];
                        key_parts.push(format!("token:{}", short));
                    }
                }
            }

            // Build final key
            let key = if !key_parts.is_empty() {
                format!("{}|ip:{}", key_parts.join("+"), ip)
            } else {
                ip.clone()
            };

            // Check rate limit
            if !limiter.is_allowed(&key, max_requests, window_seconds).await {
                let response = HttpResponse::TooManyRequests().json(
                    serde_json::json!({"error": "Rate limit exceeded. Please try again later."}),
                );
                return Ok(req.into_response(response));
            }

            service.call(req).await.map(|res| res.map_body(|_, body| body.boxed()))
        })
    }
}
