// Copyright (c) 2022 Reth Contributors
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
// THE SOFTWARE.
use crate::{Address, EthSpec, ExecutionPayloadRef, Hash64, Hash256, Uint256};
use alloy_rlp::RlpEncodable;
use metastruct::metastruct;

/// Execution block header as used for RLP encoding and Keccak hashing.
///
/// Credit to Reth for the type definition.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[metastruct(mappings(map_execution_block_header_fields_base(exclude(
    withdrawals_root,
    blob_gas_used,
    excess_blob_gas,
    parent_beacon_block_root
)),))]
pub struct ExecutionBlockHeader {
    pub parent_hash: Hash256,
    pub ommers_hash: Hash256,
    pub beneficiary: Address,
    pub state_root: Hash256,
    pub transactions_root: Hash256,
    pub receipts_root: Hash256,
    pub logs_bloom: Vec<u8>,
    pub difficulty: Uint256,
    pub number: Uint256,
    pub gas_limit: Uint256,
    pub gas_used: Uint256,
    pub timestamp: u64,
    pub extra_data: Vec<u8>,
    pub mix_hash: Hash256,
    pub nonce: Hash64,
    pub base_fee_per_gas: Uint256,
    pub withdrawals_root: Option<Hash256>,
    pub blob_gas_used: Option<u64>,
    pub excess_blob_gas: Option<u64>,
    pub parent_beacon_block_root: Option<Hash256>,
    pub requests_root: Option<Hash256>,
}

impl ExecutionBlockHeader {
    #[allow(clippy::too_many_arguments)]
    pub fn from_payload<E: EthSpec>(
        payload: ExecutionPayloadRef<E>,
        rlp_empty_list_root: Hash256,
        rlp_transactions_root: Hash256,
        rlp_withdrawals_root: Option<Hash256>,
        rlp_blob_gas_used: Option<u64>,
        rlp_excess_blob_gas: Option<u64>,
        rlp_parent_beacon_block_root: Option<Hash256>,
        rlp_requests_root: Option<Hash256>,
    ) -> Self {
        // Most of these field mappings are defined in EIP-3675 except for `mixHash`, which is
        // defined in EIP-4399.
        ExecutionBlockHeader {
            parent_hash: payload.parent_hash().into_root(),
            ommers_hash: rlp_empty_list_root,
            beneficiary: payload.fee_recipient(),
            state_root: payload.state_root(),
            transactions_root: rlp_transactions_root,
            receipts_root: payload.receipts_root(),
            logs_bloom: payload.logs_bloom().clone().into(),
            difficulty: Uint256::ZERO,
            number: Uint256::saturating_from(payload.block_number()),
            gas_limit: Uint256::saturating_from(payload.gas_limit()),
            gas_used: Uint256::saturating_from(payload.gas_used()),
            timestamp: payload.timestamp(),
            extra_data: payload.extra_data().clone().into(),
            mix_hash: payload.prev_randao(),
            nonce: Hash64::ZERO,
            base_fee_per_gas: payload.base_fee_per_gas(),
            withdrawals_root: rlp_withdrawals_root,
            blob_gas_used: rlp_blob_gas_used,
            excess_blob_gas: rlp_excess_blob_gas,
            parent_beacon_block_root: rlp_parent_beacon_block_root,
            requests_root: rlp_requests_root,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, RlpEncodable)]
#[rlp(trailing)]
pub struct EncodableExecutionBlockHeader<'a> {
    pub parent_hash: &'a [u8],
    pub ommers_hash: &'a [u8],
    pub beneficiary: &'a [u8],
    pub state_root: &'a [u8],
    pub transactions_root: &'a [u8],
    pub receipts_root: &'a [u8],
    pub logs_bloom: &'a [u8],
    pub difficulty: Uint256,
    pub number: Uint256,
    pub gas_limit: Uint256,
    pub gas_used: Uint256,
    pub timestamp: u64,
    pub extra_data: &'a [u8],
    pub mix_hash: &'a [u8],
    pub nonce: &'a [u8],
    pub base_fee_per_gas: Uint256,
    pub withdrawals_root: Option<&'a [u8]>,
    pub blob_gas_used: Option<u64>,
    pub excess_blob_gas: Option<u64>,
    pub parent_beacon_block_root: Option<&'a [u8]>,
    pub requests_root: Option<&'a [u8]>,
}

