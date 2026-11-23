//! A one-shot gate for work that starts at launch and finishes in the background.
//!
//! Slow startup work (the keyring reads, the `ssh-agent` spawn) runs after the
//! window is shown. Code that needs the result calls [`StartupGate::wait`], which
//! returns at once when the work is done and otherwise blocks, bounded, until it
//! is. A worker thread may wait. The GTK main thread must not, because the work
//! may need that thread to finish.

use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

/// Holds a value that is set once, and lets threads wait for it.
#[derive(Debug)]
pub struct StartupGate<T> {
    value: Mutex<Option<T>>,
    opened: Condvar,
}

impl<T: Clone> StartupGate<T> {
    /// A gate that is still closed.
    #[must_use]
    pub const fn pending() -> Self {
        Self {
            value: Mutex::new(None),
            opened: Condvar::new(),
        }
    }

    /// A gate that is already open with `value`.
    #[must_use]
    pub const fn ready(value: T) -> Self {
        Self {
            value: Mutex::new(Some(value)),
            opened: Condvar::new(),
        }
    }

    /// Opens the gate and wakes every waiter. The first value wins.
    pub fn open(&self, value: T) {
        let mut slot = self
            .value
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if slot.is_none() {
            *slot = Some(value);
        }
        drop(slot);
        self.opened.notify_all();
    }

    /// The value if the gate is open, without waiting.
    #[must_use]
    pub fn get(&self) -> Option<T> {
        self.value
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Waits up to `timeout` for the gate to open. `None` means it did not.
    #[must_use]
    pub fn wait(&self, timeout: Duration) -> Option<T> {
        let deadline = Instant::now() + timeout;
        let mut slot = self
            .value
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while slot.is_none() {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return None;
            }
            slot = self
                .opened
                .wait_timeout(slot, left)
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
        slot.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn ready_gate_does_not_wait() {
        let gate = StartupGate::ready(7);
        assert_eq!(gate.wait(Duration::ZERO), Some(7));
    }

    #[test]
    fn pending_gate_times_out() {
        let gate = StartupGate::<u8>::pending();
        assert_eq!(gate.wait(Duration::from_millis(20)), None);
        assert_eq!(gate.get(), None);
    }

    #[test]
    fn open_wakes_a_waiting_thread_and_first_value_wins() {
        let gate = Arc::new(StartupGate::<u8>::pending());
        let waiter = {
            let gate = Arc::clone(&gate);
            std::thread::spawn(move || gate.wait(Duration::from_secs(5)))
        };
        std::thread::sleep(Duration::from_millis(20));
        gate.open(1);
        gate.open(2);
        assert_eq!(waiter.join().unwrap(), Some(1));
    }
}
