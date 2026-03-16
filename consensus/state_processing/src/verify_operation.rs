use crate::VerifySignatures;
use crate::per_block_processing::{
    errors::{
        AttesterSlashingValidationError, BlsExecutionChangeValidationError, ExitValidationError,
        ProposerSlashingValidationError,
    },
    verify_attester_slashing, verify_bls_to_execution_change, verify_exit,
    verify_proposer_slashing,
};
#[cfg(feature = "arbitrary-fuzz")]
use arbitrary::Arbitrary;
use educe::Educe;
use smallvec::{SmallVec, smallvec};
use ssz::{Decode, Encode};
use ssz_derive::{Decode, Encode};
use std::marker::PhantomData;
use test_random_derive::TestRandom;
use types::{
    AttesterSlashing, AttesterSlashingOnDisk, AttesterSlashingRefOnDisk, BeaconState, ChainSpec,
    Epoch, EthSpec, Fork, ForkVersion, ProposerSlashing, SignedBlsToExecutionChange,
    SignedVoluntaryExit, test_utils::TestRandom,
};

const MAX_FORKS_VERIFIED_AGAINST: usize = 2;

pub trait TransformPersist {
    type Persistable: Encode + Decode;
    type PersistableRef<'a>: Encode
    where
        Self: 'a;

    /// Returns a reference to the object in a form that implements `Encode`
    fn as_persistable_ref(&self) -> Self::PersistableRef<'_>;

    /// Converts the object back into its original form.
    fn from_persistable(persistable: Self::Persistable) -> Self;
}

/// Wrapper around an operation type that acts as proof that its signature has been checked.
///
/// The inner `op` field is private, meaning instances of this type can only be constructed
/// by calling `validate`.
#[derive(Educe, Debug, Clone)]
#[cfg_attr(feature = "arbitrary-fuzz", derive(Arbitrary))]
#[educe(
    PartialEq,
    Eq,
    Hash(bound(T: TransformPersist + std::hash::Hash, E: EthSpec))
)]
#[cfg_attr(
    feature = "arbitrary-fuzz",
    arbitrary(bound = "T: TransformPersist + Arbitrary<'arbitrary>, E: EthSpec")
)]
pub struct SigVerifiedOp<T: TransformPersist, E: EthSpec> {
    op: T,
    verified_against: VerifiedAgainst,
    _phantom: PhantomData<E>,
}

impl<T: TransformPersist, E: EthSpec> Encode for SigVerifiedOp<T, E> {
    fn is_ssz_fixed_len() -> bool {
        <SigVerifiedOpEncode<T::Persistable> as Encode>::is_ssz_fixed_len()
    }

    fn ssz_fixed_len() -> usize {
        <SigVerifiedOpEncode<T::Persistable> as Encode>::ssz_fixed_len()
    }

    fn ssz_append(&self, buf: &mut Vec<u8>) {
        let persistable_ref = self.op.as_persistable_ref();
        SigVerifiedOpEncode {
            op: persistable_ref,
            verified_against: &self.verified_against,
        }
        .ssz_append(buf)
    }

    fn ssz_bytes_len(&self) -> usize {
        let persistable_ref = self.op.as_persistable_ref();
        SigVerifiedOpEncode {
            op: persistable_ref,
            verified_against: &self.verified_against,
        }
        .ssz_bytes_len()
    }
}

impl<T: TransformPersist, E: EthSpec> Decode for SigVerifiedOp<T, E> {
    fn is_ssz_fixed_len() -> bool {
        <SigVerifiedOpDecode<T::Persistable> as Decode>::is_ssz_fixed_len()
    }

    fn ssz_fixed_len() -> usize {
        <SigVerifiedOpDecode<T::Persistable> as Decode>::ssz_fixed_len()
    }

    fn from_ssz_bytes(bytes: &[u8]) -> Result<Self, ssz::DecodeError> {
        let on_disk = SigVerifiedOpDecode::<T::Persistable>::from_ssz_bytes(bytes)?;
        Ok(SigVerifiedOp {
            op: T::from_persistable(on_disk.op),
            verified_against: on_disk.verified_against,
            _phantom: PhantomData,
        })
    }
}

/// On-disk variant of `SigVerifiedOp` that implements `Encode`.
///
/// We use separate types for Encode and Decode so we can efficiently handle references: the Encode
/// type contains references, while the Decode type does not.
#[derive(Debug, Encode)]
struct SigVerifiedOpEncode<'a, P: Encode> {
    op: P,
    verified_against: &'a VerifiedAgainst,
}

