//! Writes buffered adjustments (slider moves, hotkey steps) when their quiet
//! period ends (SPEC-WR-2), on a thread of its own, so committing never
//! depends on a window's timer and never runs on the UI thread.

use std::sync::{Arc, Condvar, Mutex, Weak};
use std::thread;
use std::time::Duration;

use crate::{ControlApi, MonitorService, UseCaseError};

/// Told about each commit, e.g. to show it or an error in the UI.
pub trait CommitObserver: Send + Sync {
    /// `Ok(n)` after `n` buffered values were written (n > 0), or the error
    /// that stopped a write.
    fn committed(&self, result: Result<usize, UseCaseError>);
}

/// Wakes the committer when a new value is buffered.
#[derive(Default)]
pub(crate) struct Wake {
    pending: Mutex<bool>,
    signal: Condvar,
}

impl Wake {
    pub(crate) fn notify(&self) {
        *self.lock() = true;
        self.signal.notify_one();
    }

    /// Waits for `notify` or until `timeout` (forever with `None`).
    fn wait(&self, timeout: Option<Duration>) {
        let mut pending = self.lock();
        if !*pending {
            pending = match timeout {
                Some(timeout) => {
                    self.signal
                        .wait_timeout(pending, timeout)
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .0
                }
                None => self
                    .signal
                    .wait(pending)
                    .unwrap_or_else(|poisoned| poisoned.into_inner()),
            };
        }
        *pending = false;
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, bool> {
        self.pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl MonitorService {
    /// Starts the committer thread. It ends when the service is dropped.
    pub fn start_committer(
        self: &Arc<Self>,
        observer: Arc<dyn CommitObserver>,
    ) -> std::io::Result<()> {
        let service: Weak<Self> = Arc::downgrade(self);
        let wake = self.gate.wake.clone();
        thread::Builder::new()
            .name("dusk-committer".into())
            .spawn(move || {
                loop {
                    let next = {
                        let Some(service) = service.upgrade() else {
                            return;
                        };
                        match service.flush_pending_adjustments() {
                            Ok((committed, next)) => {
                                if committed > 0 {
                                    observer.committed(Ok(committed));
                                }
                                next
                            }
                            // The failed value was dropped; the rest stay due.
                            Err(error) => {
                                log::warn!("a buffered monitor write failed: {error}");
                                observer.committed(Err(error));
                                Some(Duration::ZERO)
                            }
                        }
                    };
                    // Also wakes now and then to notice a dropped service.
                    wake.wait(Some(next.unwrap_or(Duration::from_secs(60))));
                }
            })?;
        Ok(())
    }
}
