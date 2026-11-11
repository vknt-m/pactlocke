use std::sync::{Arc, Mutex};

use crate::Frame;

/// A frame plus its sequence number (the count of frames run when it was
/// published), so a reader can tell whether it has already shown it.
#[derive(Debug)]
pub struct PublishedFrame {
    pub seq: u64,
    pub frame: Frame,
}

/// A single slot holding only the newest published frame. The emulator thread
/// overwrites it every frame; the UI reads whatever is newest at each refresh.
#[derive(Default)]
pub struct FrameSlot {
    newest: Mutex<Option<Arc<PublishedFrame>>>,
}

impl FrameSlot {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn publish(&self, seq: u64, frame: Frame) {
        let published = Arc::new(PublishedFrame { seq, frame });
        *self.newest.lock().unwrap_or_else(|e| e.into_inner()) = Some(published);
    }

    pub fn latest(&self) -> Option<Arc<PublishedFrame>> {
        self.newest.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}
