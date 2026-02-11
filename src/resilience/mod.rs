//! Circuit Breaker Pattern for Resilience
//!
//! This module implements the Circuit Breaker pattern to prevent cascading failures
//! in distributed systems. When a service is failing, the circuit "opens" to prevent
//! further calls and allow the service time to recover.

use std::sync::Arc;
use tokio::sync::Mutex;
use std::time::{Duration, Instant};
use log::{info, warn, error};
use thiserror::Error;

/// Represents the current state of the circuit breaker.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CircuitState {
    /// Normal operation - requests are allowed through.
    Closed,
    /// Failure threshold reached - requests are blocked.
    Open,
    /// Testing recovery - limited requests allowed to check if service recovered.
    HalfOpen,
}

/// Error types specific to the Circuit Breaker.
#[derive(Debug, Error, Clone)]
pub enum CircuitBreakerError {
    /// The circuit is open and rejecting calls.
    #[error("Circuit breaker is open. Service unavailable.")]
    Open,
    /// The operation timed out.
    #[error("Operation timed out")]
    Timeout,
    /// The underlying operation failed.
    #[error("Operation failed: {0}")]
    OperationFailed(String),
}

/// Result type that can hold either the operation result or a circuit breaker error.
pub type CircuitBreakerResult<T, E> = Result<T, CircuitBreakerOutcome<E>>;

/// Outcome of a circuit breaker call - either the original error or a CB-specific error.
#[derive(Debug)]
pub enum CircuitBreakerOutcome<E> {
    /// The circuit breaker blocked the call.
    CircuitOpen,
    /// The underlying operation returned an error.
    OperationError(E),
}

impl<E: std::fmt::Display> std::fmt::Display for CircuitBreakerOutcome<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CircuitOpen => write!(f, "Circuit breaker is open"),
            Self::OperationError(e) => write!(f, "Operation error: {}", e),
        }
    }
}

impl<E: std::fmt::Debug + std::fmt::Display> std::error::Error for CircuitBreakerOutcome<E> {}

/// A thread-safe Circuit Breaker implementation.
///
/// # Example
/// ```ignore
/// let cb = CircuitBreaker::new(5, Duration::from_secs(30));
///
/// let result = cb.call(|| async {
///     some_remote_service_call().await
/// }).await;
///
/// match result {
///     Ok(value) => println!("Success: {:?}", value),
///     Err(CircuitBreakerOutcome::CircuitOpen) => println!("Service unavailable"),
///     Err(CircuitBreakerOutcome::OperationError(e)) => println!("Call failed: {}", e),
/// }
/// ```
pub struct CircuitBreaker {
    state: Arc<Mutex<CircuitState>>,
    failure_threshold: u32,
    failure_count: Arc<Mutex<u32>>,
    success_threshold: u32,
    success_count: Arc<Mutex<u32>>,
    reset_timeout: Duration,
    last_failure_time: Arc<Mutex<Option<Instant>>>,
}

impl CircuitBreaker {
    /// Creates a new Circuit Breaker with the given configuration.
    ///
    /// # Arguments
    /// * `failure_threshold` - Number of consecutive failures before opening the circuit.
    /// * `reset_timeout` - Duration to wait before transitioning from Open to HalfOpen.
    pub fn new(failure_threshold: u32, reset_timeout: Duration) -> Self {
        Self {
            state: Arc::new(Mutex::new(CircuitState::Closed)),
            failure_threshold,
            failure_count: Arc::new(Mutex::new(0)),
            success_threshold: 2, // Require 2 consecutive successes in HalfOpen to close
            success_count: Arc::new(Mutex::new(0)),
            reset_timeout,
            last_failure_time: Arc::new(Mutex::new(None)),
        }
    }

    /// Creates a new Circuit Breaker with custom success threshold for HalfOpen → Closed transition.
    pub fn with_success_threshold(mut self, threshold: u32) -> Self {
        self.success_threshold = threshold;
        self
    }

    /// Returns the current state of the circuit breaker.
    pub async fn state(&self) -> CircuitState {
        *self.state.lock().await
    }