/// On-disk variant of `SigVerifiedOp` that implements `Encode`.
#[derive(Debug, Decode)]
struct SigVerifiedOpDecode<P: Decode> {
    op: P,
    verified_against: VerifiedAgainst,
}

/// Information about the fork versions that this message was verified against.
///
/// In general it is not safe to assume that a `SigVerifiedOp` constructed at some point in the past
/// will continue to be valid in the presence of a changing `state.fork()`. The reason for this
/// is that the fork versions that the message's epochs map to might change.
///
/// For example a proposer slashing at a phase0 slot verified against an Altair state will use
/// the phase0 fork version, but will become invalid once the Bellatrix fork occurs because that
/// slot will start to map to the Altair fork version. This is because `Fork::get_fork_version` only
/// remembers the most recent two forks.
///
/// In the other direction, a proposer slashing at a Bellatrix slot verified against an Altair state
/// will use the Altair fork version, but will become invalid once the Bellatrix fork occurs because
/// that slot will start to map to the Bellatrix fork version.
///
/// We need to store multiple `ForkVersion`s because attester slashings contain two indexed
/// attestations which may be signed using different versions.
#[derive(Debug, PartialEq, Eq, Clone, Hash, Encode, Decode, TestRandom)]
#[cfg_attr(feature = "arbitrary-fuzz", derive(Arbitrary))]
pub struct VerifiedAgainst {
    fork_versions: SmallVec<[ForkVersion; MAX_FORKS_VERIFIED_AGAINST]>,
}

impl<T, E> SigVerifiedOp<T, E>
where
    T: VerifyOperation<E>,
    E: EthSpec,
{
    /// This function must be private because it assumes that `op` has already been verified.
    fn new(op: T, state: &BeaconState<E>) -> Self {
        let verified_against = VerifiedAgainst {
            fork_versions: op
                .verification_epochs()
                .into_iter()
                .map(|epoch| state.fork().get_fork_version(epoch))
                .collect(),
        };

        SigVerifiedOp {
            op,
            verified_against,
            _phantom: PhantomData,
        }
    }

    pub fn into_inner(self) -> T {
        self.op
    }

    pub fn as_inner(&self) -> &T {
        &self.op
    }

    pub fn signature_is_still_valid(&self, current_fork: &Fork) -> bool {
        // The .all() will return true if the iterator is empty.
        self.as_inner()
            .verification_epochs()
            .into_iter()
            .zip(self.verified_against.fork_versions.iter())
            .all(|(epoch, verified_fork_version)| {
                current_fork.get_fork_version(epoch) == *verified_fork_version
            })
    }

    /// Return one of the fork versions this message was verified against.
    ///
    /// This is only required for the v12 schema downgrade and can be deleted once all nodes
    /// are upgraded to v12.
    pub fn first_fork_verified_against(&self) -> Option<ForkVersion> {
        self.verified_against.fork_versions.first().copied()
    }
}

/// Trait for operations that can be verified and transformed into a `SigVerifiedOp`.
pub trait VerifyOperation<E: EthSpec>: TransformPersist + Sized {
    type Error;

    fn validate(
        self,
        state: &BeaconState<E>,
        spec: &ChainSpec,
    ) -> Result<SigVerifiedOp<Self, E>, Self::Error>;

    /// Return the epochs at which parts of this message were verified.
    ///
    /// These need to map 1-to-1 to the `SigVerifiedOp::verified_against` for this type.
    ///
    /// If the message is valid across all forks it should return an empty smallvec.
    fn verification_epochs(&self) -> SmallVec<[Epoch; MAX_FORKS_VERIFIED_AGAINST]>;
}

impl<E: EthSpec> VerifyOperation<E> for SignedVoluntaryExit {
    type Error = ExitValidationError;

    fn validate(
        self,
        state: &BeaconState<E>,
        spec: &ChainSpec,
    ) -> Result<SigVerifiedOp<Self, E>, Self::Error> {
        verify_exit(state, None, &self, VerifySignatures::True, spec)?;
        Ok(SigVerifiedOp::new(self, state))
    }

    #[allow(clippy::arithmetic_side_effects)]
    fn verification_epochs(&self) -> SmallVec<[Epoch; MAX_FORKS_VERIFIED_AGAINST]> {
        smallvec![self.message.epoch]
    }
}

