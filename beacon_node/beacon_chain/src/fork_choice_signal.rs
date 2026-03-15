//! Concurrency helpers for synchronising block proposal with fork choice.
//!
//! The transmitter provides a way for a thread runnning fork choice on a schedule to signal
//! to the receiver that fork choice has been updated for a given slot.
use crate::BeaconChainError;
use parking_lot::{Condvar, Mutex};
use std::sync::Arc;
use std::time::Duration;
use types::Slot;

/// Sender, for use by the per-slot task timer.
pub struct ForkChoiceSignalTx {
    pair: Arc<(Mutex<Slot>, Condvar)>,
}

/// Receiver, for use by the beacon chain waiting on fork choice to complete.
pub struct ForkChoiceSignalRx {
    pair: Arc<(Mutex<Slot>, Condvar)>,
}

pub enum ForkChoiceWaitResult {
    /// Successfully reached a slot greater than or equal to the awaited slot.
    Success(Slot),
    /// Fork choice was updated to a lower slot, indicative of lag or processing delays.
    Behind(Slot),
    /// Timed out waiting for the fork choice update from the sender.
    TimeOut,
}

impl ForkChoiceSignalTx {
    pub fn new() -> Self {
        let pair = Arc::new((Mutex::new(Slot::new(0)), Condvar::new()));
        Self { pair }
    }

    pub fn get_receiver(&self) -> ForkChoiceSignalRx {
        ForkChoiceSignalRx {
            pair: self.pair.clone(),
        }
    }

    /// Signal to the receiver that fork choice has been updated to `slot`.
    ///
    /// Return an error if the provided `slot` is strictly less than any previously provided slot.
    pub fn notify_fork_choice_complete(&self, slot: Slot) -> Result<(), BeaconChainError> {
        let (lock, condvar) = &*self.pair;

        let mut current_slot = lock.lock();

        if slot < *current_slot {
            return Err(BeaconChainError::ForkChoiceSignalOutOfOrder {
                current: *current_slot,
                latest: slot,
            });
        } else {
            *current_slot = slot;
        }

        // We use `notify_all` because there may be multiple block proposals waiting simultaneously.
        // Usually there'll be 0-1.
        condvar.notify_all();

        Ok(())
    }
}

impl Default for ForkChoiceSignalTx {
    fn default() -> Self {
        Self::new()
    }
}

