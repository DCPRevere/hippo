use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

pub struct RateLimiter {
    buckets: Mutex<HashMap<String, TokenBucket>>,
    capacity: u32,
}

struct TokenBucket {
    tokens: f64,
    last_refill: Instant,
}

impl RateLimiter {
    pub fn new(requests_per_minute: u32) -> Self {
        Self {
            buckets: Mutex::new(HashMap::new()),
            capacity: requests_per_minute,
        }
    }

    /// Returns Ok(()) if allowed, Err(()) if rate limited.
    #[allow(clippy::result_unit_err)]
    pub fn check(&self, user_id: &str) -> Result<(), ()> {
        let mut buckets = self.buckets.lock().expect("rate limiter lock poisoned");
        let cap = self.capacity as f64;
        let rate_per_sec = cap / 60.0;

        let bucket = buckets
            .entry(user_id.to_string())
            .or_insert_with(|| TokenBucket {
                tokens: cap,
                last_refill: Instant::now(),
            });

        // Refill tokens based on elapsed time
        let now = Instant::now();
        let elapsed = now.duration_since(bucket.last_refill).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * rate_per_sec).min(cap);
        bucket.last_refill = now;

        // Try to consume one token
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            Ok(())
        } else {
            Err(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn allows_requests_within_limit() {
        let limiter = RateLimiter::new(10);
        for _ in 0..10 {
            assert!(limiter.check("alice").is_ok());
        }
    }

    #[test]
    fn rejects_when_exhausted() {
        let limiter = RateLimiter::new(2);
        assert!(limiter.check("bob").is_ok());
        assert!(limiter.check("bob").is_ok());
        assert!(limiter.check("bob").is_err());
    }

    #[test]
    fn separate_buckets_per_user() {
        let limiter = RateLimiter::new(1);
        assert!(limiter.check("alice").is_ok());
        assert!(limiter.check("bob").is_ok());
        assert!(limiter.check("alice").is_err());
        assert!(limiter.check("bob").is_err());
    }

    #[test]
    fn refills_over_time() {
        let limiter = RateLimiter::new(60); // 1 per second
                                            // Exhaust all tokens
        for _ in 0..60 {
            let _ = limiter.check("carol");
        }
        assert!(limiter.check("carol").is_err());

        // Wait just over 1 second to refill ~1 token
        thread::sleep(Duration::from_millis(1100));
        assert!(limiter.check("carol").is_ok());
    }

    #[test]
    fn capacity_is_capped() {
        let limiter = RateLimiter::new(5);
        // Wait a bit (tokens should not exceed capacity)
        thread::sleep(Duration::from_millis(200));
        // Should be able to use exactly 5
        for _ in 0..5 {
            assert!(limiter.check("dave").is_ok());
        }
        assert!(limiter.check("dave").is_err());
    }
}