impl<E: EthSpec> VerifyOperation<E> for AttesterSlashing<E> {
    type Error = AttesterSlashingValidationError;

    fn validate(
        self,
        state: &BeaconState<E>,
        spec: &ChainSpec,
    ) -> Result<SigVerifiedOp<Self, E>, Self::Error> {
        verify_attester_slashing(state, self.to_ref(), VerifySignatures::True, spec)?;
        Ok(SigVerifiedOp::new(self, state))
    }

    #[allow(clippy::arithmetic_side_effects)]
    fn verification_epochs(&self) -> SmallVec<[Epoch; MAX_FORKS_VERIFIED_AGAINST]> {
        smallvec![
            self.attestation_1().data().target.epoch,
            self.attestation_2().data().target.epoch
        ]
    }
}

impl<E: EthSpec> VerifyOperation<E> for ProposerSlashing {
    type Error = ProposerSlashingValidationError;

    fn validate(
        self,
        state: &BeaconState<E>,
        spec: &ChainSpec,
    ) -> Result<SigVerifiedOp<Self, E>, Self::Error> {
        verify_proposer_slashing(&self, state, VerifySignatures::True, spec)?;
        Ok(SigVerifiedOp::new(self, state))
    }

    #[allow(clippy::arithmetic_side_effects)]
    fn verification_epochs(&self) -> SmallVec<[Epoch; MAX_FORKS_VERIFIED_AGAINST]> {
        // Only need a single epoch because the slots of the two headers must be equal.
        smallvec![
            self.signed_header_1
                .message
                .slot
                .epoch(E::slots_per_epoch())
        ]
    }
}

impl<E: EthSpec> VerifyOperation<E> for SignedBlsToExecutionChange {
    type Error = BlsExecutionChangeValidationError;

    fn validate(
        self,
        state: &BeaconState<E>,
        spec: &ChainSpec,
    ) -> Result<SigVerifiedOp<Self, E>, Self::Error> {
        verify_bls_to_execution_change(state, &self, VerifySignatures::True, spec)?;
        Ok(SigVerifiedOp::new(self, state))
    }

    #[allow(clippy::arithmetic_side_effects)]
    fn verification_epochs(&self) -> SmallVec<[Epoch; MAX_FORKS_VERIFIED_AGAINST]> {
        smallvec![]
    }
}

/// Trait for operations that can be verified and transformed into a
/// `SigVerifiedOp`.
///
/// The `At` suffix indicates that we can specify a particular epoch at which to
/// verify the operation.
pub trait VerifyOperationAt<E: EthSpec>: VerifyOperation<E> + Sized {
    fn validate_at(
        self,
        state: &BeaconState<E>,
        validate_at_epoch: Epoch,
        spec: &ChainSpec,
    ) -> Result<SigVerifiedOp<Self, E>, Self::Error>;
}

impl<E: EthSpec> VerifyOperationAt<E> for SignedVoluntaryExit {
    fn validate_at(
        self,
        state: &BeaconState<E>,
        validate_at_epoch: Epoch,
        spec: &ChainSpec,
    ) -> Result<SigVerifiedOp<Self, E>, Self::Error> {
        verify_exit(
            state,
            Some(validate_at_epoch),
            &self,
            VerifySignatures::True,
            spec,
        )?;
        Ok(SigVerifiedOp::new(self, state))
    }
}

impl TransformPersist for SignedVoluntaryExit {
    type Persistable = Self;
    type PersistableRef<'a> = &'a Self;

    fn as_persistable_ref(&self) -> Self::PersistableRef<'_> {
        self
    }

    fn from_persistable(persistable: Self::Persistable) -> Self {
        persistable
    }
}

impl<E: EthSpec> TransformPersist for AttesterSlashing<E> {
    type Persistable = AttesterSlashingOnDisk<E>;
    type PersistableRef<'a> = AttesterSlashingRefOnDisk<'a, E>;

    fn as_persistable_ref(&self) -> Self::PersistableRef<'_> {
        self.to_ref().into()
    }

    fn from_persistable(persistable: Self::Persistable) -> Self {
        persistable.into()
    }
}

impl TransformPersist for ProposerSlashing {
    type Persistable = Self;
    type PersistableRef<'a> = &'a Self;

