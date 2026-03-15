use crate::context_deserialize;
use crate::{
    Address, BeaconState, ChainSpec, Checkpoint, Epoch, EthSpec, FixedBytesExtended, ForkName,
    Hash256, PublicKeyBytes, test_utils::TestRandom,
};
use serde::{Deserialize, Serialize};
use ssz_derive::{Decode, Encode};
use test_random_derive::TestRandom;
use tree_hash_derive::TreeHash;

/// Information about a `BeaconChain` validator.
///
/// Spec v0.12.1
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[derive(
    Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Encode, Decode, TestRandom, TreeHash,
)]
#[context_deserialize(ForkName)]
pub struct Validator {
    pub pubkey: PublicKeyBytes,
    pub withdrawal_credentials: Hash256,
    #[serde(with = "serde_utils::quoted_u64")]
    pub effective_balance: u64,
    pub slashed: bool,
    pub activation_eligibility_epoch: Epoch,
    pub activation_epoch: Epoch,
    pub exit_epoch: Epoch,
    pub withdrawable_epoch: Epoch,
}

impl Validator {
    #[allow(clippy::arithmetic_side_effects)]
    pub fn from_deposit(
        pubkey: PublicKeyBytes,
        withdrawal_credentials: Hash256,
        amount: u64,
        fork_name: ForkName,
        spec: &ChainSpec,
    ) -> Self {
        let mut validator = Validator {
            pubkey,
            withdrawal_credentials,
            activation_eligibility_epoch: spec.far_future_epoch,
            activation_epoch: spec.far_future_epoch,
            exit_epoch: spec.far_future_epoch,
            withdrawable_epoch: spec.far_future_epoch,
            effective_balance: 0,
            slashed: false,
        };

        let max_effective_balance = validator.get_max_effective_balance(spec, fork_name);
        // safe math is unnecessary here since the spec.effective_balance_increment is never <= 0
        validator.effective_balance = std::cmp::min(
            amount - (amount % spec.effective_balance_increment),
            max_effective_balance,
        );

        validator
    }

    /// Returns `true` if the validator is considered active at some epoch.
    pub fn is_active_at(&self, epoch: Epoch) -> bool {
        self.activation_epoch <= epoch && epoch < self.exit_epoch
    }

    /// Returns `true` if the validator is slashable at some epoch.
    pub fn is_slashable_at(&self, epoch: Epoch) -> bool {
        !self.slashed && self.activation_epoch <= epoch && epoch < self.withdrawable_epoch
    }

    /// Returns `true` if the validator is considered exited at some epoch.
    pub fn is_exited_at(&self, epoch: Epoch) -> bool {
        self.exit_epoch <= epoch
    }

    /// Returns `true` if the validator is able to withdraw at some epoch.
    pub fn is_withdrawable_at(&self, epoch: Epoch) -> bool {
        epoch >= self.withdrawable_epoch
    }

    /// Returns `true` if the validator is eligible to join the activation queue.
    ///
    /// Calls the correct function depending on the provided `fork_name`.
    pub fn is_eligible_for_activation_queue(
        &self,
        spec: &ChainSpec,
        current_fork: ForkName,
    ) -> bool {
        if current_fork.electra_enabled() {
            self.is_eligible_for_activation_queue_electra(spec)
        } else {
            self.is_eligible_for_activation_queue_base(spec)
        }
    }

    /// Returns `true` if the validator is eligible to join the activation queue.
    ///
    /// Spec v0.12.1
    fn is_eligible_for_activation_queue_base(&self, spec: &ChainSpec) -> bool {
        self.activation_eligibility_epoch == spec.far_future_epoch
            && self.effective_balance == spec.max_effective_balance
    }

    /// Returns `true` if the validator is eligible to join the activation queue.
    ///
    /// Modified in electra as part of EIP 7251.
    fn is_eligible_for_activation_queue_electra(&self, spec: &ChainSpec) -> bool {
        self.activation_eligibility_epoch == spec.far_future_epoch
            && self.effective_balance >= spec.min_activation_balance
    }

    /// Returns `true` if the validator is eligible to be activated.
    pub fn is_eligible_for_activation<E: EthSpec>(
        &self,
        state: &BeaconState<E>,
        spec: &ChainSpec,
    ) -> bool {
        self.is_eligible_for_activation_with_finalized_checkpoint(
            &state.finalized_checkpoint(),
            spec,
        )
    }

