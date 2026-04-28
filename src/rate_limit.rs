use std::collections::HashMap;
use std::time::Instant;

use parking_lot::Mutex;

/// Source of monotonic time for the rate limiter.
///
/// Injected so tests can advance time deterministically without `thread::sleep`.
pub trait Clock: Send + Sync + 'static {
    fn now(&self) -> Instant;
}

/// Default clock backed by [`Instant::now`].
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

pub struct RateLimiter {
    buckets: Mutex<HashMap<String, TokenBucket>>,
    capacity: u32,
    clock: Box<dyn Clock>,
}

struct TokenBucket {
    tokens: f64,
    last_refill: Instant,
}

impl RateLimiter {
    pub fn new(requests_per_minute: u32) -> Self {
        Self::with_clock(requests_per_minute, Box::new(SystemClock))
    }

    pub fn with_clock(requests_per_minute: u32, clock: Box<dyn Clock>) -> Self {
        Self {
            buckets: Mutex::new(HashMap::new()),
            capacity: requests_per_minute,
            clock,
        }
    }

    /// Returns Ok(()) if allowed, Err(()) if rate limited.
    #[allow(clippy::result_unit_err)]
    pub fn check(&self, user_id: &str) -> Result<(), ()> {
        // parking_lot::Mutex never poisons, so a panic in one request handler
        // can't take down rate limiting for the whole server.
        let mut buckets = self.buckets.lock();
        let cap = self.capacity as f64;
        let rate_per_sec = cap / 60.0;
        let now = self.clock.now();

        let bucket = buckets
            .entry(user_id.to_string())
            .or_insert_with(|| TokenBucket {
                tokens: cap,
                last_refill: now,
            });

        // Refill tokens based on elapsed time
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
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    /// Test clock: starts at a fixed origin and advances only when explicitly told to.
    struct FakeClock {
        origin: Instant,
        offset_ms: Arc<AtomicU64>,
    }

    impl FakeClock {
        fn new() -> (Self, Arc<AtomicU64>) {
            let offset = Arc::new(AtomicU64::new(0));
            (
                Self {
                    origin: Instant::now(),
                    offset_ms: offset.clone(),
                },
                offset,
            )
        }
    }

    impl Clock for FakeClock {
        fn now(&self) -> Instant {
            self.origin + Duration::from_millis(self.offset_ms.load(Ordering::SeqCst))
        }
    }

    fn limiter_with_clock(rpm: u32) -> (RateLimiter, Arc<AtomicU64>) {
        let (clock, handle) = FakeClock::new();
        (RateLimiter::with_clock(rpm, Box::new(clock)), handle)
    }

    fn advance(handle: &Arc<AtomicU64>, ms: u64) {
        handle.fetch_add(ms, Ordering::SeqCst);
    }

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
    fn refills_over_time_using_fake_clock() {
        let (limiter, clock) = limiter_with_clock(60); // 1 token/sec
        for _ in 0..60 {
            let _ = limiter.check("carol");
        }
        assert!(limiter.check("carol").is_err());

        // Advance just over 1 second; ~1 token should have refilled.
        advance(&clock, 1100);
        assert!(limiter.check("carol").is_ok());
    }

    #[test]
    fn capacity_is_capped_after_long_idle() {
        let (limiter, clock) = limiter_with_clock(5);
        // Advance an hour while idle; tokens should cap at 5, not accumulate.
        advance(&clock, 3_600_000);
        for _ in 0..5 {
            assert!(limiter.check("dave").is_ok());
        }
        assert!(limiter.check("dave").is_err());
    }

    #[test]
    fn fractional_refills_accumulate() {
        // 60rpm = 1 token/sec. Advancing 500ms gives 0.5 tokens; not enough on its own.
        let (limiter, clock) = limiter_with_clock(60);
        for _ in 0..60 {
            let _ = limiter.check("eve");
        }
        advance(&clock, 500);
        assert!(limiter.check("eve").is_err());
        // After another 600ms, total elapsed is 1100ms → 1 full token.
        advance(&clock, 600);
        assert!(limiter.check("eve").is_ok());
    }

    #[test]
    fn zero_capacity_rejects_immediately() {
        let limiter = RateLimiter::new(0);
        assert!(limiter.check("frank").is_err());
    }

    #[test]
    fn first_request_for_new_user_succeeds() {
        let limiter = RateLimiter::new(1);
        // Brand-new user starts with a full bucket.
        assert!(limiter.check("ghost").is_ok());
    }

    #[test]
    fn many_users_do_not_interact() {
        let limiter = RateLimiter::new(1);
        for i in 0..100 {
            let user = format!("u{}", i);
            assert!(limiter.check(&user).is_ok());
            assert!(limiter.check(&user).is_err());
        }
    }

    /// Regression: if a thread panics while holding the buckets mutex,
    /// std::sync::Mutex would poison and the next caller's `.lock().expect()`
    /// would crash. parking_lot::Mutex doesn't poison, so the limiter keeps
    /// working.
    #[test]
    fn limiter_survives_panic_while_holding_lock() {
        use std::panic::AssertUnwindSafe;
        let limiter = std::sync::Arc::new(RateLimiter::new(10));
        let limiter_clone = limiter.clone();
        let handle = std::thread::spawn(move || {
            let _ = std::panic::catch_unwind(AssertUnwindSafe(|| {
                let _guard = limiter_clone.buckets.lock();
                panic!("panic while holding buckets lock");
            }));
        });
        handle.join().unwrap();
        // Following call would .expect()-panic if std::Mutex had poisoned.
        assert!(limiter.check("after-panic").is_ok());
    }
}
