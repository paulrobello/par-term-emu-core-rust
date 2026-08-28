//! Input rate limiting for streaming clients (ARC-004).
//!
//! Split out of `server.rs`: the token-bucket limiter that caps per-client
//! input bytes when `StreamingConfig::input_rate_limit_bytes_per_sec` is set.

// =============================================================================
// Input Rate Limiter
// =============================================================================

/// Token bucket rate limiter for per-client input
pub(crate) struct InputRateLimiter {
    tokens: f64,
    max_tokens: f64,
    rate: f64,
    last_check: tokio::time::Instant,
}

impl InputRateLimiter {
    /// Create a new rate limiter with the given bytes-per-second rate.
    /// Burst capacity is 2x the rate.
    pub(crate) fn new(bytes_per_sec: usize) -> Self {
        let rate = bytes_per_sec as f64;
        let max_tokens = rate * 2.0;
        Self {
            tokens: max_tokens,
            max_tokens,
            rate,
            last_check: tokio::time::Instant::now(),
        }
    }

    /// Try to consume `bytes` tokens. Returns true if allowed.
    pub(crate) fn try_consume(&mut self, bytes: usize) -> bool {
        let now = tokio::time::Instant::now();
        let elapsed = now.duration_since(self.last_check).as_secs_f64();
        self.last_check = now;

        // Replenish tokens
        self.tokens = (self.tokens + elapsed * self.rate).min(self.max_tokens);

        let cost = bytes as f64;
        if self.tokens >= cost {
            self.tokens -= cost;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_rate_limiter_initial_burst_capacity() {
        let mut limiter = InputRateLimiter::new(1000);
        // 1000 bytes/sec → burst = 2000 bytes; starts full
        assert!(
            limiter.try_consume(2000),
            "should allow consuming burst capacity"
        );
    }
    #[tokio::test]
    async fn test_rate_limiter_rejects_over_burst() {
        let mut limiter = InputRateLimiter::new(1000);
        // Burst is 2000; requesting 2001 should fail
        assert!(
            !limiter.try_consume(2001),
            "should reject request exceeding burst capacity"
        );
    }
    #[tokio::test]
    async fn test_rate_limiter_exhausted_rejects() {
        let mut limiter = InputRateLimiter::new(1000);
        assert!(limiter.try_consume(2000));
        assert!(!limiter.try_consume(1), "should reject after exhaustion");
    }
    #[tokio::test]
    async fn test_rate_limiter_zero_bytes_always_allowed() {
        let mut limiter = InputRateLimiter::new(1000);
        assert!(limiter.try_consume(0));
        assert!(limiter.try_consume(2000));
        assert!(
            limiter.try_consume(0),
            "zero bytes should be allowed even when exhausted"
        );
    }
    #[tokio::test]
    async fn test_rate_limiter_refills_over_time() {
        tokio::time::pause();
        let mut limiter = InputRateLimiter::new(1000);
        assert!(limiter.try_consume(2000));
        assert!(!limiter.try_consume(1));
        tokio::time::advance(std::time::Duration::from_secs(1)).await;
        assert!(
            limiter.try_consume(1000),
            "should allow 1000 bytes after 1 second refill"
        );
        assert!(
            !limiter.try_consume(1),
            "should reject after consuming refilled tokens"
        );
    }
    #[tokio::test]
    async fn test_rate_limiter_capped_at_max_tokens() {
        tokio::time::pause();
        let mut limiter = InputRateLimiter::new(1000);
        tokio::time::advance(std::time::Duration::from_secs(10)).await;
        // Even after 10 seconds, tokens should be capped at 2000 (not 10000)
        assert!(limiter.try_consume(2000), "should allow burst capacity");
        assert!(
            !limiter.try_consume(1),
            "should not exceed max_tokens even after long wait"
        );
    }
    #[tokio::test]
    async fn test_rate_limiter_partial_refill() {
        tokio::time::pause();
        let mut limiter = InputRateLimiter::new(1000);
        // Consume 1500 bytes (leaving 500)
        assert!(limiter.try_consume(1500));
        // Advance 0.25 seconds → refill 250 tokens → total ~750
        tokio::time::advance(std::time::Duration::from_millis(250)).await;
        assert!(
            limiter.try_consume(750),
            "should allow ~750 bytes after partial refill"
        );
    }
    #[tokio::test]
    async fn test_rate_limiter_sequential_small_requests() {
        let mut limiter = InputRateLimiter::new(1000);
        // 2000 burst capacity; 10 requests of 200 bytes = 2000 total, all should pass
        for _ in 0..10 {
            assert!(
                limiter.try_consume(200),
                "each 200-byte chunk should be allowed within burst"
            );
        }
        // 11th request should fail
        assert!(!limiter.try_consume(200));
    }
}