    fn as_persistable_ref(&self) -> Self::PersistableRef<'_> {
        self
    }

    fn from_persistable(persistable: Self::Persistable) -> Self {
        persistable
    }
}

impl TransformPersist for SignedBlsToExecutionChange {
    type Persistable = Self;
    type PersistableRef<'a> = &'a Self;

    fn as_persistable_ref(&self) -> Self::PersistableRef<'_> {
        self
    }

    fn from_persistable(persistable: Self::Persistable) -> Self {
        persistable
    }
}

#[cfg(all(test, not(debug_assertions)))]
mod test {
    use super::*;
    use types::{
        Address, AggregateSignature, BeaconBlockHeader, BlsToExecutionChange, FixedBytesExtended,
        ForkVersion, MainnetEthSpec, PublicKeyBytes, Signature, SignedBeaconBlockHeader,
        VariableList, VoluntaryExit,
        test_utils::{SeedableRng, TestRandom, XorShiftRng},
    };

    type E = MainnetEthSpec;

    fn roundtrip_test<T: TestRandom + TransformPersist + PartialEq + std::fmt::Debug>() {
        let runs = 10;
        let mut rng = XorShiftRng::seed_from_u64(0xff0af5a356af1123);

        for _ in 0..runs {
            let op = T::random_for_test(&mut rng);
            let verified_against = VerifiedAgainst::random_for_test(&mut rng);

            let verified_op = SigVerifiedOp {
                op,
                verified_against,
                _phantom: PhantomData::<E>,
            };

            let serialized = verified_op.as_ssz_bytes();
            let deserialized = SigVerifiedOp::from_ssz_bytes(&serialized).unwrap();
            let reserialized = deserialized.as_ssz_bytes();
            assert_eq!(verified_op, deserialized);
            assert_eq!(serialized, reserialized);
        }
    }

    #[test]
    fn sig_verified_op_exit_roundtrip() {
        roundtrip_test::<SignedVoluntaryExit>();
    }

    #[test]
    fn proposer_slashing_roundtrip() {
        roundtrip_test::<ProposerSlashing>();
    }

    #[test]
    fn attester_slashing_roundtrip() {
        roundtrip_test::<AttesterSlashing<E>>();
    }

    #[test]
    fn bls_to_execution_roundtrip() {
        roundtrip_test::<SignedBlsToExecutionChange>();
    }

    // ── Helper to build a SigVerifiedOp directly (bypassing signature check) ──

    fn make_verified_op<T: TransformPersist + VerifyOperation<E>>(
        op: T,
        fork_versions: SmallVec<[ForkVersion; MAX_FORKS_VERIFIED_AGAINST]>,
    ) -> SigVerifiedOp<T, E> {
        SigVerifiedOp {
            op,
            verified_against: VerifiedAgainst { fork_versions },
            _phantom: PhantomData,
        }
    }

    fn make_exit(epoch: u64) -> SignedVoluntaryExit {
        SignedVoluntaryExit {
            message: VoluntaryExit {
                epoch: Epoch::new(epoch),
                validator_index: 0,
            },
            signature: Signature::empty(),
        }
    }

    fn make_fork(previous: [u8; 4], current: [u8; 4], epoch: u64) -> Fork {
        Fork {
            previous_version: previous,
            current_version: current,
            epoch: Epoch::new(epoch),
        }
    }

    // ── verification_epochs tests ──

    #[test]
    fn exit_verification_epochs_returns_message_epoch() {
        let exit = make_exit(42);
        let epochs = <SignedVoluntaryExit as VerifyOperation<E>>::verification_epochs(&exit);
        assert_eq!(epochs.len(), 1);
        assert_eq!(epochs[0], Epoch::new(42));
    }

    #[test]
    fn exit_verification_epochs_at_zero() {
        let exit = make_exit(0);
        let epochs = <SignedVoluntaryExit as VerifyOperation<E>>::verification_epochs(&exit);
        assert_eq!(epochs.len(), 1);
        assert_eq!(epochs[0], Epoch::new(0));
    }

