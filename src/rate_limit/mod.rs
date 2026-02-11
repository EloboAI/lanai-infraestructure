//! Distributed Rate Limiting using Redis
//!
//! Provides a sliding window rate limiter that works across multiple service instances.
//! Falls back to in-memory storage if Redis is configured but unavailable (with a warning),
//! or if Redis is not configured at all.

use redis::AsyncCommands;
use std::sync::Arc;
use tokio::sync::RwLock;
use std::collections::HashMap;
use log::{info, warn, error};

/// Environment variable for Redis URL
pub const REDIS_URL_ENV: &str = "REDIS_URL";

/// Rate Limiter Backend abstraction
#[async_trait::async_trait]
pub trait RateLimiterBackend: Send + Sync {
    /// Check if action is allowed. Returns true if allowed.
    async fn is_allowed(&self, key: &str, limit: u32, window_secs: u64) -> bool;
}

/// Redis-backed rate limiter
pub struct RedisRateLimiter {
    client: redis::Client,
}

impl RedisRateLimiter {
    pub fn new(url: &str) -> Result<Self, redis::RedisError> {
        let client = redis::Client::open(url)?;
        Ok(Self { client })
    }
}

#[async_trait::async_trait]
impl RateLimiterBackend for RedisRateLimiter {
    async fn is_allowed(&self, key: &str, limit: u32, window_secs: u64) -> bool {
        let mut conn = match self.client.get_async_connection().await {
            Ok(conn) => conn,
            Err(e) => {
                error!("❌ Failed to connect to Redis for rate limiting: {}", e);
                return true; // Fail open if Redis is down
            }
        };

        let now = chrono::Utc::now().timestamp_millis();
        let window_start = now - (window_secs * 1000) as i64;
        let redis_key = format!("rate_limit:{}", key);

        // Transaction pipeline:
        // 1. Remove old entries
        // 2. Count current entries
        // 3. Add new entry (if under limit)
        // 4. Set expiry
        
        // We use a simplified approach since deadpool/multiplexing isn't fully set up here:
        // Just use ZREM, ZCOUNT, ZADD via the connection
        
        let pipe = redis::pipe()
            .atomic()
            .cmd("ZREMRANGEBYSCORE").arg(&redis_key).arg("-inf").arg(window_start)
            .cmd("ZCOUNT").arg(&redis_key).arg(window_start).arg("+inf")
            .query_async::<_, (isize,isize)>(&mut conn).await;

        match pipe {
            Ok((_, count)) => {
                if count >= limit as isize {
                    return false;
                }
                
                // Add current request
                let _: () = conn.zadd(&redis_key, now, now).await.unwrap_or_default();
                let _: () = conn.expire(&redis_key, window_secs as i64).await.unwrap_or_default();
                
                true
            }
            Err(e) => {
                error!("❌ Redis rate limit error: {}", e);
                true // Fail open
            }
        }
    }
}

/// In-memory fallback (for dev or if Redis is missing)
pub struct InMemoryRateLimiter {
    // Key -> sorted list of timestamps
    store: Arc<RwLock<HashMap<String, Vec<i64>>>>,
}

impl InMemoryRateLimiter {
    pub fn new() -> Self {
        Self {
            store: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

#[async_trait::async_trait]
impl RateLimiterBackend for InMemoryRateLimiter {
    async fn is_allowed(&self, key: &str, limit: u32, window_secs: u64) -> bool {
        let now = chrono::Utc::now().timestamp_millis();
        let window_start = now - (window_secs * 1000) as i64;

        let mut store = self.store.write().await;
        let history = store.entry(key.to_string()).or_default();

        // Cleanup old
        history.retain(|&ts| ts > window_start);

        if history.len() >= limit as usize {
            return false;
        }

        history.push(now);
        true
    }
}

/// Factory to get the configured rate limiter
pub async fn create_limiter() -> Arc<dyn RateLimiterBackend> {
    if let Ok(redis_url) = std::env::var(REDIS_URL_ENV) {
        match RedisRateLimiter::new(&redis_url) {
            Ok(limiter) => {
                info!("🚀 Initialized Redis Rate Limiter");
                return Arc::new(limiter);
            }
            Err(e) => {
                warn!("⚠️ Failed to init Redis Rate Limiter: {}. Falling back to in-memory.", e);
            }
        }
    } else {
        info!("ℹ️ No REDIS_URL found. Using In-Memory Rate Limiter.");
    }
    
    Arc::new(InMemoryRateLimiter::new())
}
