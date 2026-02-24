use crate::{
    EthSpec, ExecutionPayloadEnvelope, ExecutionPayloadGloas, ExecutionPayloadHeaderGloas,
    ExecutionRequests, Hash256, Slot, Withdrawals,
};
use derivative::Derivative;
use serde::{Deserialize, Serialize};
use ssz_derive::{Decode, Encode};
use tree_hash_derive::TreeHash;

/// Blinded execution payload envelope for database storage.
///
/// Same as `ExecutionPayloadEnvelope` but with `ExecutionPayloadHeaderGloas`
/// (transactions_root, withdrawals_root) instead of the full
/// `ExecutionPayloadGloas` (transactions list, withdrawals list).
///
/// This allows pruning the large transaction data while keeping enough
/// envelope metadata for block replay and state reconstruction.
#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, TreeHash, Derivative)]
#[derivative(PartialEq, Hash)]
#[serde(bound = "E: EthSpec")]
pub struct BlindedExecutionPayloadEnvelope<E: EthSpec> {
    /// The execution payload header (blinded: no transactions/withdrawals lists)
    pub payload_header: ExecutionPayloadHeaderGloas<E>,
    /// Execution layer requests (deposits, withdrawals, consolidations)
    pub execution_requests: ExecutionRequests<E>,
    /// Index of the builder revealing this payload
    #[serde(with = "serde_utils::quoted_u64")]
    pub builder_index: u64,
    /// Root of the beacon block this payload is for
    pub beacon_block_root: Hash256,
    /// Slot this payload is for (must match the committed bid)
    pub slot: Slot,
    /// Beacon state root after processing this payload
    pub state_root: Hash256,
}

impl<E: EthSpec> BlindedExecutionPayloadEnvelope<E> {
    /// Create from a full `ExecutionPayloadEnvelope` by replacing the payload
    /// with its header (transactions → transactions_root, withdrawals → withdrawals_root).
    pub fn from_full(envelope: &ExecutionPayloadEnvelope<E>) -> Self {
        Self {
            payload_header: ExecutionPayloadHeaderGloas::from(&envelope.payload),
            execution_requests: envelope.execution_requests.clone(),
            builder_index: envelope.builder_index,
            beacon_block_root: envelope.beacon_block_root,
            slot: envelope.slot,
            state_root: envelope.state_root,
        }
    }

    /// Reconstruct a full `ExecutionPayloadEnvelope` by combining this blinded
    /// envelope with the full payload.
    pub fn into_full(self, payload: ExecutionPayloadGloas<E>) -> ExecutionPayloadEnvelope<E> {
        ExecutionPayloadEnvelope {
            payload,
            execution_requests: self.execution_requests,
            builder_index: self.builder_index,
            beacon_block_root: self.beacon_block_root,
            slot: self.slot,
            state_root: self.state_root,
        }
    }

    /// Reconstruct a full `ExecutionPayloadEnvelope` using the header fields
    /// and externally supplied withdrawals. Transactions are set to empty
    /// (they are not needed during state processing / block replay).
    ///
    /// This is used for block replay of finalized blocks where the full
    /// payload has been pruned but the blinded envelope is retained.
    pub fn into_full_with_withdrawals(
        self,
        withdrawals: Withdrawals<E>,
    ) -> ExecutionPayloadEnvelope<E> {
        let h = &self.payload_header;
        let payload = ExecutionPayloadGloas {
            parent_hash: h.parent_hash,
            fee_recipient: h.fee_recipient,
            state_root: h.state_root,
            receipts_root: h.receipts_root,
            logs_bloom: h.logs_bloom.clone(),
            prev_randao: h.prev_randao,
            block_number: h.block_number,
            gas_limit: h.gas_limit,
            gas_used: h.gas_used,
            timestamp: h.timestamp,
            extra_data: h.extra_data.clone(),
            base_fee_per_gas: h.base_fee_per_gas,
            block_hash: h.block_hash,
            transactions: Default::default(),
            withdrawals,
            blob_gas_used: h.blob_gas_used,
            excess_blob_gas: h.excess_blob_gas,
        };
        self.into_full(payload)
    }
}