impl<'a> From<&'a ExecutionBlockHeader> for EncodableExecutionBlockHeader<'a> {
    fn from(header: &'a ExecutionBlockHeader) -> Self {
        let mut encodable = Self {
            parent_hash: header.parent_hash.as_slice(),
            ommers_hash: header.ommers_hash.as_slice(),
            beneficiary: header.beneficiary.as_slice(),
            state_root: header.state_root.as_slice(),
            transactions_root: header.transactions_root.as_slice(),
            receipts_root: header.receipts_root.as_slice(),
            logs_bloom: header.logs_bloom.as_slice(),
            difficulty: header.difficulty,
            number: header.number,
            gas_limit: header.gas_limit,
            gas_used: header.gas_used,
            timestamp: header.timestamp,
            extra_data: header.extra_data.as_slice(),
            mix_hash: header.mix_hash.as_slice(),
            nonce: header.nonce.as_slice(),
            base_fee_per_gas: header.base_fee_per_gas,
            withdrawals_root: None,
            blob_gas_used: header.blob_gas_used,
            excess_blob_gas: header.excess_blob_gas,
            parent_beacon_block_root: None,
            requests_root: None,
        };
        if let Some(withdrawals_root) = &header.withdrawals_root {
            encodable.withdrawals_root = Some(withdrawals_root.as_slice());
        }
        if let Some(parent_beacon_block_root) = &header.parent_beacon_block_root {
            encodable.parent_beacon_block_root = Some(parent_beacon_block_root.as_slice())
        }
        if let Some(requests_root) = &header.requests_root {
            encodable.requests_root = Some(requests_root.as_slice())
        }
        encodable
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FixedBytesExtended;
    use alloy_rlp::Encodable;

    fn sample_header() -> ExecutionBlockHeader {
        ExecutionBlockHeader {
            parent_hash: Hash256::from_low_u64_be(1),
            ommers_hash: Hash256::from_low_u64_be(2),
            beneficiary: Address::repeat_byte(0xaa),
            state_root: Hash256::from_low_u64_be(3),
            transactions_root: Hash256::from_low_u64_be(4),
            receipts_root: Hash256::from_low_u64_be(5),
            logs_bloom: vec![0u8; 256],
            difficulty: Uint256::ZERO,
            number: Uint256::saturating_from(100u64),
            gas_limit: Uint256::saturating_from(30_000_000u64),
            gas_used: Uint256::saturating_from(21_000u64),
            timestamp: 1_700_000_000,
            extra_data: vec![0x42, 0x43],
            mix_hash: Hash256::from_low_u64_be(6),
            nonce: Hash64::ZERO,
            base_fee_per_gas: Uint256::saturating_from(7u64),
            withdrawals_root: None,
            blob_gas_used: None,
            excess_blob_gas: None,
            parent_beacon_block_root: None,
            requests_root: None,
        }
    }

    #[test]
    fn header_equality() {
        let h1 = sample_header();
        let h2 = sample_header();
        assert_eq!(h1, h2);
    }

    #[test]
    fn header_inequality_on_different_field() {
        let h1 = sample_header();
        let mut h2 = sample_header();
        h2.timestamp = 999;
        assert_ne!(h1, h2);
    }

    #[test]
    fn header_clone() {
        let h1 = sample_header();
        let h2 = h1.clone();
        assert_eq!(h1, h2);
    }

    #[test]
    fn encodable_conversion_no_optional_fields() {
        let header = sample_header();
        let encodable = EncodableExecutionBlockHeader::from(&header);
        assert_eq!(encodable.parent_hash, header.parent_hash.as_slice());
        assert_eq!(encodable.beneficiary, header.beneficiary.as_slice());
        assert_eq!(encodable.timestamp, header.timestamp);
        assert_eq!(encodable.base_fee_per_gas, header.base_fee_per_gas);
        assert!(encodable.withdrawals_root.is_none());
        assert!(encodable.blob_gas_used.is_none());
        assert!(encodable.excess_blob_gas.is_none());
        assert!(encodable.parent_beacon_block_root.is_none());
        assert!(encodable.requests_root.is_none());
    }

    #[test]
    fn encodable_conversion_with_optional_fields() {
        let mut header = sample_header();
        let wr = Hash256::from_low_u64_be(10);
        let pbr = Hash256::from_low_u64_be(11);
        let rr = Hash256::from_low_u64_be(12);
        header.withdrawals_root = Some(wr);
        header.blob_gas_used = Some(131072);
        header.excess_blob_gas = Some(0);
        header.parent_beacon_block_root = Some(pbr);
        header.requests_root = Some(rr);

        let encodable = EncodableExecutionBlockHeader::from(&header);
        assert_eq!(encodable.withdrawals_root, Some(wr.as_slice()));
        assert_eq!(encodable.blob_gas_used, Some(131072));
        assert_eq!(encodable.excess_blob_gas, Some(0));
        assert_eq!(encodable.parent_beacon_block_root, Some(pbr.as_slice()));
        assert_eq!(encodable.requests_root, Some(rr.as_slice()));
    }

    #[test]
    fn encodable_rlp_deterministic() {
        let header = sample_header();
        let encodable = EncodableExecutionBlockHeader::from(&header);
        let mut buf1 = Vec::new();
        let mut buf2 = Vec::new();
        encodable.encode(&mut buf1);
        encodable.encode(&mut buf2);
        assert_eq!(buf1, buf2);
        assert!(!buf1.is_empty());
    }

    #[test]
    fn encodable_rlp_different_headers_differ() {
        let h1 = sample_header();
        let mut h2 = sample_header();
        h2.number = Uint256::saturating_from(999u64);
        let e1 = EncodableExecutionBlockHeader::from(&h1);
        let e2 = EncodableExecutionBlockHeader::from(&h2);
        let mut buf1 = Vec::new();
        let mut buf2 = Vec::new();
        e1.encode(&mut buf1);
        e2.encode(&mut buf2);
        assert_ne!(buf1, buf2);
    }

    #[test]
    fn encodable_with_partial_optionals() {
        // Only withdrawals_root set, others None
        let mut header = sample_header();
        header.withdrawals_root = Some(Hash256::from_low_u64_be(77));
        let encodable = EncodableExecutionBlockHeader::from(&header);
        assert!(encodable.withdrawals_root.is_some());
        assert!(encodable.parent_beacon_block_root.is_none());
        assert!(encodable.requests_root.is_none());
        // blob fields are independent
        assert!(encodable.blob_gas_used.is_none());
    }

    #[test]
    fn header_debug_and_hash() {
        use std::collections::HashSet;
        let h1 = sample_header();
        let mut h2 = sample_header();
        h2.timestamp = 42;
        let mut set = HashSet::new();
        set.insert(h1.clone());
        assert!(set.contains(&h1));
        assert!(!set.contains(&h2));
        // Debug doesn't panic
        let _ = format!("{:?}", h1);
    }

    #[test]
    fn difficulty_and_nonce_are_zero() {
        // Post-merge headers should have zero difficulty and nonce
        let header = sample_header();
        assert_eq!(header.difficulty, Uint256::ZERO);
        assert_eq!(header.nonce, Hash64::ZERO);
    }
}
