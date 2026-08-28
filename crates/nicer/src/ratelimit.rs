use std::time::{Duration, Instant};

use nicer_proto::payload::{ShutUp, ShutUpScope};

/// A leaky bucket that reports how long the caller must wait rather than merely refusing,
/// because `SHUT_UP` has to carry `retry_after_ms` (RFC R11).
#[derive(Debug)]
pub struct Bucket {
    capacity: f64,
    per_second: f64,
    tokens: f64,
    updated: Instant,
}

impl Bucket {
    pub fn per_minute(rate: u32) -> Self {
        Self::new(rate.max(1) as f64, rate.max(1) as f64 / 60.0)
    }

    pub fn per_second(rate: u32) -> Self {
        Self::new(rate.max(1) as f64, rate.max(1) as f64)
    }

    pub fn new(capacity: f64, per_second: f64) -> Self {
        Self {
            capacity,
            per_second,
            tokens: capacity,
            updated: Instant::now(),
        }
    }

    fn refill(&mut self, now: Instant) {
        let elapsed = now.duration_since(self.updated).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.per_second).min(self.capacity);
        self.updated = now;
    }

    /// `Ok(())` to proceed, `Err(delay)` to wait that long first.
    pub fn take(&mut self) -> Result<(), Duration> {
        self.take_at(Instant::now())
    }

    pub fn take_at(&mut self, now: Instant) -> Result<(), Duration> {
        self.refill(now);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            return Ok(());
        }
        let shortfall = 1.0 - self.tokens;
        Err(Duration::from_secs_f64(shortfall / self.per_second))
    }
}

/// The buckets a single connection is measured against.
#[derive(Debug)]
pub struct ConnectionLimiter {
    pub offers: Bucket,
    pub chat: Bucket,
    pub frames: Bucket,
}

impl ConnectionLimiter {
    pub fn new(limits: &crate::config::Limits) -> Self {
        Self {
            offers: Bucket::per_minute(limits.offers_per_minute),
            chat: Bucket::per_minute(limits.chat_per_minute),
            frames: Bucket::per_second(limits.frames_per_second),
        }
    }
}

pub fn shut_up(delay: Duration, scope: ShutUpScope, kind: Option<&str>) -> ShutUp {
    ShutUp {
        retry_after_ms: (delay.as_millis() as u64).max(1),
        scope,
        kind: kind.map(str::to_string),
    }
}

/// Tracks a `SHUT_UP` this node received, so it stops initiating what it was told to stop.
#[derive(Debug, Default)]
pub struct Restraint {
    connection_until: Option<Instant>,
    kinds: Vec<(String, Instant)>,
}

impl Restraint {
    pub fn apply(&mut self, shut_up: &ShutUp) {
        let until = Instant::now() + Duration::from_millis(shut_up.retry_after_ms());
        match shut_up.scope {
            ShutUpScope::Connection | ShutUpScope::Stream => {
                self.connection_until = Some(until);
            }
            ShutUpScope::Kind => {
                let kind = shut_up.kind.clone().unwrap_or_default();
                self.kinds.retain(|(name, _)| name != &kind);
                self.kinds.push((kind, until));
            }
        }
    }

    pub fn blocked(&self, kind: &str) -> Option<Duration> {
        let now = Instant::now();
        let connection = self
            .connection_until
            .filter(|until| *until > now)
            .map(|until| until - now);
        let per_kind = self
            .kinds
            .iter()
            .find(|(name, until)| name == kind && *until > now)
            .map(|(_, until)| *until - now);
        connection.into_iter().chain(per_kind).max()
    }
}