/// Signed blinded execution payload envelope for database storage.
///
/// Same as `SignedExecutionPayloadEnvelope` but wraps a
/// `BlindedExecutionPayloadEnvelope` instead of the full version.
#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, TreeHash, Derivative)]
#[derivative(PartialEq, Hash)]
#[serde(bound = "E: EthSpec")]
pub struct SignedBlindedExecutionPayloadEnvelope<E: EthSpec> {
    /// The blinded execution payload envelope
    pub message: BlindedExecutionPayloadEnvelope<E>,
    /// BLS signature from the builder
    pub signature: bls::Signature,
}

impl<E: EthSpec> SignedBlindedExecutionPayloadEnvelope<E> {
    /// Create from a full `SignedExecutionPayloadEnvelope`.
    pub fn from_full(signed: &crate::SignedExecutionPayloadEnvelope<E>) -> Self {
        Self {
            message: BlindedExecutionPayloadEnvelope::from_full(&signed.message),
            signature: signed.signature.clone(),
        }
    }

    /// Reconstruct a full `SignedExecutionPayloadEnvelope` with the given payload.
    pub fn into_full(
        self,
        payload: ExecutionPayloadGloas<E>,
    ) -> crate::SignedExecutionPayloadEnvelope<E> {
        crate::SignedExecutionPayloadEnvelope {
            message: self.message.into_full(payload),
            signature: self.signature,
        }
    }