    #[test]
    fn proposer_slashing_verification_epochs_returns_single_epoch() {
        let header = BeaconBlockHeader {
            slot: types::Slot::new(64), // epoch 2 with 32 slots/epoch
            proposer_index: 0,
            parent_root: types::Hash256::zero(),
            state_root: types::Hash256::zero(),
            body_root: types::Hash256::zero(),
        };
        let slashing = ProposerSlashing {
            signed_header_1: SignedBeaconBlockHeader {
                message: header.clone(),
                signature: Signature::empty(),
            },
            signed_header_2: SignedBeaconBlockHeader {
                message: header,
                signature: Signature::empty(),
            },
        };
        let epochs = <ProposerSlashing as VerifyOperation<E>>::verification_epochs(&slashing);
        // Both headers must have the same slot, so only one epoch is needed.
        assert_eq!(epochs.len(), 1);
        assert_eq!(epochs[0], types::Slot::new(64).epoch(E::slots_per_epoch()));
    }

    fn make_attester_slashing(epoch1: u64, epoch2: u64) -> AttesterSlashing<E> {
        let make_att = |target_epoch: u64| types::indexed_attestation::IndexedAttestationBase {
            attesting_indices: VariableList::empty(),
            data: types::AttestationData {
                target: types::Checkpoint {
                    epoch: Epoch::new(target_epoch),
                    root: types::Hash256::zero(),
                },
                ..Default::default()
            },
            signature: AggregateSignature::empty(),
        };
        AttesterSlashing::Base(types::attester_slashing::AttesterSlashingBase {
            attestation_1: make_att(epoch1),
            attestation_2: make_att(epoch2),
        })
    }

    fn make_bls_change() -> SignedBlsToExecutionChange {
        SignedBlsToExecutionChange {
            message: BlsToExecutionChange {
                validator_index: 0,
                from_bls_pubkey: PublicKeyBytes::empty(),
                to_execution_address: Address::zero(),
            },
            signature: Signature::empty(),
        }
    }

    #[test]
    fn attester_slashing_verification_epochs_returns_two_epochs() {
        let slashing = make_attester_slashing(5, 10);
        let epochs = <AttesterSlashing<E> as VerifyOperation<E>>::verification_epochs(&slashing);
        assert_eq!(epochs.len(), 2);
        assert_eq!(epochs[0], Epoch::new(5));
        assert_eq!(epochs[1], Epoch::new(10));
    }

    #[test]
    fn bls_to_execution_change_verification_epochs_is_empty() {
        // BLS-to-execution changes are valid across all forks.
        let change = make_bls_change();
        let epochs =
            <SignedBlsToExecutionChange as VerifyOperation<E>>::verification_epochs(&change);
        assert!(epochs.is_empty());
    }

    // ── signature_is_still_valid tests ──

    #[test]
    fn signature_valid_when_fork_unchanged() {
        let exit = make_exit(5);
        // Fork: previous=[1,0,0,0] for epochs < 10, current=[2,0,0,0] for epochs >= 10.
        // Exit at epoch 5 → uses previous_version = [1,0,0,0].
        let fork = make_fork([1, 0, 0, 0], [2, 0, 0, 0], 10);
        let verified = make_verified_op(exit, smallvec![[1, 0, 0, 0]]);
        assert!(
            verified.signature_is_still_valid(&fork),
            "signature should be valid when fork version matches"
        );
    }

    #[test]
    fn signature_invalid_after_fork_transition() {
        // Exit at epoch 5, verified when fork epoch was 10 → used previous_version [1,0,0,0].
        // Now the fork has changed: epoch 5 maps to a different version.
        let exit = make_exit(5);
        let verified = make_verified_op(exit, smallvec![[1, 0, 0, 0]]);

        // New fork: previous=[3,0,0,0] current=[4,0,0,0] epoch=3.
        // Epoch 5 >= 3, so get_fork_version(5) = [4,0,0,0] ≠ [1,0,0,0].
        let new_fork = make_fork([3, 0, 0, 0], [4, 0, 0, 0], 3);
        assert!(
            !verified.signature_is_still_valid(&new_fork),
            "signature should be invalid when fork version changed for the epoch"
        );
    }

    #[test]
    fn signature_valid_when_epoch_still_in_previous_fork() {
        let exit = make_exit(2);
        // Verified with previous_version [1,0,0,0] (epoch 2 < fork epoch 10).
        let verified = make_verified_op(exit, smallvec![[1, 0, 0, 0]]);

        // Fork still has same previous_version for epoch 2.
        let fork = make_fork([1, 0, 0, 0], [5, 0, 0, 0], 10);
        assert!(verified.signature_is_still_valid(&fork));
    }

