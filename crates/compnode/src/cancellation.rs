//! Cooperative cancellation for queued worker jobs.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Default)]
pub(crate) struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    pub(crate) fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_is_shared_by_clones() {
        let token = CancellationToken::default();
        let worker_token = token.clone();
        assert!(!worker_token.is_cancelled());
        token.cancel();
        assert!(worker_token.is_cancelled());
    }
}
