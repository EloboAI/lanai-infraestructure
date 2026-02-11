use actix_web::{
    body::BoxBody,
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    http::header,
    Error,
};
use futures_util::future::LocalBoxFuture;
use std::future::{ready, Ready};
use std::sync::Arc;

/// Security headers middleware
pub struct SecurityHeadersMiddleware {
    pub content_security_policy: Option<String>,
    pub hsts_preload: bool,
    pub hsts_max_age_seconds: u64,
    pub hsts_include_subdomains: bool,
    pub referrer_policy: String,
    pub permissions_policy: Option<String>,
}

impl<S, B> Transform<S, ServiceRequest> for SecurityHeadersMiddleware
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    S: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type InitError = ();
    type Transform = SecurityHeadersMiddlewareService<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(SecurityHeadersMiddlewareService {
            service: Arc::new(service),
            content_security_policy: self.content_security_policy.clone(),
            hsts_preload: self.hsts_preload,
            hsts_max_age_seconds: self.hsts_max_age_seconds,
            hsts_include_subdomains: self.hsts_include_subdomains,
            referrer_policy: self.referrer_policy.clone(),
            permissions_policy: self.permissions_policy.clone(),
        }))
    }
}

pub struct SecurityHeadersMiddlewareService<S> {
    service: Arc<S>,
    content_security_policy: Option<String>,
    hsts_preload: bool,
    hsts_max_age_seconds: u64,
    hsts_include_subdomains: bool,
    referrer_policy: String,
    permissions_policy: Option<String>,
}

impl<S, B> Service<ServiceRequest> for SecurityHeadersMiddlewareService<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    S: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let service = Arc::clone(&self.service);
        let content_security_policy = self.content_security_policy.clone();
        let hsts_preload = self.hsts_preload;
        let hsts_max_age_seconds = self.hsts_max_age_seconds;
        let hsts_include_subdomains = self.hsts_include_subdomains;
        let referrer_policy = self.referrer_policy.clone();
        let permissions_policy = self.permissions_policy.clone();

        Box::pin(async move {
            let mut res = service.call(req).await?;

            let headers = res.headers_mut();

            // Security headers
            headers.insert(
                header::X_CONTENT_TYPE_OPTIONS,
                header::HeaderValue::from_static("nosniff"),
            );

            headers.insert(
                header::X_FRAME_OPTIONS,
                header::HeaderValue::from_static("DENY"),
            );

            headers.insert(
                header::X_XSS_PROTECTION,
                header::HeaderValue::from_static("1; mode=block"),
            );

            // HSTS header: build value based on config
            let mut hsts_value = format!("max-age={}", hsts_max_age_seconds);
            if hsts_include_subdomains {
                hsts_value.push_str("; includeSubDomains");
            }
            if hsts_preload {
                hsts_value.push_str("; preload");
            }

            headers.insert(
                header::STRICT_TRANSPORT_SECURITY,
                header::HeaderValue::from_str(&hsts_value).unwrap_or_else(|_| {
                    header::HeaderValue::from_static("max-age=31536000; includeSubDomains")
                }),
            );

            // Referrer policy from config
            headers.insert(
                header::REFERRER_POLICY,
                header::HeaderValue::from_str(&referrer_policy).unwrap_or_else(|_| {
                    header::HeaderValue::from_static("strict-origin-when-cross-origin")
                }),
            );

            // Content Security Policy (optional)
            if let Some(csp) = &content_security_policy {
                if !csp.trim().is_empty() {
                    headers.insert(
                        header::HeaderName::from_static("content-security-policy"),
                        header::HeaderValue::from_str(csp).unwrap_or_else(|_| {
                            header::HeaderValue::from_static("default-src 'self'")
                        }),
                    );
                }
            }

            // Permissions-Policy / Feature-Policy (optional)
            if let Some(pp) = &permissions_policy {
                if !pp.trim().is_empty() {
                    headers.insert(
                        header::HeaderName::from_static("permissions-policy"),
                        header::HeaderValue::from_str(pp)
                            .unwrap_or_else(|_| header::HeaderValue::from_static("geolocation=()")),
                    );
                }
            }

            Ok(res)
        })
    }
}
