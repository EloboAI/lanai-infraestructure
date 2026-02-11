use actix_web::{
    body::{BoxBody, MessageBody},
    dev::{Service, ServiceRequest, ServiceResponse, Transform},
    Error, HttpMessage, HttpResponse,
};
use futures_util::future::{ok, LocalBoxFuture, Ready};
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use std::{rc::Rc, sync::Arc};
use log::{warn, error};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    pub sub: String,
    pub email: String,
    pub username: String,
    pub role: String,
    pub org_id: Option<String>,
    pub vertical: Option<String>,
    pub exp: i64,
    pub iat: i64,
    pub iss: String,
    pub jti: String,
}

pub struct AuthGuard {
    pub public_key_pem: String,
}

impl AuthGuard {
    /// Create new AuthGuard with Public Key PEM
    pub fn new(public_key_pem: String) -> Self {
        Self {
            public_key_pem,
        }
    }
}

impl<S, B> Transform<S, ServiceRequest> for AuthGuard
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: MessageBody + 'static, 
{
    type Response = ServiceResponse<BoxBody>;
    type Error = Error;
    type InitError = ();
    type Transform = AuthGuardMiddleware<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        // Support for single-line env variables with \n
        let pem_str = self.public_key_pem.replace("\\n", "\n");
        let decoding_key = match DecodingKey::from_rsa_pem(pem_str.as_bytes()) {
            Ok(k) => k,
            Err(e) => {
                error!("❌ FATAL: Failed to parse JWT Public Key PEM in AuthGuard: {}", e);
                // We panic here because if the key is invalid, security is broken.
                panic!("Invalid JWT Public Key PEM");
            }
        };

        ok(AuthGuardMiddleware {
            service: Rc::new(service),
            decoding_key: Arc::new(decoding_key),
        })
    }
}

pub struct AuthGuardMiddleware<S> {
    service: Rc<S>,
    decoding_key: Arc<DecodingKey>,
}

impl<S, B> Service<ServiceRequest> for AuthGuardMiddleware<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: MessageBody + 'static,
{
    type Response = ServiceResponse<BoxBody>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&self, ctx: &mut core::task::Context<'_>) -> core::task::Poll<Result<(), Self::Error>> {
        self.service.poll_ready(ctx)
    }

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let service = self.service.clone();
        let decoding_key = self.decoding_key.clone();

        Box::pin(async move {
            // Allow OPTIONS for CORS preflight
            if req.method() == actix_web::http::Method::OPTIONS {
                let res = service.call(req).await?;
                return Ok(res.map_into_boxed_body());
            }

            let token = match extract_token_from_request(&req) {
                Some(token) => token,
                None => {
                    return Ok(req.into_response(
                        HttpResponse::Unauthorized()
                            .json(serde_json::json!({
                                "error": "Missing authentication token"
                            }))
                    ).map_into_boxed_body());
                }
            };

            let mut validation = Validation::new(Algorithm::RS256);
            validation.set_issuer(&["lanai-auth"]);
            validation.set_required_spec_claims(&["exp", "sub"]);

            match decode::<Claims>(&token, &decoding_key, &validation) {
                Ok(token_data) => {
                    req.extensions_mut().insert(token_data.claims);
                    let res = service.call(req).await?;
                    Ok(res.map_into_boxed_body())
                }
                Err(e) => {
                    warn!("Token validation failed: {}", e);
                    Ok(req.into_response(
                        HttpResponse::Unauthorized()
                            .json(serde_json::json!({
                                "error": "Invalid or expired token"
                            }))
                    ).map_into_boxed_body())
                }
            }
        })
    }
}

/// Extract token from request headers or cookies
pub fn extract_token_from_request(req: &ServiceRequest) -> Option<String> {
    // 1. Try Authorization header
    if let Some(auth_header) = req.headers().get("Authorization") {
        if let Ok(auth_str) = auth_header.to_str() {
            if auth_str.starts_with("Bearer ") {
                return Some(auth_str[7..].to_string());
            }
        }
    }

    // 2. Try cookie fallback with CSRF protection
    if let Some(cookie_header) = req.headers().get("cookie") {
        if let Ok(cookie_str) = cookie_header.to_str() {
            let mut cookies = std::collections::HashMap::new();
            for cookie in cookie_str.split(';') {
                let cookie = cookie.trim();
                // Simple parser, for robust parsing actix-web::cookie should be used if available
                if let Some(idx) = cookie.find('=') {
                    let (k, v) = cookie.split_at(idx);
                    cookies.insert(k.trim(), v[1..].trim());
                }
            }

            if let Some(access_token) = cookies.get("access_token") {
                // Mandatory CSRF check for cookie auth
                if let Some(csrf_cookie) = cookies.get("csrf_token") {
                    if let Some(csrf_header_val) = req.headers().get("X-CSRF-Token") {
                        if let Ok(csrf_header_str) = csrf_header_val.to_str() {
                            if csrf_header_str == *csrf_cookie {
                                return Some(access_token.to_string());
                            } else {
                                warn!("CSRF token mismatch: header != cookie");
                            }
                        }
                    } else {
                        warn!("Missing X-CSRF-Token header for cookie auth");
                    }
                } else {
                    warn!("Missing csrf_token cookie for cookie auth");
                }
            }
        }
    }

    None
}
