use beacon_chain::{BeaconBlockResponse, BeaconBlockResponseWrapper, BlockProductionError};
use eth2::types::{BlockContents, FullBlockContents, ProduceBlockV3Response};
use types::{EthSpec, ForkName};
type Error = warp::reject::Rejection;

pub fn build_block_contents<E: EthSpec>(
    fork_name: ForkName,
    block_response: BeaconBlockResponseWrapper<E>,
) -> Result<ProduceBlockV3Response<E>, Error> {
    match block_response {
        BeaconBlockResponseWrapper::Blinded(block) => {
            Ok(ProduceBlockV3Response::Blinded(block.block))
        }

        BeaconBlockResponseWrapper::Full(block) => {
            if fork_name.deneb_enabled() {
                let BeaconBlockResponse {
                    block,
                    state: _,
                    blob_items,
                    execution_payload_value: _,
                    consensus_block_value: _,
                    execution_payload_envelope,
                } = block;

                // Gloas ePBS: KZG commitments are in the bid (not the block body), so
                // blob_items may be None when there are no blobs. Use empty proofs/blobs.
                let (kzg_proofs, blobs) = if fork_name.gloas_enabled() {
                    blob_items.unwrap_or_default()
                } else {
                    let Some(items) = blob_items else {
                        return Err(warp_utils::reject::unhandled_error(
                            BlockProductionError::MissingBlobs,
                        ));
                    };
                    items
                };

                // Extract the unsigned envelope message from the
                // SignedExecutionPayloadEnvelope (signature is a placeholder).
                let unsigned_envelope = execution_payload_envelope.map(|signed| signed.message);

                Ok(ProduceBlockV3Response::Full(
                    FullBlockContents::BlockContents(BlockContents {
                        block,
                        kzg_proofs,
                        blobs,
                        execution_payload_envelope: unsigned_envelope,
                    }),
                ))
            } else {
                Ok(ProduceBlockV3Response::Full(FullBlockContents::Block(
                    block.block,
                )))
            }
        }
    }
}