    /// Executes an async operation through the circuit breaker.
    ///
    /// If the circuit is Open, returns `Err(CircuitBreakerOutcome::CircuitOpen)` immediately.
    /// If the circuit is Closed or HalfOpen, executes the operation.
    pub async fn call<F, Fut, T, E>(&self, f: F) -> CircuitBreakerResult<T, E>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<T, E>>,
        E: std::fmt::Display,
    {
        // Check if circuit should transition from Open to HalfOpen
        {
            let mut state = self.state.lock().await;
            if *state == CircuitState::Open {
                let last_failure = self.last_failure_time.lock().await;
                if let Some(instant) = *last_failure {
                    if instant.elapsed() >= self.reset_timeout {
                        *state = CircuitState::HalfOpen;
                        // Reset success count for HalfOpen testing
                        let mut success_count = self.success_count.lock().await;
                        *success_count = 0;
                        warn!("Circuit Breaker: Reset timeout elapsed. State transitioning to HalfOpen.");
                    } else {
                        error!("Circuit Breaker: Operation rejected. State is Open. Retry in {:?}", 
                               self.reset_timeout - instant.elapsed());
                        return Err(CircuitBreakerOutcome::CircuitOpen);
                    }
                }
            }
        }

        // Execute the operation
        match f().await {
            Ok(res) => {
                let mut state = self.state.lock().await;
                
                if *state == CircuitState::HalfOpen {
                    let mut success_count = self.success_count.lock().await;
                    *success_count += 1;
                    
                    if *success_count >= self.success_threshold {
                        info!("Circuit Breaker: {} consecutive successes in HalfOpen. Transitioning to Closed.", 
                              self.success_threshold);
                        *state = CircuitState::Closed;
                        let mut failures = self.failure_count.lock().await;
                        *failures = 0;
                        *success_count = 0;
                    } else {
                        info!("Circuit Breaker: Success in HalfOpen ({}/{})", 
                              *success_count, self.success_threshold);
                    }
                } else if *state == CircuitState::Closed {
                    // Reset failure count on success in Closed state
                    let mut failures = self.failure_count.lock().await;
                    *failures = 0;
                }
                
                Ok(res)
            }
            Err(e) => {
                let mut failures = self.failure_count.lock().await;
                *failures += 1;

                let mut state = self.state.lock().await;
                
                // In HalfOpen, any failure immediately opens the circuit
                if *state == CircuitState::HalfOpen {
                    *state = CircuitState::Open;
                    let mut last_failure = self.last_failure_time.lock().await;
                    *last_failure = Some(Instant::now());
                    error!("Circuit Breaker: Failure in HalfOpen. Reopening circuit. Error: {}", e);
                } else if *failures >= self.failure_threshold {
                    *state = CircuitState::Open;
                    let mut last_failure = self.last_failure_time.lock().await;
                    *last_failure = Some(Instant::now());
                    error!("Circuit Breaker: Failure threshold reached ({}). Transitioning to Open. Error: {}", 
                           self.failure_threshold, e);
                }
                
                Err(CircuitBreakerOutcome::OperationError(e))
            }
        }
    }

    /// Manually reset the circuit breaker to Closed state.
    pub async fn reset(&self) {
        let mut state = self.state.lock().await;
        *state = CircuitState::Closed;
        let mut failures = self.failure_count.lock().await;
        *failures = 0;
        let mut successes = self.success_count.lock().await;
        *successes = 0;
        info!("Circuit Breaker: Manually reset to Closed state.");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_circuit_breaker_stays_closed_on_success() {
        let cb = CircuitBreaker::new(3, Duration::from_secs(5));
        
        let result: CircuitBreakerResult<i32, &str> = cb.call(|| async { Ok(42) }).await;
        assert!(result.is_ok());
        assert_eq!(cb.state().await, CircuitState::Closed);
    }

    #[tokio::test]
    async fn test_circuit_breaker_opens_after_threshold() {
        let cb = CircuitBreaker::new(2, Duration::from_secs(5));
        
        // First failure
        let _: CircuitBreakerResult<i32, &str> = cb.call(|| async { Err("fail1") }).await;
        assert_eq!(cb.state().await, CircuitState::Closed);
        
        // Second failure - should open
        let _: CircuitBreakerResult<i32, &str> = cb.call(|| async { Err("fail2") }).await;
        assert_eq!(cb.state().await, CircuitState::Open);
    }

    #[tokio::test]
    async fn test_circuit_breaker_rejects_when_open() {
        let cb = CircuitBreaker::new(1, Duration::from_secs(60));
        
        // Trigger opening
        let _: CircuitBreakerResult<i32, &str> = cb.call(|| async { Err("fail") }).await;
        assert_eq!(cb.state().await, CircuitState::Open);
        
        // Next call should be rejected
        let result: CircuitBreakerResult<i32, &str> = cb.call(|| async { Ok(42) }).await;
        assert!(matches!(result, Err(CircuitBreakerOutcome::CircuitOpen)));
    }
}