impl ForkChoiceSignalRx {
    pub fn wait_for_fork_choice(&self, slot: Slot, timeout: Duration) -> ForkChoiceWaitResult {
        let (lock, condvar) = &*self.pair;

        let mut current_slot = lock.lock();

        // Wait for `current_slot >= slot`.
        //
        // Do not loop and wait, if we receive an update for the wrong slot then something is
        // quite out of whack and we shouldn't waste more time waiting.
        if *current_slot < slot {
            let timeout_result = condvar.wait_for(&mut current_slot, timeout);

            if timeout_result.timed_out() {
                return ForkChoiceWaitResult::TimeOut;
            }
        }

        if *current_slot >= slot {
            ForkChoiceWaitResult::Success(*current_slot)
        } else {
            ForkChoiceWaitResult::Behind(*current_slot)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn notify_and_wait_success() {
        let tx = ForkChoiceSignalTx::new();
        let rx = tx.get_receiver();

        tx.notify_fork_choice_complete(Slot::new(5)).unwrap();

        match rx.wait_for_fork_choice(Slot::new(5), Duration::from_millis(100)) {
            ForkChoiceWaitResult::Success(slot) => assert_eq!(slot, Slot::new(5)),
            other => panic!("expected Success, got {:?}", result_name(&other)),
        }
    }

    #[test]
    fn wait_already_ahead() {
        let tx = ForkChoiceSignalTx::new();
        let rx = tx.get_receiver();

        tx.notify_fork_choice_complete(Slot::new(10)).unwrap();

        // Wait for slot 5 when already at 10 — should succeed immediately.
        match rx.wait_for_fork_choice(Slot::new(5), Duration::from_millis(100)) {
            ForkChoiceWaitResult::Success(slot) => assert_eq!(slot, Slot::new(10)),
            other => panic!("expected Success, got {:?}", result_name(&other)),
        }
    }

    #[test]
    fn wait_times_out_when_no_signal() {
        let tx = ForkChoiceSignalTx::new();
        let rx = tx.get_receiver();

        // Slot 0 (initial) < slot 5, no signal sent → timeout.
        let _ = &tx; // keep tx alive
        match rx.wait_for_fork_choice(Slot::new(5), Duration::from_millis(50)) {
            ForkChoiceWaitResult::TimeOut => {}
            other => panic!("expected TimeOut, got {:?}", result_name(&other)),
        }
    }

    #[test]
    fn notify_out_of_order_returns_error() {
        let tx = ForkChoiceSignalTx::new();

        tx.notify_fork_choice_complete(Slot::new(10)).unwrap();

        let result = tx.notify_fork_choice_complete(Slot::new(5));
        assert!(result.is_err(), "out-of-order slot should error");
    }

    #[test]
    fn notify_same_slot_is_ok() {
        let tx = ForkChoiceSignalTx::new();

        tx.notify_fork_choice_complete(Slot::new(5)).unwrap();
        // Same slot again is not strictly less, so should succeed.
        tx.notify_fork_choice_complete(Slot::new(5)).unwrap();
    }

    #[test]
    fn notify_monotonically_increasing() {
        let tx = ForkChoiceSignalTx::new();

        for i in 0..10 {
            tx.notify_fork_choice_complete(Slot::new(i)).unwrap();
        }
    }

    #[test]
    fn concurrent_notify_then_wait() {
        let tx = ForkChoiceSignalTx::new();
        let rx = tx.get_receiver();

        let handle = thread::spawn(move || {
            thread::sleep(Duration::from_millis(20));
            tx.notify_fork_choice_complete(Slot::new(7)).unwrap();
        });

        match rx.wait_for_fork_choice(Slot::new(7), Duration::from_secs(2)) {
            ForkChoiceWaitResult::Success(slot) => assert_eq!(slot, Slot::new(7)),
            other => panic!("expected Success, got {:?}", result_name(&other)),
        }

        handle.join().unwrap();
    }

    #[test]
    fn behind_when_signaled_lower_slot() {
        let tx = ForkChoiceSignalTx::new();
        let rx = tx.get_receiver();

        // Receiver wants slot 10, but sender only notifies slot 3.
        let handle = thread::spawn(move || {
            thread::sleep(Duration::from_millis(20));
            tx.notify_fork_choice_complete(Slot::new(3)).unwrap();
        });

        match rx.wait_for_fork_choice(Slot::new(10), Duration::from_secs(2)) {
            ForkChoiceWaitResult::Behind(slot) => assert_eq!(slot, Slot::new(3)),
            other => panic!("expected Behind, got {:?}", result_name(&other)),
        }

        handle.join().unwrap();
    }

    #[test]
    fn multiple_receivers_all_wake() {
        let tx = ForkChoiceSignalTx::new();
        let rx1 = tx.get_receiver();
        let rx2 = tx.get_receiver();

        let handle = thread::spawn(move || {
            thread::sleep(Duration::from_millis(20));
            tx.notify_fork_choice_complete(Slot::new(5)).unwrap();
        });

        let h1 =
            thread::spawn(move || rx1.wait_for_fork_choice(Slot::new(5), Duration::from_secs(2)));
        let h2 =
            thread::spawn(move || rx2.wait_for_fork_choice(Slot::new(5), Duration::from_secs(2)));

        match h1.join().unwrap() {
            ForkChoiceWaitResult::Success(s) => assert_eq!(s, Slot::new(5)),
            other => panic!("rx1: expected Success, got {:?}", result_name(&other)),
        }
        match h2.join().unwrap() {
            ForkChoiceWaitResult::Success(s) => assert_eq!(s, Slot::new(5)),
            other => panic!("rx2: expected Success, got {:?}", result_name(&other)),
        }

        handle.join().unwrap();
    }

    #[test]
    fn default_tx_starts_at_slot_zero() {
        let tx = ForkChoiceSignalTx::default();
        let rx = tx.get_receiver();

        // Slot 0 is already reached, so waiting for slot 0 succeeds immediately.
        match rx.wait_for_fork_choice(Slot::new(0), Duration::from_millis(50)) {
            ForkChoiceWaitResult::Success(slot) => assert_eq!(slot, Slot::new(0)),
            other => panic!("expected Success, got {:?}", result_name(&other)),
        }
    }

    fn result_name(r: &ForkChoiceWaitResult) -> &'static str {
        match r {
            ForkChoiceWaitResult::Success(_) => "Success",
            ForkChoiceWaitResult::Behind(_) => "Behind",
            ForkChoiceWaitResult::TimeOut => "TimeOut",
        }
    }
}