    /// Reconstruct a full `SignedExecutionPayloadEnvelope` using header fields
    /// and externally supplied withdrawals (transactions set to empty).
    pub fn into_full_with_withdrawals(
        self,
        withdrawals: Withdrawals<E>,
    ) -> crate::SignedExecutionPayloadEnvelope<E> {
        crate::SignedExecutionPayloadEnvelope {
            message: self.message.into_full_with_withdrawals(withdrawals),
            signature: self.signature,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Address, ExecutionBlockHash, MinimalEthSpec, SignedExecutionPayloadEnvelope, Withdrawal,
    };
    use ssz::{Decode, Encode};

    type E = MinimalEthSpec;

    #[test]
    fn blinded_from_full_preserves_metadata() {
        let full = ExecutionPayloadEnvelope::<E> {
            builder_index: 42,
            slot: Slot::new(100),
            beacon_block_root: Hash256::repeat_byte(0xaa),
            state_root: Hash256::repeat_byte(0xbb),
            payload: ExecutionPayloadGloas {
                block_hash: ExecutionBlockHash::repeat_byte(0xcc),
                ..Default::default()
            },
            ..Default::default()
        };

        let blinded = BlindedExecutionPayloadEnvelope::from_full(&full);
        assert_eq!(blinded.builder_index, 42);
        assert_eq!(blinded.slot, Slot::new(100));
        assert_eq!(blinded.beacon_block_root, Hash256::repeat_byte(0xaa));
        assert_eq!(blinded.state_root, Hash256::repeat_byte(0xbb));
        assert_eq!(
            blinded.payload_header.block_hash,
            ExecutionBlockHash::repeat_byte(0xcc)
        );
    }

    #[test]
    fn blinded_ssz_roundtrip() {
        let full = ExecutionPayloadEnvelope::<E> {
            builder_index: 7,
            slot: Slot::new(42),
            payload: ExecutionPayloadGloas {
                block_hash: ExecutionBlockHash::repeat_byte(0xdd),
                ..Default::default()
            },
            ..Default::default()
        };

        let blinded = BlindedExecutionPayloadEnvelope::from_full(&full);
        let bytes = blinded.as_ssz_bytes();
        let decoded = BlindedExecutionPayloadEnvelope::<E>::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(blinded, decoded);
    }

    #[test]
    fn signed_blinded_from_full_roundtrip() {
        let signed = SignedExecutionPayloadEnvelope::<E> {
            message: ExecutionPayloadEnvelope {
                builder_index: 99,
                slot: Slot::new(50),
                payload: ExecutionPayloadGloas {
                    block_hash: ExecutionBlockHash::repeat_byte(0xee),
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };

        let blinded = SignedBlindedExecutionPayloadEnvelope::from_full(&signed);
        assert_eq!(blinded.message.builder_index, 99);
        assert_eq!(blinded.message.slot, Slot::new(50));

        // Reconstruct with the original payload
        let reconstructed = blinded.into_full(signed.message.payload.clone());
        assert_eq!(reconstructed, signed);
    }

    #[test]
    fn signed_blinded_ssz_roundtrip() {
        let signed = SignedExecutionPayloadEnvelope::<E> {
            message: ExecutionPayloadEnvelope {
                builder_index: 55,
                ..Default::default()
            },
            ..Default::default()
        };

        let blinded = SignedBlindedExecutionPayloadEnvelope::from_full(&signed);
        let bytes = blinded.as_ssz_bytes();
        let decoded = SignedBlindedExecutionPayloadEnvelope::<E>::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(blinded, decoded);
    }

    #[test]
    fn into_full_with_withdrawals_sets_empty_transactions() {
        let full = ExecutionPayloadEnvelope::<E> {
            builder_index: 10,
            payload: ExecutionPayloadGloas {
                block_hash: ExecutionBlockHash::repeat_byte(0xff),
                ..Default::default()
            },
            ..Default::default()
        };

        let blinded = BlindedExecutionPayloadEnvelope::from_full(&full);
        let reconstructed = blinded.into_full_with_withdrawals(Default::default());
        assert_eq!(reconstructed.builder_index, 10);
        assert_eq!(
            reconstructed.payload.block_hash,
            ExecutionBlockHash::repeat_byte(0xff)
        );
        assert!(reconstructed.payload.transactions.is_empty());
    }

    #[test]
    fn blinded_header_has_correct_roots() {
        let full = ExecutionPayloadEnvelope::<E> {
            payload: ExecutionPayloadGloas {
                block_hash: ExecutionBlockHash::repeat_byte(0x11),
                ..Default::default()
            },
            ..Default::default()
        };

        let blinded = BlindedExecutionPayloadEnvelope::from_full(&full);
        let expected_header = ExecutionPayloadHeaderGloas::from(&full.payload);
        assert_eq!(
            blinded.payload_header.transactions_root,
            expected_header.transactions_root
        );
        assert_eq!(
            blinded.payload_header.withdrawals_root,
            expected_header.withdrawals_root
        );
    }

    #[test]
    fn into_full_with_nonempty_withdrawals() {
        let full = ExecutionPayloadEnvelope::<E> {
            builder_index: 5,
            slot: Slot::new(42),
            beacon_block_root: Hash256::repeat_byte(0xaa),
            state_root: Hash256::repeat_byte(0xbb),
            payload: ExecutionPayloadGloas {
                block_hash: ExecutionBlockHash::repeat_byte(0xcc),
                fee_recipient: Address::repeat_byte(0x11),
                gas_limit: 30_000_000,
                timestamp: 1700000000,
                ..Default::default()
            },
            ..Default::default()
        };

        let blinded = BlindedExecutionPayloadEnvelope::from_full(&full);

        // Create non-empty withdrawals
        let mut withdrawals = Withdrawals::<E>::default();
        withdrawals
            .push(Withdrawal {
                index: 0,
                validator_index: 7,
                address: Address::repeat_byte(0xdd),
                amount: 1_000_000_000,
            })
            .unwrap();
        withdrawals
            .push(Withdrawal {
                index: 1,
                validator_index: 12,
                address: Address::repeat_byte(0xee),
                amount: 2_000_000_000,
            })
            .unwrap();

        let reconstructed = blinded.into_full_with_withdrawals(withdrawals);

        // Metadata preserved
        assert_eq!(reconstructed.builder_index, 5);
        assert_eq!(reconstructed.slot, Slot::new(42));
        assert_eq!(reconstructed.beacon_block_root, Hash256::repeat_byte(0xaa));
        assert_eq!(reconstructed.state_root, Hash256::repeat_byte(0xbb));

        // Header fields transferred to payload
        assert_eq!(
            reconstructed.payload.block_hash,
            ExecutionBlockHash::repeat_byte(0xcc)
        );
        assert_eq!(
            reconstructed.payload.fee_recipient,
            Address::repeat_byte(0x11)
        );
        assert_eq!(reconstructed.payload.gas_limit, 30_000_000);
        assert_eq!(reconstructed.payload.timestamp, 1700000000);

        // Withdrawals populated (not empty)
        assert_eq!(reconstructed.payload.withdrawals.len(), 2);
        assert_eq!(reconstructed.payload.withdrawals[0].validator_index, 7);
        assert_eq!(reconstructed.payload.withdrawals[0].amount, 1_000_000_000);
        assert_eq!(
            reconstructed.payload.withdrawals[0].address,
            Address::repeat_byte(0xdd)
        );
        assert_eq!(reconstructed.payload.withdrawals[1].validator_index, 12);
        assert_eq!(reconstructed.payload.withdrawals[1].amount, 2_000_000_000);

        // Transactions should be empty (pruned)
        assert!(reconstructed.payload.transactions.is_empty());
    }

    #[test]
    fn signed_into_full_with_nonempty_withdrawals() {
        let signed = SignedExecutionPayloadEnvelope::<E> {
            message: ExecutionPayloadEnvelope {
                builder_index: 3,
                slot: Slot::new(10),
                payload: ExecutionPayloadGloas {
                    block_hash: ExecutionBlockHash::repeat_byte(0xff),
                    gas_used: 15_000_000,
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };

        let blinded = SignedBlindedExecutionPayloadEnvelope::from_full(&signed);

        let mut withdrawals = Withdrawals::<E>::default();
        withdrawals
            .push(Withdrawal {
                index: 5,
                validator_index: 99,
                address: Address::repeat_byte(0xab),
                amount: 500_000_000,
            })
            .unwrap();

        let reconstructed = blinded.into_full_with_withdrawals(withdrawals);
        assert_eq!(reconstructed.message.builder_index, 3);
        assert_eq!(reconstructed.message.slot, Slot::new(10));
        assert_eq!(reconstructed.message.payload.gas_used, 15_000_000);
        assert_eq!(reconstructed.message.payload.withdrawals.len(), 1);
        assert_eq!(
            reconstructed.message.payload.withdrawals[0].amount,
            500_000_000
        );
        assert!(reconstructed.message.payload.transactions.is_empty());
    }

    #[test]
    fn blinded_preserves_execution_requests() {
        use crate::DepositRequest;

        let mut full = ExecutionPayloadEnvelope::<E>::empty();
        full.builder_index = 8;

        // Add a non-default execution request (deposit)
        full.execution_requests
            .deposits
            .push(DepositRequest {
                pubkey: crate::PublicKeyBytes::empty(),
                withdrawal_credentials: Hash256::repeat_byte(0x01),
                amount: 32_000_000_000,
                signature: crate::SignatureBytes::empty(),
                index: 42,
            })
            .unwrap();

        let blinded = BlindedExecutionPayloadEnvelope::from_full(&full);
        assert_eq!(blinded.execution_requests.deposits.len(), 1);
        assert_eq!(
            blinded.execution_requests.deposits[0].amount,
            32_000_000_000
        );
        assert_eq!(blinded.execution_requests.deposits[0].index, 42);

        // Roundtrip via into_full preserves execution_requests
        let reconstructed = blinded.into_full_with_withdrawals(Default::default());
        assert_eq!(reconstructed.execution_requests.deposits.len(), 1);
        assert_eq!(
            reconstructed.execution_requests.deposits[0].amount,
            32_000_000_000
        );
    }

    #[test]
    fn blinded_ssz_roundtrip_with_execution_requests() {
        use crate::DepositRequest;

        let mut full = ExecutionPayloadEnvelope::<E>::empty();
        full.execution_requests
            .deposits
            .push(DepositRequest {
                pubkey: crate::PublicKeyBytes::empty(),
                withdrawal_credentials: Hash256::repeat_byte(0x03),
                amount: 64_000_000_000,
                signature: crate::SignatureBytes::empty(),
                index: 7,
            })
            .unwrap();

        let blinded = BlindedExecutionPayloadEnvelope::from_full(&full);
        let bytes = blinded.as_ssz_bytes();
        let decoded = BlindedExecutionPayloadEnvelope::<E>::from_ssz_bytes(&bytes).unwrap();
        assert_eq!(blinded, decoded);
        assert_eq!(decoded.execution_requests.deposits.len(), 1);
        assert_eq!(
            decoded.execution_requests.deposits[0].amount,
            64_000_000_000
        );
    }
}