    /// Returns `true` if the validator is eligible to be activated.
    pub fn is_eligible_for_activation_with_finalized_checkpoint(
        &self,
        finalized_checkpoint: &Checkpoint,
        spec: &ChainSpec,
    ) -> bool {
        // Placement in queue is finalized
        self.activation_eligibility_epoch <= finalized_checkpoint.epoch
        // Has not yet been activated
        && self.activation_epoch == spec.far_future_epoch
    }

    /// Returns `true` if the validator *could* be eligible for activation at `epoch`.
    ///
    /// Eligibility depends on finalization, so we assume best-possible finalization. This function
    /// returning true is a necessary but *not sufficient* condition for a validator to activate in
    /// the epoch transition at the end of `epoch`.
    pub fn could_be_eligible_for_activation_at(&self, epoch: Epoch, spec: &ChainSpec) -> bool {
        // Has not yet been activated
        self.activation_epoch == spec.far_future_epoch
        // Placement in queue could be finalized.
        //
        // NOTE: the epoch distance is 1 rather than 2 because we consider the activations that
        // occur at the *end* of `epoch`, after `process_justification_and_finalization` has already
        // updated the state's checkpoint.
        && self.activation_eligibility_epoch < epoch
    }

    /// Returns `true` if the validator has eth1 withdrawal credential.
    pub fn has_eth1_withdrawal_credential(&self, spec: &ChainSpec) -> bool {
        self.withdrawal_credentials
            .as_slice()
            .first()
            .map(|byte| *byte == spec.eth1_address_withdrawal_prefix_byte)
            .unwrap_or(false)
    }

    /// Check if ``validator`` has an 0x02 prefixed "compounding" withdrawal credential.
    pub fn has_compounding_withdrawal_credential(&self, spec: &ChainSpec) -> bool {
        is_compounding_withdrawal_credential(self.withdrawal_credentials, spec)
    }

    /// Get the execution withdrawal address if this validator has one initialized.
    pub fn get_execution_withdrawal_address(&self, spec: &ChainSpec) -> Option<Address> {
        self.has_execution_withdrawal_credential(spec)
            .then(|| {
                self.withdrawal_credentials
                    .as_slice()
                    .get(12..)
                    .map(Address::from_slice)
            })
            .flatten()
    }

    /// Changes withdrawal credentials to  the provided eth1 execution address.
    ///
    /// WARNING: this function does NO VALIDATION - it just does it!
    pub fn change_withdrawal_credentials(&mut self, execution_address: &Address, spec: &ChainSpec) {
        let mut bytes = [0u8; 32];
        bytes[0] = spec.eth1_address_withdrawal_prefix_byte;
        bytes[12..].copy_from_slice(execution_address.as_slice());
        self.withdrawal_credentials = Hash256::from(bytes);
    }

    /// Returns `true` if the validator is fully withdrawable at some epoch.
    ///
    /// Calls the correct function depending on the provided `fork_name`.
    pub fn is_fully_withdrawable_validator(
        &self,
        balance: u64,
        epoch: Epoch,
        spec: &ChainSpec,
        current_fork: ForkName,
    ) -> bool {
        if current_fork.electra_enabled() {
            self.is_fully_withdrawable_validator_electra(balance, epoch, spec)
        } else {
            self.is_fully_withdrawable_validator_capella(balance, epoch, spec)
        }
    }

    /// Returns `true` if the validator is fully withdrawable at some epoch.
    fn is_fully_withdrawable_validator_capella(
        &self,
        balance: u64,
        epoch: Epoch,
        spec: &ChainSpec,
    ) -> bool {
        self.has_eth1_withdrawal_credential(spec) && self.withdrawable_epoch <= epoch && balance > 0
    }

    /// Returns `true` if the validator is fully withdrawable at some epoch.
    ///
    /// Modified in electra as part of EIP 7251.
    fn is_fully_withdrawable_validator_electra(
        &self,
        balance: u64,
        epoch: Epoch,
        spec: &ChainSpec,
    ) -> bool {
        self.has_execution_withdrawal_credential(spec)
            && self.withdrawable_epoch <= epoch
            && balance > 0
    }

