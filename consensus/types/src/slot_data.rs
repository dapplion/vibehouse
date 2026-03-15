use crate::Slot;

/// A trait providing a `Slot` getter for messages that are related to a single slot. Useful in
/// making parts of attestation and sync committee processing generic.
pub trait SlotData {
    fn get_slot(&self) -> Slot;
}

impl SlotData for Slot {
    fn get_slot(&self) -> Slot {
        *self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_returns_self() {
        let slot = Slot::new(42);
        assert_eq!(slot.get_slot(), Slot::new(42));
    }

    #[test]
    fn slot_zero() {
        let slot = Slot::new(0);
        assert_eq!(slot.get_slot(), slot);
    }

    #[test]
    fn slot_large_value() {
        let slot = Slot::new(u64::MAX);
        assert_eq!(slot.get_slot(), Slot::new(u64::MAX));
    }
}
