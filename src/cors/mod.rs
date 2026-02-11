//! CORS Configuration for Lanai Services
//!
//! Provides a centralized, secure CORS configuration that reads allowed origins
//! from environment variables. This ensures consistent security across all microservices.

use actix_cors::Cors;
use actix_web::http::header;
use log::info;

/// Environment variable name for allowed origins (comma-separated).
pub const CORS_ALLOWED_ORIGINS_ENV: &str = "CORS_ALLOWED_ORIGINS";

/// Default allowed origins for development.
const DEV_ORIGINS: &[&str] = &[
    "http://localhost:5173",
    "http://localhost:3000",
    "http://localhost:8080",
    "http://127.0.0.1:5173",
    "http://127.0.0.1:3000",
    "http://127.0.0.1:8080",
];

/// Creates a properly configured CORS middleware for production use.
///
/// # Configuration
/// - Reads `CORS_ALLOWED_ORIGINS` environment variable (comma-separated list).
/// - Falls back to development origins if not set.
/// - Always allows credentials.
/// - Restricts methods to GET, POST, PUT, PATCH, DELETE, OPTIONS.
/// - Allows common headers + custom Lanai headers.
///
/// # Example
/// ```ignore
/// use lanai_infrastructure::cors::create_cors;
///
/// App::new()
///     .wrap(create_cors())
///     // ... rest of app
/// ```
pub fn create_cors() -> Cors {
    let allowed_origins = get_allowed_origins();
    
    info!(
        "🔒 CORS configured with {} allowed origin(s): {:?}",
        allowed_origins.len(),
        if allowed_origins.len() <= 3 { 
            allowed_origins.join(", ") 
        } else { 
            format!("{}, ... and {} more", allowed_origins[..2].join(", "), allowed_origins.len() - 2)
        }
    );

    let mut cors = Cors::default()
        .allowed_methods(vec!["GET", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"])
        .allowed_headers(vec![
            header::AUTHORIZATION,
            header::ACCEPT,
            header::CONTENT_TYPE,
            header::ORIGIN,
            header::HeaderName::from_static("x-csrf-token"),
            header::HeaderName::from_static("x-organization-id"),
            header::HeaderName::from_static("x-tenant-id"),
            header::HeaderName::from_static("x-user-id"),
            header::HeaderName::from_static("x-store-id"),
            header::HeaderName::from_static("x-request-id"),
        ])
        .expose_headers(vec![
            header::HeaderName::from_static("x-request-id"),
            header::HeaderName::from_static("x-rate-limit-remaining"),
        ])
        .supports_credentials()
        .max_age(3600);

    // Add each allowed origin
    for origin in allowed_origins {
        cors = cors.allowed_origin(&origin);
    }

    cors
}

/// Gets the list of allowed origins from environment or defaults.
fn get_allowed_origins() -> Vec<String> {
    match std::env::var(CORS_ALLOWED_ORIGINS_ENV) {
        Ok(origins) if !origins.is_empty() => {
            origins
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        }
        _ => {
            // Development fallback
            log::warn!(
                "⚠️ {} not set. Using development defaults. SET THIS IN PRODUCTION!",
                CORS_ALLOWED_ORIGINS_ENV
            );
            DEV_ORIGINS.iter().map(|s| s.to_string()).collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_allowed_origins_from_env() {
        std::env::set_var(CORS_ALLOWED_ORIGINS_ENV, "https://app.posc.com,https://admin.posc.com");
        let origins = get_allowed_origins();
        assert_eq!(origins.len(), 2);
        assert!(origins.contains(&"https://app.posc.com".to_string()));
        std::env::remove_var(CORS_ALLOWED_ORIGINS_ENV);
    }

    #[test]
    fn test_get_allowed_origins_fallback() {
        std::env::remove_var(CORS_ALLOWED_ORIGINS_ENV);
        let origins = get_allowed_origins();
        assert!(!origins.is_empty());
        assert!(origins.iter().any(|o| o.contains("localhost")));
    }
}
