use super::SlotClock;
use parking_lot::RwLock;
use std::ops::Add;
use std::sync::Arc;
use std::time::Duration;
use types::Slot;

/// Determines the present slot based upon a manually-incremented UNIX timestamp.
pub struct ManualSlotClock {
    genesis_slot: Slot,
    /// Duration from UNIX epoch to genesis.
    genesis_duration: Duration,
    /// Duration from UNIX epoch to right now.
    current_time: Arc<RwLock<Duration>>,
    /// The length of each slot.
    slot_duration: Duration,
    /// The slot at which the Gloas fork activates.
    /// When set, slot timing uses 4 intervals per slot instead of 3.
    gloas_fork_slot: Arc<RwLock<Option<Slot>>>,
}

impl Clone for ManualSlotClock {
    fn clone(&self) -> Self {
        ManualSlotClock {
            genesis_slot: self.genesis_slot,
            genesis_duration: self.genesis_duration,
            current_time: Arc::clone(&self.current_time),
            slot_duration: self.slot_duration,
            gloas_fork_slot: Arc::clone(&self.gloas_fork_slot),
        }
    }
}

impl ManualSlotClock {
    pub fn set_slot(&self, slot: u64) {
        let slots_since_genesis = slot
            .checked_sub(self.genesis_slot.as_u64())
            .expect("slot must be post-genesis")
            .try_into()
            .expect("slot must fit within a u32");
        *self.current_time.write() =
            self.genesis_duration + self.slot_duration * slots_since_genesis;
    }

    pub fn set_current_time(&self, duration: Duration) {
        *self.current_time.write() = duration;
    }

    pub fn advance_time(&self, duration: Duration) {
        let current_time = *self.current_time.read();
        *self.current_time.write() = current_time.add(duration);
    }

    pub fn advance_slot(&self) {
        self.set_slot(self.now().unwrap().as_u64() + 1)
    }

    pub fn genesis_duration(&self) -> &Duration {
        &self.genesis_duration
    }

    /// Returns the duration from `now` until the start of `slot`.
    ///
    /// Will return `None` if `now` is later than the start of `slot`.
    pub fn duration_to_slot(&self, slot: Slot, now: Duration) -> Option<Duration> {
        self.start_of(slot)?.checked_sub(now)
    }

    /// Returns the duration between `now` and the start of the next slot.
    pub fn duration_to_next_slot_from(&self, now: Duration) -> Option<Duration> {
        if now < self.genesis_duration {
            self.genesis_duration.checked_sub(now)
        } else {
            self.duration_to_slot(self.slot_of(now)? + 1, now)
        }
    }

    /// Returns the duration between `now` and the start of the next epoch.
    pub fn duration_to_next_epoch_from(
        &self,
        now: Duration,
        slots_per_epoch: u64,
    ) -> Option<Duration> {
        if now < self.genesis_duration {
            self.genesis_duration.checked_sub(now)
        } else {
            let next_epoch_start_slot =
                (self.slot_of(now)?.epoch(slots_per_epoch) + 1).start_slot(slots_per_epoch);

            self.duration_to_slot(next_epoch_start_slot, now)
        }
    }
}

impl SlotClock for ManualSlotClock {
    fn new(genesis_slot: Slot, genesis_duration: Duration, slot_duration: Duration) -> Self {
        if slot_duration.as_millis() == 0 {
            panic!("ManualSlotClock cannot have a < 1ms slot duration");
        }

        Self {
            genesis_slot,
            current_time: Arc::new(RwLock::new(genesis_duration)),
            genesis_duration,
            slot_duration,
            gloas_fork_slot: Arc::new(RwLock::new(None)),
        }
    }

    fn now(&self) -> Option<Slot> {
        self.slot_of(*self.current_time.read())
    }

    fn is_prior_to_genesis(&self) -> Option<bool> {
        Some(*self.current_time.read() < self.genesis_duration)
    }

    fn now_duration(&self) -> Option<Duration> {
        Some(*self.current_time.read())
    }

    fn slot_of(&self, now: Duration) -> Option<Slot> {
        let genesis = self.genesis_duration;

        if now >= genesis {
            let since_genesis = now
                .checked_sub(genesis)
                .expect("Control flow ensures now is greater than or equal to genesis");
            let slot =
                Slot::from((since_genesis.as_millis() / self.slot_duration.as_millis()) as u64);
            Some(slot + self.genesis_slot)
        } else {
            None
        }
    }

    fn duration_to_next_slot(&self) -> Option<Duration> {
        self.duration_to_next_slot_from(*self.current_time.read())
    }

