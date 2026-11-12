use std::time::{SystemTime, UNIX_EPOCH};

/// Wall-clock time for the Run: when events happen, and when the input
/// stream is flushed.
pub trait RunClock: Send {
    /// Unix time in milliseconds (UTC).
    fn now_ms(&self) -> i64;
}

/// The real clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemRunClock;

impl RunClock for SystemRunClock {
    fn now_ms(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64)
    }
}