    /// Returns `true` if the validator is partially withdrawable.
    ///
    /// Calls the correct function depending on the provided `fork_name`.
    pub fn is_partially_withdrawable_validator(
        &self,
        balance: u64,
        spec: &ChainSpec,
        current_fork: ForkName,
    ) -> bool {
        if current_fork.electra_enabled() {
            self.is_partially_withdrawable_validator_electra(balance, spec, current_fork)
        } else {
            self.is_partially_withdrawable_validator_capella(balance, spec)
        }
    }

    /// Returns `true` if the validator is partially withdrawable.
    fn is_partially_withdrawable_validator_capella(&self, balance: u64, spec: &ChainSpec) -> bool {
        self.has_eth1_withdrawal_credential(spec)
            && self.effective_balance == spec.max_effective_balance
            && balance > spec.max_effective_balance
    }

    /// Returns `true` if the validator is partially withdrawable.
    ///
    /// Modified in electra as part of EIP 7251.
    pub fn is_partially_withdrawable_validator_electra(
        &self,
        balance: u64,
        spec: &ChainSpec,
        current_fork: ForkName,
    ) -> bool {
        let max_effective_balance = self.get_max_effective_balance(spec, current_fork);
        let has_max_effective_balance = self.effective_balance == max_effective_balance;
        let has_excess_balance = balance > max_effective_balance;
        self.has_execution_withdrawal_credential(spec)
            && has_max_effective_balance
            && has_excess_balance
    }

    /// Returns `true` if the validator has a 0x01 or 0x02 prefixed withdrawal credential.
    pub fn has_execution_withdrawal_credential(&self, spec: &ChainSpec) -> bool {
        self.has_compounding_withdrawal_credential(spec)
            || self.has_eth1_withdrawal_credential(spec)
    }

    /// Returns the max effective balance for a validator in gwei.
    pub fn get_max_effective_balance(&self, spec: &ChainSpec, current_fork: ForkName) -> u64 {
        if current_fork >= ForkName::Electra {
            if self.has_compounding_withdrawal_credential(spec) {
                spec.max_effective_balance_electra
            } else {
                spec.min_activation_balance
            }
        } else {
            spec.max_effective_balance
        }
    }
}

impl Default for Validator {
    /// Yields a "default" `Validator`. Primarily used for testing.
    fn default() -> Self {
        Self {
            pubkey: PublicKeyBytes::empty(),
            withdrawal_credentials: Hash256::zero(),
            activation_eligibility_epoch: Epoch::from(u64::MAX),
            activation_epoch: Epoch::from(u64::MAX),
            exit_epoch: Epoch::from(u64::MAX),
            withdrawable_epoch: Epoch::from(u64::MAX),
            slashed: false,
            effective_balance: u64::MAX,
        }
    }
}