    #[test]
    fn signature_valid_when_epoch_in_current_fork() {
        let exit = make_exit(15);
        // Verified with current_version [2,0,0,0] (epoch 15 >= fork epoch 10).
        let verified = make_verified_op(exit, smallvec![[2, 0, 0, 0]]);

        let fork = make_fork([1, 0, 0, 0], [2, 0, 0, 0], 10);
        assert!(verified.signature_is_still_valid(&fork));
    }

    #[test]
    fn signature_invalid_when_fork_epoch_shifts_past_message_epoch() {
        // Exit at epoch 5. Originally verified when epoch 5 was in current fork (fork epoch 3).
        // After a new fork, epoch 5 is now in the previous fork with a different version.
        let exit = make_exit(5);
        let verified = make_verified_op(exit, smallvec![[2, 0, 0, 0]]);

        // New fork: epoch 5 < fork epoch 8, so get_fork_version(5) = previous = [3,0,0,0].
        let new_fork = make_fork([3, 0, 0, 0], [4, 0, 0, 0], 8);
        assert!(!verified.signature_is_still_valid(&new_fork));
    }

    #[test]
    fn signature_valid_with_empty_verification_epochs() {
        // BLS-to-execution changes have no verification epochs → always valid.
        let change = make_bls_change();
        let verified = make_verified_op(change, smallvec![]);

        let fork = make_fork([99, 0, 0, 0], [100, 0, 0, 0], 50);
        assert!(
            verified.signature_is_still_valid(&fork),
            "empty verification epochs should always pass"
        );
    }

    #[test]
    fn signature_valid_with_two_matching_fork_versions() {
        // Attester slashing: both attestation epochs still map to same fork versions.
        let slashing = make_attester_slashing(3, 12);

        // Fork: epoch 3 < 10 → previous [1,0,0,0]; epoch 12 >= 10 → current [2,0,0,0].
        let verified = make_verified_op(slashing, smallvec![[1, 0, 0, 0], [2, 0, 0, 0]]);
        let fork = make_fork([1, 0, 0, 0], [2, 0, 0, 0], 10);
        assert!(verified.signature_is_still_valid(&fork));
    }

    #[test]
    fn signature_invalid_when_one_of_two_fork_versions_changes() {
        // Attester slashing: first attestation epoch still valid, second changed.
        let slashing = make_attester_slashing(3, 12);

        // Originally: epoch 3 → [1,0,0,0], epoch 12 → [2,0,0,0].
        let verified = make_verified_op(slashing, smallvec![[1, 0, 0, 0], [2, 0, 0, 0]]);

        // New fork: epoch 12 >= 5, so current = [3,0,0,0] ≠ [2,0,0,0].
        let new_fork = make_fork([1, 0, 0, 0], [3, 0, 0, 0], 5);
        assert!(
            !verified.signature_is_still_valid(&new_fork),
            "should be invalid when second fork version no longer matches"
        );
    }

    // ── into_inner / as_inner / first_fork_verified_against tests ──

    #[test]
    fn into_inner_returns_original_op() {
        let exit = make_exit(7);
        let verified = make_verified_op(exit.clone(), smallvec![[1, 0, 0, 0]]);
        let recovered = verified.into_inner();
        assert_eq!(recovered.message.epoch, Epoch::new(7));
        assert_eq!(recovered.message.validator_index, 0);
    }

    #[test]
    fn as_inner_returns_reference_to_op() {
        let exit = make_exit(7);
        let verified = make_verified_op(exit.clone(), smallvec![[1, 0, 0, 0]]);
        assert_eq!(verified.as_inner().message.epoch, Epoch::new(7));
    }

    #[test]
    fn first_fork_verified_against_returns_first_version() {
        let exit = make_exit(0);
        let verified = make_verified_op(exit, smallvec![[42, 0, 0, 0]]);
        assert_eq!(verified.first_fork_verified_against(), Some([42, 0, 0, 0]));
    }

    #[test]
    fn first_fork_verified_against_none_when_empty() {
        let change = make_bls_change();
        let verified = make_verified_op(change, smallvec![]);
        assert_eq!(verified.first_fork_verified_against(), None);
    }

    #[test]
    fn first_fork_verified_against_with_multiple_versions() {
        let exit = make_exit(0);
        let verified = make_verified_op(exit, smallvec![[10, 0, 0, 0], [20, 0, 0, 0]]);
        assert_eq!(verified.first_fork_verified_against(), Some([10, 0, 0, 0]));
    }
}