    fn duration_to_next_epoch(&self, slots_per_epoch: u64) -> Option<Duration> {
        self.duration_to_next_epoch_from(*self.current_time.read(), slots_per_epoch)
    }

    fn slot_duration(&self) -> Duration {
        self.slot_duration
    }

    fn duration_to_slot(&self, slot: Slot) -> Option<Duration> {
        self.duration_to_slot(slot, *self.current_time.read())
    }

    /// Returns the duration between UNIX epoch and the start of `slot`.
    fn start_of(&self, slot: Slot) -> Option<Duration> {
        let slot = slot
            .as_u64()
            .checked_sub(self.genesis_slot.as_u64())?
            .try_into()
            .ok()?;
        let unadjusted_slot_duration = self.slot_duration.checked_mul(slot)?;

        self.genesis_duration.checked_add(unadjusted_slot_duration)
    }

    fn genesis_slot(&self) -> Slot {
        self.genesis_slot
    }

    fn genesis_duration(&self) -> Duration {
        self.genesis_duration
    }

    fn gloas_fork_slot(&self) -> Option<Slot> {
        *self.gloas_fork_slot.read()
    }

    fn set_gloas_fork_slot(&self, slot: Option<Slot>) {
        *self.gloas_fork_slot.write() = slot;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slot_now() {
        let clock = ManualSlotClock::new(
            Slot::new(10),
            Duration::from_secs(0),
            Duration::from_secs(1),
        );
        assert_eq!(clock.now(), Some(Slot::new(10)));
        clock.set_slot(123);
        assert_eq!(clock.now(), Some(Slot::new(123)));
    }

    #[test]
    fn test_is_prior_to_genesis() {
        let genesis_secs = 1;

        let clock = ManualSlotClock::new(
            Slot::new(0),
            Duration::from_secs(genesis_secs),
            Duration::from_secs(1),
        );

        *clock.current_time.write() = Duration::from_secs(genesis_secs - 1);
        assert!(clock.is_prior_to_genesis().unwrap(), "prior to genesis");

        *clock.current_time.write() = Duration::from_secs(genesis_secs);
        assert!(!clock.is_prior_to_genesis().unwrap(), "at genesis");

        *clock.current_time.write() = Duration::from_secs(genesis_secs + 1);
        assert!(!clock.is_prior_to_genesis().unwrap(), "after genesis");
    }

    #[test]
    fn start_of() {
        // Genesis slot and genesis duration 0.
        let clock =
            ManualSlotClock::new(Slot::new(0), Duration::from_secs(0), Duration::from_secs(1));
        assert_eq!(clock.start_of(Slot::new(0)), Some(Duration::from_secs(0)));
        assert_eq!(clock.start_of(Slot::new(1)), Some(Duration::from_secs(1)));
        assert_eq!(clock.start_of(Slot::new(2)), Some(Duration::from_secs(2)));

        // Genesis slot 1 and genesis duration 10.
        let clock = ManualSlotClock::new(
            Slot::new(0),
            Duration::from_secs(10),
            Duration::from_secs(1),
        );
        assert_eq!(clock.start_of(Slot::new(0)), Some(Duration::from_secs(10)));
        assert_eq!(clock.start_of(Slot::new(1)), Some(Duration::from_secs(11)));
        assert_eq!(clock.start_of(Slot::new(2)), Some(Duration::from_secs(12)));

        // Genesis slot 1 and genesis duration 0.
        let clock =
            ManualSlotClock::new(Slot::new(1), Duration::from_secs(0), Duration::from_secs(1));
        assert_eq!(clock.start_of(Slot::new(0)), None);
        assert_eq!(clock.start_of(Slot::new(1)), Some(Duration::from_secs(0)));
        assert_eq!(clock.start_of(Slot::new(2)), Some(Duration::from_secs(1)));

        // Genesis slot 1 and genesis duration 10.
        let clock = ManualSlotClock::new(
            Slot::new(1),
            Duration::from_secs(10),
            Duration::from_secs(1),
        );
        assert_eq!(clock.start_of(Slot::new(0)), None);
        assert_eq!(clock.start_of(Slot::new(1)), Some(Duration::from_secs(10)));
        assert_eq!(clock.start_of(Slot::new(2)), Some(Duration::from_secs(11)));
    }

    #[test]
    fn test_duration_to_next_slot() {
        let slot_duration = Duration::from_secs(1);

        // Genesis time is now.
        let clock = ManualSlotClock::new(Slot::new(0), Duration::from_secs(0), slot_duration);
        *clock.current_time.write() = Duration::from_secs(0);
        assert_eq!(clock.duration_to_next_slot(), Some(Duration::from_secs(1)));

        // Genesis time is in the future.
        let clock = ManualSlotClock::new(Slot::new(0), Duration::from_secs(10), slot_duration);
        *clock.current_time.write() = Duration::from_secs(0);
        assert_eq!(clock.duration_to_next_slot(), Some(Duration::from_secs(10)));

        // Genesis time is in the past.
        let clock = ManualSlotClock::new(Slot::new(0), Duration::from_secs(0), slot_duration);
        *clock.current_time.write() = Duration::from_secs(10);
        assert_eq!(clock.duration_to_next_slot(), Some(Duration::from_secs(1)));
    }

    #[test]
    fn test_duration_to_next_epoch() {
        let slot_duration = Duration::from_secs(1);
        let slots_per_epoch = 32;

        // Genesis time is now.
        let clock = ManualSlotClock::new(Slot::new(0), Duration::from_secs(0), slot_duration);
        *clock.current_time.write() = Duration::from_secs(0);
        assert_eq!(
            clock.duration_to_next_epoch(slots_per_epoch),
            Some(Duration::from_secs(32))
        );

        // Genesis time is in the future.
        let clock = ManualSlotClock::new(Slot::new(0), Duration::from_secs(10), slot_duration);
        *clock.current_time.write() = Duration::from_secs(0);
        assert_eq!(
            clock.duration_to_next_epoch(slots_per_epoch),
            Some(Duration::from_secs(10))
        );

        // Genesis time is in the past.
        let clock = ManualSlotClock::new(Slot::new(0), Duration::from_secs(0), slot_duration);
        *clock.current_time.write() = Duration::from_secs(10);
        assert_eq!(
            clock.duration_to_next_epoch(slots_per_epoch),
            Some(Duration::from_secs(22))
        );

        // Genesis time is in the past.
        let clock = ManualSlotClock::new(
            Slot::new(0),
            Duration::from_secs(0),
            Duration::from_secs(12),
        );
        *clock.current_time.write() = Duration::from_secs(72_333);
        assert!(clock.duration_to_next_epoch(slots_per_epoch).is_some(),);
    }

    // ── Gloas 4-interval slot timing tests ─────────────────────

    #[test]
    fn gloas_fork_slot_round_trip() {
        let clock = ManualSlotClock::new(
            Slot::new(0),
            Duration::from_secs(0),
            Duration::from_secs(12),
        );
        assert_eq!(clock.gloas_fork_slot(), None, "default is None");

        clock.set_gloas_fork_slot(Some(Slot::new(32)));
        assert_eq!(clock.gloas_fork_slot(), Some(Slot::new(32)));

        clock.set_gloas_fork_slot(None);
        assert_eq!(clock.gloas_fork_slot(), None, "can be unset");
    }

    #[test]
    fn current_intervals_pre_gloas_is_3() {
        let clock = ManualSlotClock::new(
            Slot::new(0),
            Duration::from_secs(0),
            Duration::from_secs(12),
        );
        // No Gloas fork configured — always 3.
        assert_eq!(clock.current_intervals_per_slot(), 3);

        // Gloas fork at slot 100, current slot = 0 → pre-Gloas → 3.
        clock.set_gloas_fork_slot(Some(Slot::new(100)));
        assert_eq!(
            clock.current_intervals_per_slot(),
            3,
            "before fork slot should use 3 intervals"
        );
    }

    #[test]
    fn current_intervals_at_gloas_fork_is_4() {
        let clock = ManualSlotClock::new(
            Slot::new(0),
            Duration::from_secs(0),
            Duration::from_secs(12),
        );
        clock.set_gloas_fork_slot(Some(Slot::new(10)));
        clock.set_slot(10); // exactly at fork slot
        assert_eq!(
            clock.current_intervals_per_slot(),
            4,
            "at fork slot should use 4 intervals"
        );
    }

    #[test]
    fn current_intervals_after_gloas_fork_is_4() {
        let clock = ManualSlotClock::new(
            Slot::new(0),
            Duration::from_secs(0),
            Duration::from_secs(12),
        );
        clock.set_gloas_fork_slot(Some(Slot::new(10)));
        clock.set_slot(15); // well after fork
        assert_eq!(
            clock.current_intervals_per_slot(),
            4,
            "after fork slot should use 4 intervals"
        );
    }

    #[test]
    fn current_intervals_one_before_gloas_fork_is_3() {
        let clock = ManualSlotClock::new(
            Slot::new(0),
            Duration::from_secs(0),
            Duration::from_secs(12),
        );
        clock.set_gloas_fork_slot(Some(Slot::new(10)));
        clock.set_slot(9); // one slot before fork
        assert_eq!(
            clock.current_intervals_per_slot(),
            3,
            "one slot before fork should still use 3 intervals"
        );
    }

    #[test]
    fn unagg_attestation_delay_pre_gloas() {
        let slot_duration = Duration::from_secs(12);
        let clock = ManualSlotClock::new(Slot::new(0), Duration::from_secs(0), slot_duration);
        // No Gloas → 3 intervals → 12/3 = 4s
        assert_eq!(
            clock.unagg_attestation_production_delay(),
            Duration::from_secs(4),
            "pre-Gloas: unagg delay = slot_duration / 3"
        );
    }

    #[test]
    fn unagg_attestation_delay_post_gloas() {
        let slot_duration = Duration::from_secs(12);
        let clock = ManualSlotClock::new(Slot::new(0), Duration::from_secs(0), slot_duration);
        clock.set_gloas_fork_slot(Some(Slot::new(0)));
        // At Gloas → 4 intervals → 12/4 = 3s
        assert_eq!(
            clock.unagg_attestation_production_delay(),
            Duration::from_secs(3),
            "post-Gloas: unagg delay = slot_duration / 4"
        );
    }

    #[test]
    fn agg_attestation_delay_pre_gloas() {
        let slot_duration = Duration::from_secs(12);
        let clock = ManualSlotClock::new(Slot::new(0), Duration::from_secs(0), slot_duration);
        // No Gloas → 3 intervals → 2*12/3 = 8s
        assert_eq!(
            clock.agg_attestation_production_delay(),
            Duration::from_secs(8),
            "pre-Gloas: agg delay = 2 * slot_duration / 3"
        );
    }

    #[test]
    fn agg_attestation_delay_post_gloas() {
        let slot_duration = Duration::from_secs(12);
        let clock = ManualSlotClock::new(Slot::new(0), Duration::from_secs(0), slot_duration);
        clock.set_gloas_fork_slot(Some(Slot::new(0)));
        // At Gloas → 4 intervals → 2*12/4 = 6s
        assert_eq!(
            clock.agg_attestation_production_delay(),
            Duration::from_secs(6),
            "post-Gloas: agg delay = 2 * slot_duration / 4"
        );
    }

    #[test]
    fn sync_committee_delays_mirror_attestation_delays() {
        let slot_duration = Duration::from_secs(12);
        let clock = ManualSlotClock::new(Slot::new(0), Duration::from_secs(0), slot_duration);

        // Pre-Gloas
        assert_eq!(
            clock.sync_committee_message_production_delay(),
            clock.unagg_attestation_production_delay(),
            "sync message delay should match unagg delay"
        );
        assert_eq!(
            clock.sync_committee_contribution_production_delay(),
            clock.agg_attestation_production_delay(),
            "sync contribution delay should match agg delay"
        );

        // Post-Gloas
        clock.set_gloas_fork_slot(Some(Slot::new(0)));
        assert_eq!(
            clock.sync_committee_message_production_delay(),
            clock.unagg_attestation_production_delay(),
            "post-Gloas: sync message delay should match unagg delay"
        );
        assert_eq!(
            clock.sync_committee_contribution_production_delay(),
            clock.agg_attestation_production_delay(),
            "post-Gloas: sync contribution delay should match agg delay"
        );
    }

    #[test]
    fn single_lookup_delay_changes_with_gloas() {
        let slot_duration = Duration::from_secs(12);
        let clock = ManualSlotClock::new(Slot::new(0), Duration::from_secs(0), slot_duration);

        // Pre-Gloas: single_lookup = unagg_delay / 2 = 4/2 = 2s
        assert_eq!(
            clock.single_lookup_delay(),
            Duration::from_secs(2),
            "pre-Gloas: single lookup delay = unagg_delay / 2"
        );

        // Post-Gloas: single_lookup = unagg_delay / 2 = 3/2 = 1.5s
        clock.set_gloas_fork_slot(Some(Slot::new(0)));
        assert_eq!(
            clock.single_lookup_delay(),
            Duration::from_millis(1500),
            "post-Gloas: single lookup delay = unagg_delay / 2"
        );
    }

    #[test]
    fn freeze_at_preserves_gloas_fork_slot() {
        let slot_duration = Duration::from_secs(12);
        let clock = ManualSlotClock::new(Slot::new(0), Duration::from_secs(0), slot_duration);
        clock.set_gloas_fork_slot(Some(Slot::new(5)));
        clock.set_slot(10); // post-Gloas

        // Freeze the clock at a specific time (slot 10 start)
        let frozen = clock.freeze_at(Duration::from_secs(120));
        assert_eq!(
            frozen.gloas_fork_slot(),
            Some(Slot::new(5)),
            "freeze_at should preserve gloas_fork_slot"
        );
        assert_eq!(
            frozen.current_intervals_per_slot(),
            4,
            "frozen clock should use Gloas intervals"
        );
    }

    #[test]
    fn timing_transition_at_fork_boundary() {
        // Simulate crossing the Gloas fork boundary.
        let slot_duration = Duration::from_secs(12);
        let clock = ManualSlotClock::new(Slot::new(0), Duration::from_secs(0), slot_duration);
        clock.set_gloas_fork_slot(Some(Slot::new(5)));

        // Pre-fork (slot 4)
        clock.set_slot(4);
        assert_eq!(clock.current_intervals_per_slot(), 3);
        assert_eq!(
            clock.unagg_attestation_production_delay(),
            Duration::from_secs(4)
        );
        assert_eq!(
            clock.agg_attestation_production_delay(),
            Duration::from_secs(8)
        );

        // At fork (slot 5)
        clock.set_slot(5);
        assert_eq!(clock.current_intervals_per_slot(), 4);
        assert_eq!(
            clock.unagg_attestation_production_delay(),
            Duration::from_secs(3)
        );
        assert_eq!(
            clock.agg_attestation_production_delay(),
            Duration::from_secs(6)
        );

        // Post fork (slot 6)
        clock.set_slot(6);
        assert_eq!(clock.current_intervals_per_slot(), 4);
        assert_eq!(
            clock.unagg_attestation_production_delay(),
            Duration::from_secs(3)
        );
        assert_eq!(
            clock.agg_attestation_production_delay(),
            Duration::from_secs(6)
        );
    }

    #[test]
    fn gloas_fork_at_genesis() {
        // Gloas from genesis (slot 0).
        let slot_duration = Duration::from_secs(12);
        let clock = ManualSlotClock::new(Slot::new(0), Duration::from_secs(0), slot_duration);
        clock.set_gloas_fork_slot(Some(Slot::new(0)));

        // At genesis (slot 0) → should already be Gloas.
        assert_eq!(clock.current_intervals_per_slot(), 4);
        assert_eq!(
            clock.unagg_attestation_production_delay(),
            Duration::from_secs(3)
        );
    }

    #[test]
    fn test_tolerance() {
        let clock = ManualSlotClock::new(
            Slot::new(0),
            Duration::from_secs(10),
            Duration::from_secs(1),
        );

        // Set clock to the 0'th slot.
        *clock.current_time.write() = Duration::from_secs(10);
        assert_eq!(
            clock
                .now_with_future_tolerance(Duration::from_secs(0))
                .unwrap(),
            Slot::new(0),
            "future tolerance of zero should return current slot"
        );
        assert_eq!(
            clock
                .now_with_past_tolerance(Duration::from_secs(0))
                .unwrap(),
            Slot::new(0),
            "past tolerance of zero should return current slot"
        );
        assert_eq!(
            clock
                .now_with_future_tolerance(Duration::from_millis(10))
                .unwrap(),
            Slot::new(0),
            "insignificant future tolerance should return current slot"
        );
        assert_eq!(
            clock
                .now_with_past_tolerance(Duration::from_millis(10))
                .unwrap(),
            Slot::new(0),
            "past tolerance that precedes genesis should return genesis slot"
        );

        // Set clock to part-way through the 1st slot.
        *clock.current_time.write() = Duration::from_millis(11_200);
        assert_eq!(
            clock
                .now_with_future_tolerance(Duration::from_secs(0))
                .unwrap(),
            Slot::new(1),
            "future tolerance of zero should return current slot"
        );
        assert_eq!(
            clock
                .now_with_past_tolerance(Duration::from_secs(0))
                .unwrap(),
            Slot::new(1),
            "past tolerance of zero should return current slot"
        );
        assert_eq!(
            clock
                .now_with_future_tolerance(Duration::from_millis(800))
                .unwrap(),
            Slot::new(2),
            "significant future tolerance should return next slot"
        );
        assert_eq!(
            clock
                .now_with_past_tolerance(Duration::from_millis(201))
                .unwrap(),
            Slot::new(0),
            "significant past tolerance should return previous slot"
        );
    }
}