pub fn is_compounding_withdrawal_credential(
    withdrawal_credentials: Hash256,
    spec: &ChainSpec,
) -> bool {
    withdrawal_credentials
        .as_slice()
        .first()
        .map(|prefix_byte| *prefix_byte == spec.compounding_withdrawal_prefix_byte)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default() {
        let v = Validator::default();

        let epoch = Epoch::new(0);

        assert!(!v.is_active_at(epoch));
        assert!(!v.is_exited_at(epoch));
        assert!(!v.is_withdrawable_at(epoch));
        assert!(!v.slashed);
    }

    #[test]
    fn is_active_at() {
        let epoch = Epoch::new(10);

        let v = Validator {
            activation_epoch: epoch,
            ..Validator::default()
        };

        assert!(!v.is_active_at(epoch - 1));
        assert!(v.is_active_at(epoch));
        assert!(v.is_active_at(epoch + 1));
    }

    #[test]
    fn is_exited_at() {
        let epoch = Epoch::new(10);

        let v = Validator {
            exit_epoch: epoch,
            ..Validator::default()
        };

        assert!(!v.is_exited_at(epoch - 1));
        assert!(v.is_exited_at(epoch));
        assert!(v.is_exited_at(epoch + 1));
    }

    #[test]
    fn is_withdrawable_at() {
        let epoch = Epoch::new(10);

        let v = Validator {
            withdrawable_epoch: epoch,
            ..Validator::default()
        };

        assert!(!v.is_withdrawable_at(epoch - 1));
        assert!(v.is_withdrawable_at(epoch));
        assert!(v.is_withdrawable_at(epoch + 1));
    }

    ssz_and_tree_hash_tests!(Validator);

    fn make_spec() -> ChainSpec {
        crate::MinimalEthSpec::default_spec()
    }

    fn eth1_creds(spec: &ChainSpec) -> Hash256 {
        let mut bytes = [0u8; 32];
        bytes[0] = spec.eth1_address_withdrawal_prefix_byte;
        bytes[12..].copy_from_slice(&[0xAA; 20]);
        Hash256::from(bytes)
    }

    fn compounding_creds(spec: &ChainSpec) -> Hash256 {
        let mut bytes = [0u8; 32];
        bytes[0] = spec.compounding_withdrawal_prefix_byte;
        bytes[12..].copy_from_slice(&[0xBB; 20]);
        Hash256::from(bytes)
    }

    fn bls_creds() -> Hash256 {
        let mut bytes = [0u8; 32];
        bytes[0] = 0x00;
        Hash256::from(bytes)
    }

    fn active_validator(spec: &ChainSpec) -> Validator {
        Validator {
            activation_epoch: Epoch::new(0),
            exit_epoch: spec.far_future_epoch,
            withdrawable_epoch: spec.far_future_epoch,
            effective_balance: spec.max_effective_balance,
            ..Validator::default()
        }
    }

    #[test]
    fn is_slashable_at_active_unslashed() {
        let v = Validator {
            activation_epoch: Epoch::new(5),
            withdrawable_epoch: Epoch::new(20),
            slashed: false,
            ..Validator::default()
        };
        assert!(!v.is_slashable_at(Epoch::new(4)));
        assert!(v.is_slashable_at(Epoch::new(5)));
        assert!(v.is_slashable_at(Epoch::new(19)));
        assert!(!v.is_slashable_at(Epoch::new(20)));
    }

    #[test]
    fn is_slashable_at_already_slashed() {
        let v = Validator {
            activation_epoch: Epoch::new(0),
            withdrawable_epoch: Epoch::new(100),
            slashed: true,
            ..Validator::default()
        };
        assert!(!v.is_slashable_at(Epoch::new(50)));
    }

    #[test]
    fn from_deposit_caps_effective_balance() {
        let spec = make_spec();
        let amount = spec.max_effective_balance + 1_000_000_000;
        let v = Validator::from_deposit(
            PublicKeyBytes::empty(),
            Hash256::zero(),
            amount,
            ForkName::Base,
            &spec,
        );
        assert_eq!(v.effective_balance, spec.max_effective_balance);
        assert_eq!(v.activation_eligibility_epoch, spec.far_future_epoch);
        assert_eq!(v.activation_epoch, spec.far_future_epoch);
        assert!(!v.slashed);
    }

    #[test]
    fn from_deposit_rounds_down_to_increment() {
        let spec = make_spec();
        let amount = spec.max_effective_balance - 1;
        let v = Validator::from_deposit(
            PublicKeyBytes::empty(),
            Hash256::zero(),
            amount,
            ForkName::Base,
            &spec,
        );
        let expected = amount - (amount % spec.effective_balance_increment);
        assert_eq!(v.effective_balance, expected);
    }

    #[test]
    fn has_eth1_withdrawal_credential_true() {
        let spec = make_spec();
        let v = Validator {
            withdrawal_credentials: eth1_creds(&spec),
            ..active_validator(&spec)
        };
        assert!(v.has_eth1_withdrawal_credential(&spec));
        assert!(!v.has_compounding_withdrawal_credential(&spec));
    }

    #[test]
    fn has_compounding_withdrawal_credential_true() {
        let spec = make_spec();
        let v = Validator {
            withdrawal_credentials: compounding_creds(&spec),
            ..active_validator(&spec)
        };
        assert!(v.has_compounding_withdrawal_credential(&spec));
        assert!(!v.has_eth1_withdrawal_credential(&spec));
    }

    #[test]
    fn has_execution_withdrawal_credential_either() {
        let spec = make_spec();
        let v_eth1 = Validator {
            withdrawal_credentials: eth1_creds(&spec),
            ..active_validator(&spec)
        };
        let v_comp = Validator {
            withdrawal_credentials: compounding_creds(&spec),
            ..active_validator(&spec)
        };
        let v_bls = Validator {
            withdrawal_credentials: bls_creds(),
            ..active_validator(&spec)
        };
        assert!(v_eth1.has_execution_withdrawal_credential(&spec));
        assert!(v_comp.has_execution_withdrawal_credential(&spec));
        assert!(!v_bls.has_execution_withdrawal_credential(&spec));
    }

    #[test]
    fn get_execution_withdrawal_address() {
        let spec = make_spec();
        let v = Validator {
            withdrawal_credentials: eth1_creds(&spec),
            ..active_validator(&spec)
        };
        let addr = v.get_execution_withdrawal_address(&spec).unwrap();
        assert_eq!(addr.as_slice(), &[0xAA; 20]);
    }

    #[test]
    fn get_execution_withdrawal_address_bls_returns_none() {
        let spec = make_spec();
        let v = Validator {
            withdrawal_credentials: bls_creds(),
            ..active_validator(&spec)
        };
        assert!(v.get_execution_withdrawal_address(&spec).is_none());
    }

    #[test]
    fn change_withdrawal_credentials_sets_eth1_prefix() {
        let spec = make_spec();
        let mut v = Validator {
            withdrawal_credentials: bls_creds(),
            ..active_validator(&spec)
        };
        let addr = Address::from_slice(&[0xCC; 20]);
        v.change_withdrawal_credentials(&addr, &spec);
        assert!(v.has_eth1_withdrawal_credential(&spec));
        assert_eq!(v.get_execution_withdrawal_address(&spec).unwrap(), addr);
    }

    #[test]
    fn is_eligible_for_activation_queue_base() {
        let spec = make_spec();
        let v = Validator {
            activation_eligibility_epoch: spec.far_future_epoch,
            effective_balance: spec.max_effective_balance,
            ..active_validator(&spec)
        };
        assert!(v.is_eligible_for_activation_queue(&spec, ForkName::Base));
    }

    #[test]
    fn is_eligible_for_activation_queue_base_wrong_balance() {
        let spec = make_spec();
        let v = Validator {
            activation_eligibility_epoch: spec.far_future_epoch,
            effective_balance: spec.max_effective_balance - 1,
            ..active_validator(&spec)
        };
        assert!(!v.is_eligible_for_activation_queue(&spec, ForkName::Base));
    }

    #[test]
    fn is_eligible_for_activation_queue_electra() {
        let spec = make_spec();
        let v = Validator {
            activation_eligibility_epoch: spec.far_future_epoch,
            effective_balance: spec.min_activation_balance,
            ..active_validator(&spec)
        };
        assert!(v.is_eligible_for_activation_queue(&spec, ForkName::Electra));
    }

    #[test]
    fn is_eligible_for_activation_queue_already_eligible() {
        let spec = make_spec();
        let v = Validator {
            activation_eligibility_epoch: Epoch::new(5),
            effective_balance: spec.max_effective_balance,
            ..active_validator(&spec)
        };
        assert!(!v.is_eligible_for_activation_queue(&spec, ForkName::Base));
    }

    #[test]
    fn could_be_eligible_for_activation_at() {
        let spec = make_spec();
        let v = Validator {
            activation_eligibility_epoch: Epoch::new(3),
            activation_epoch: spec.far_future_epoch,
            ..Validator::default()
        };
        assert!(!v.could_be_eligible_for_activation_at(Epoch::new(3), &spec));
        assert!(v.could_be_eligible_for_activation_at(Epoch::new(4), &spec));
        assert!(v.could_be_eligible_for_activation_at(Epoch::new(100), &spec));
    }

    #[test]
    fn could_be_eligible_for_activation_at_already_activated() {
        let spec = make_spec();
        let v = Validator {
            activation_eligibility_epoch: Epoch::new(3),
            activation_epoch: Epoch::new(5),
            ..Validator::default()
        };
        assert!(!v.could_be_eligible_for_activation_at(Epoch::new(10), &spec));
    }

    #[test]
    fn get_max_effective_balance_pre_electra() {
        let spec = make_spec();
        let v = Validator {
            withdrawal_credentials: compounding_creds(&spec),
            ..active_validator(&spec)
        };
        assert_eq!(
            v.get_max_effective_balance(&spec, ForkName::Deneb),
            spec.max_effective_balance
        );
    }

    #[test]
    fn get_max_effective_balance_electra_compounding() {
        let spec = make_spec();
        let v = Validator {
            withdrawal_credentials: compounding_creds(&spec),
            ..active_validator(&spec)
        };
        assert_eq!(
            v.get_max_effective_balance(&spec, ForkName::Electra),
            spec.max_effective_balance_electra
        );
    }

    #[test]
    fn get_max_effective_balance_electra_eth1() {
        let spec = make_spec();
        let v = Validator {
            withdrawal_credentials: eth1_creds(&spec),
            ..active_validator(&spec)
        };
        assert_eq!(
            v.get_max_effective_balance(&spec, ForkName::Electra),
            spec.min_activation_balance
        );
    }

    #[test]
    fn is_fully_withdrawable_capella() {
        let spec = make_spec();
        let v = Validator {
            withdrawal_credentials: eth1_creds(&spec),
            withdrawable_epoch: Epoch::new(10),
            ..active_validator(&spec)
        };
        assert!(!v.is_fully_withdrawable_validator(100, Epoch::new(9), &spec, ForkName::Capella));
        assert!(v.is_fully_withdrawable_validator(100, Epoch::new(10), &spec, ForkName::Capella));
        assert!(!v.is_fully_withdrawable_validator(0, Epoch::new(10), &spec, ForkName::Capella));
    }

    #[test]
    fn is_fully_withdrawable_capella_bls_creds() {
        let spec = make_spec();
        let v = Validator {
            withdrawal_credentials: bls_creds(),
            withdrawable_epoch: Epoch::new(10),
            ..active_validator(&spec)
        };
        assert!(!v.is_fully_withdrawable_validator(100, Epoch::new(10), &spec, ForkName::Capella));
    }

    #[test]
    fn is_fully_withdrawable_electra_compounding() {
        let spec = make_spec();
        let v = Validator {
            withdrawal_credentials: compounding_creds(&spec),
            withdrawable_epoch: Epoch::new(10),
            ..active_validator(&spec)
        };
        assert!(v.is_fully_withdrawable_validator(100, Epoch::new(10), &spec, ForkName::Electra));
    }

    #[test]
    fn is_partially_withdrawable_capella() {
        let spec = make_spec();
        let v = Validator {
            withdrawal_credentials: eth1_creds(&spec),
            effective_balance: spec.max_effective_balance,
            ..active_validator(&spec)
        };
        let excess = spec.max_effective_balance + 1;
        assert!(v.is_partially_withdrawable_validator(excess, &spec, ForkName::Capella));
        assert!(!v.is_partially_withdrawable_validator(
            spec.max_effective_balance,
            &spec,
            ForkName::Capella
        ));
    }

    #[test]
    fn is_partially_withdrawable_capella_bls_creds() {
        let spec = make_spec();
        let v = Validator {
            withdrawal_credentials: bls_creds(),
            effective_balance: spec.max_effective_balance,
            ..active_validator(&spec)
        };
        assert!(!v.is_partially_withdrawable_validator(
            spec.max_effective_balance + 1,
            &spec,
            ForkName::Capella
        ));
    }

    #[test]
    fn is_partially_withdrawable_electra_compounding() {
        let spec = make_spec();
        let v = Validator {
            withdrawal_credentials: compounding_creds(&spec),
            effective_balance: spec.max_effective_balance_electra,
            ..active_validator(&spec)
        };
        assert!(v.is_partially_withdrawable_validator(
            spec.max_effective_balance_electra + 1,
            &spec,
            ForkName::Electra
        ));
        assert!(!v.is_partially_withdrawable_validator(
            spec.max_effective_balance_electra,
            &spec,
            ForkName::Electra
        ));
    }

    #[test]
    fn is_compounding_withdrawal_credential_standalone() {
        let spec = make_spec();
        assert!(is_compounding_withdrawal_credential(
            compounding_creds(&spec),
            &spec
        ));
        assert!(!is_compounding_withdrawal_credential(
            eth1_creds(&spec),
            &spec
        ));
        assert!(!is_compounding_withdrawal_credential(bls_creds(), &spec));
    }
}
