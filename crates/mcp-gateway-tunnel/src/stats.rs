use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

#[derive(Debug, Default)]
pub struct Stats {
    pub inflight: AtomicU32,
    pub total: AtomicU64,
    pub reconnects: AtomicU64,
}

impl Stats {
    pub(crate) fn begin_request(&self, max_inflight: u32) -> bool {
        let prev = self.inflight.fetch_add(1, Ordering::SeqCst);
        if prev >= max_inflight {
            self.inflight.fetch_sub(1, Ordering::SeqCst);
            false
        } else {
            true
        }
    }

    pub(crate) fn end_request(&self) {
        self.inflight.fetch_sub(1, Ordering::SeqCst);
        self.total.fetch_add(1, Ordering::SeqCst);
    }
}
