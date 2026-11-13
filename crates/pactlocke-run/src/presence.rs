//! Who is online: every other Player's app is heard from all the time (their
//! input stream, their log entries). One silent for a few seconds is shown
//! offline; their game freezes on its last frame until they are back. After
//! 15 minutes the others are offered a Dropout vote, and may keep waiting.
//! The Host silent for about 10 seconds is replaced by the next Player in
//! join order.

use std::collections::BTreeMap;

use crate::stream::PlayerKey;

/// Silent this long, a Player is shown offline.
pub const OFFLINE_MS: i64 = 5_000;
/// Silent this long, the Host is taken over from.
pub const HOST_SILENT_MS: i64 = 10_000;
/// Offline this long, the others are offered a Dropout vote.
pub const DROPOUT_OFFER_MS: i64 = 15 * 60 * 1000;

/// When each other Player's app was last heard from.
#[derive(Debug, Clone, Default)]
pub struct Presence {
    last: BTreeMap<PlayerKey, i64>,
    since: i64,
}

impl Presence {
    /// Nobody heard yet; everyone counts from `now_ms` (the Run's start, or
    /// this app's).
    pub fn new(now_ms: i64) -> Self {
        Self {
            last: BTreeMap::new(),
            since: now_ms,
        }
    }

    /// Something arrived from this Player's app.
    pub fn heard(&mut self, player: PlayerKey, now_ms: i64) {
        self.last.insert(player, now_ms);
    }

    /// How long this Player has been silent.
    pub fn silent_for(&self, player: &PlayerKey, now_ms: i64) -> i64 {
        now_ms - self.last.get(player).copied().unwrap_or(self.since)
    }

    /// Whether this Player is shown offline.
    pub fn offline(&self, player: &PlayerKey, now_ms: i64) -> bool {
        self.silent_for(player, now_ms) >= OFFLINE_MS
    }

    /// Whether the Host has been silent long enough to take over from.
    pub fn host_gone(&self, host: &PlayerKey, now_ms: i64) -> bool {
        self.silent_for(host, now_ms) >= HOST_SILENT_MS
    }

    /// Whether the others may vote this Player out: offline 15 minutes.
    pub fn dropout_offered(&self, player: &PlayerKey, now_ms: i64) -> bool {
        self.silent_for(player, now_ms) >= DROPOUT_OFFER_MS
    }
}
