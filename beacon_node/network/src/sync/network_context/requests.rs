use std::time::Instant;
use std::{collections::hash_map::Entry, hash::Hash};

use beacon_chain::validator_monitor::timestamp_now;
use fnv::FnvHashMap;
use strum::IntoStaticStr;
use tracing::{Span, debug};
use types::{Hash256, Slot};
use vibehouse_network::PeerId;

pub(crate) use blobs_by_range::BlobsByRangeRequestItems;
pub(crate) use blobs_by_root::{BlobsByRootRequestItems, BlobsByRootSingleBlockRequest};
pub(crate) use blocks_by_range::BlocksByRangeRequestItems;
pub(crate) use blocks_by_root::{BlocksByRootRequestItems, BlocksByRootSingleRequest};
pub(crate) use data_columns_by_range::DataColumnsByRangeRequestItems;
pub(crate) use data_columns_by_root::{
    DataColumnsByRootRequestItems, DataColumnsByRootSingleBlockRequest,
};

use crate::metrics;

use super::{RpcEvent, RpcResponseError, RpcResponseResult};

mod blobs_by_range;
mod blobs_by_root;
mod blocks_by_range;
mod blocks_by_root;
mod data_columns_by_range;
mod data_columns_by_root;

#[derive(Debug, PartialEq, Eq, IntoStaticStr)]
pub(crate) enum LookupVerifyError {
    NotEnoughResponsesReturned { actual: usize },
    UnrequestedBlockRoot(Hash256),
    UnrequestedIndex(u64),
    UnrequestedSlot(Slot),
    InvalidInclusionProof,
    DuplicatedData(Slot, u64),
    InternalError(String),
}

/// Collection of active requests of a single ReqResp method, i.e. `blocks_by_root`
pub(crate) struct ActiveRequests<K: Eq + Hash, T: ActiveRequestItems> {
    requests: FnvHashMap<K, ActiveRequest<T>>,
    name: &'static str,
}

/// Stateful container for a single active ReqResp request
struct ActiveRequest<T: ActiveRequestItems> {
    state: State<T>,
    peer_id: PeerId,
    // Error if the request terminates before receiving max expected responses
    expect_max_responses: bool,
    start_instant: Instant,
    span: Span,
}

enum State<T> {
    Active(T),
    CompletedEarly,
    Errored,
}

impl<K: Copy + Eq + Hash + std::fmt::Display, T: ActiveRequestItems> ActiveRequests<K, T> {
    pub(super) fn new(name: &'static str) -> Self {
        Self {
            requests: <_>::default(),
            name,
        }
    }

    pub(super) fn insert(
        &mut self,
        id: K,
        peer_id: PeerId,
        expect_max_responses: bool,
        items: T,
        span: Span,
    ) {
        let _guard = span.clone().entered();
        self.requests.insert(
            id,
            ActiveRequest {
                state: State::Active(items),
                peer_id,
                expect_max_responses,
                start_instant: Instant::now(),
                span,
            },
        );
    }

    /// Handle an `RpcEvent` for a specific request index by `id`.
    ///
    /// Vibehouse ReqResp protocol API promises to send 0 or more `RpcEvent::Response` chunks,
    /// and EITHER a single `RpcEvent::RPCError` or RpcEvent::StreamTermination.
    ///
    /// Downstream code expects to receive a single `Result` value per request ID. However,
    /// `add_item` may convert ReqResp success chunks into errors. This function handles the
    /// multiple errors / stream termination internally ensuring that a single `Some<Result>` is
    /// returned.
    ///
    /// ## Returns
    /// - `Some` if the request has either completed or errored, and needs to be actioned by the
    ///   caller.
    /// - `None` if no further action is currently needed.
    pub(super) fn on_response(
        &mut self,
        id: K,
        rpc_event: RpcEvent<T::Item>,
    ) -> Option<RpcResponseResult<Vec<T::Item>>> {
        let Entry::Occupied(mut entry) = self.requests.entry(id) else {
            metrics::inc_counter_vec(&metrics::SYNC_UNKNOWN_NETWORK_REQUESTS, &[self.name]);
            return None;
        };

        let result = match rpc_event {
            // Handler of a success ReqResp chunk. Adds the item to the request accumulator.
            // `ActiveRequestItems` validates the item before appending to its internal state.
            RpcEvent::Response(item, seen_timestamp) => {
                let request = &mut *entry.get_mut();
                let _guard = request.span.clone().entered();
                match &mut request.state {
                    State::Active(items) => {
                        match items.add(item) {
                            // Received all items we are expecting for, return early, but keep the request
                            // struct to handle the stream termination gracefully.
                            Ok(true) => {
                                let items = items.consume();
                                request.state = State::CompletedEarly;
                                Some(Ok((items, seen_timestamp, request.start_instant.elapsed())))
                            }
                            // Received item, but we are still expecting more
                            Ok(false) => None,
                            // Received an invalid item
                            Err(e) => {
                                request.state = State::Errored;
                                Some(Err(e.into()))
                            }
                        }
                    }
                    // Should never happen, ReqResp network behaviour enforces a max count of chunks
                    // When `max_remaining_chunks <= 1` a the inbound stream in terminated in
                    // `rpc/handler.rs`. Handling this case adds complexity for no gain. Even if an
                    // attacker could abuse this, there's no gain in sending garbage chunks that
                    // will be ignored anyway.
                    // Ignore items after completion or errors. We may want to penalize repeated
                    // invalid chunks for the same response. But that's an optimization to ban
                    // peers sending invalid data faster that we choose to not adopt for now.
                    State::CompletedEarly | State::Errored => None,
                }
            }
            RpcEvent::StreamTermination => {
                // After stream termination we must forget about this request, there will be no more
                // messages coming from the network
                let request = entry.remove();
                let _guard = request.span.clone().entered();
                match request.state {
                    // Received a stream termination in a valid sequence, consume items
                    State::Active(mut items) => {
                        if request.expect_max_responses {
                            Some(Err(LookupVerifyError::NotEnoughResponsesReturned {
                                actual: items.consume().len(),
                            }
                            .into()))
                        } else {
                            Some(Ok((
                                items.consume(),
                                timestamp_now(),
                                request.start_instant.elapsed(),
                            )))
                        }
                    }
                    // Items already returned or error earlier, ignore stream termination
                    State::CompletedEarly | State::Errored => None,
                }
            }
            RpcEvent::RPCError(e) => {
                // After an Error event from the network we must forget about this request as this
                // may be the last message for this request.
                let request = entry.remove();
                let _guard = request.span.clone().entered();
                match request.state {
                    // Received error while request is still active, propagate error.
                    State::Active(_) => Some(Err(e.into())),
                    // Received error after completing the request, ignore the error. This is okay
                    // because the network has already registered a downscore event if necessary for
                    // this message.
                    // Received error after completing or after a validity error. Okay to ignore.
                    State::CompletedEarly | State::Errored => None,
                }
            }
        };

        result.map(|result| match result {
            Ok((items, seen_timestamp, duration)) => {
                metrics::inc_counter_vec(&metrics::SYNC_RPC_REQUEST_SUCCESSES, &[self.name]);
                metrics::observe_timer_vec(&metrics::SYNC_RPC_REQUEST_TIME, &[self.name], duration);
                debug!(
                    %id,
                    method = self.name,
                    count = items.len(),
                    "Sync RPC request completed"
                );

                Ok((items, seen_timestamp))
            }
            Err(e) => {
                let err_str: &'static str = match &e {
                    RpcResponseError::Rpc(e) => e.into(),
                    RpcResponseError::Verify(e) => e.into(),
                    RpcResponseError::CustodyRequest(_) => "CustodyRequestError",
                    RpcResponseError::BlockComponentCoupling(_) => "BlockComponentCouplingError",
                };
                metrics::inc_counter_vec(&metrics::SYNC_RPC_REQUEST_ERRORS, &[self.name, err_str]);
                debug!(
                    %id,
                    method = self.name,
                    error = ?e,
                    "Sync RPC request error"
                );

                Err(e)
            }
        })
    }

    pub(super) fn active_requests_of_peer(&self, peer_id: &PeerId) -> Vec<&K> {
        self.requests
            .iter()
            .filter(|(_, request)| &request.peer_id == peer_id)
            .map(|(id, _)| id)
            .collect()
    }

    pub(super) fn iter_request_peers(&self) -> impl Iterator<Item = PeerId> + '_ {
        self.requests.values().map(|request| request.peer_id)
    }

    pub(super) fn len(&self) -> usize {
        self.requests.len()
    }
}

pub(crate) trait ActiveRequestItems {
    type Item;

    /// Add a new item into the accumulator. Returns true if all expected items have been received.
    fn add(&mut self, item: Self::Item) -> Result<bool, LookupVerifyError>;

    /// Return all accumulated items consuming them.
    fn consume(&mut self) -> Vec<Self::Item>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use vibehouse_network::rpc::RPCError;

    /// Simple mock that accepts N items then signals completion.
    struct MockRequestItems {
        expected: usize,
        items: Vec<u32>,
        /// If set, rejects items with this error.
        reject: bool,
    }

    impl MockRequestItems {
        fn new(expected: usize) -> Self {
            Self {
                expected,
                items: vec![],
                reject: false,
            }
        }

        fn rejecting() -> Self {
            Self {
                expected: 1,
                items: vec![],
                reject: true,
            }
        }
    }

    impl ActiveRequestItems for MockRequestItems {
        type Item = u32;

        fn add(&mut self, item: Self::Item) -> Result<bool, LookupVerifyError> {
            if self.reject {
                return Err(LookupVerifyError::UnrequestedIndex(u64::from(item)));
            }
            self.items.push(item);
            Ok(self.items.len() >= self.expected)
        }

        fn consume(&mut self) -> Vec<Self::Item> {
            std::mem::take(&mut self.items)
        }
    }

    fn make_active_requests() -> ActiveRequests<u64, MockRequestItems> {
        ActiveRequests::new("test")
    }

    fn response(item: u32) -> RpcEvent<u32> {
        RpcEvent::Response(item, Duration::from_secs(0))
    }

    fn stream_termination() -> RpcEvent<u32> {
        RpcEvent::StreamTermination
    }

    fn rpc_error() -> RpcEvent<u32> {
        RpcEvent::RPCError(RPCError::StreamTimeout)
    }

    #[test]
    fn response_completes_early_when_all_items_received() {
        let mut reqs = make_active_requests();
        let peer = PeerId::random();
        // Request expects 2 items
        reqs.insert(1, peer, false, MockRequestItems::new(2), Span::none());
        assert_eq!(reqs.len(), 1);

        // First item — not complete yet
        assert!(reqs.on_response(1, response(10)).is_none());

        // Second item — completes early
        let result = reqs.on_response(1, response(20));
        assert!(result.is_some());
        let (items, _seen) = result.unwrap().unwrap();
        assert_eq!(items, vec![10, 20]);

        // Stream termination after early completion is ignored
        assert!(reqs.on_response(1, stream_termination()).is_none());
        // Request removed after stream termination
        assert_eq!(reqs.len(), 0);
    }

    #[test]
    fn stream_termination_returns_items_when_not_expecting_max() {
        let mut reqs = make_active_requests();
        let peer = PeerId::random();
        // expect_max_responses=false, so partial response is OK
        reqs.insert(1, peer, false, MockRequestItems::new(3), Span::none());

        reqs.on_response(1, response(10));

        let result = reqs.on_response(1, stream_termination());
        assert!(result.is_some());
        let (items, _) = result.unwrap().unwrap();
        assert_eq!(items, vec![10]);
        assert_eq!(reqs.len(), 0);
    }

    #[test]
    fn stream_termination_errors_when_expecting_max_responses() {
        let mut reqs = make_active_requests();
        let peer = PeerId::random();
        // expect_max_responses=true — stream termination before all items is an error
        reqs.insert(1, peer, true, MockRequestItems::new(3), Span::none());

        reqs.on_response(1, response(10));

        let result = reqs.on_response(1, stream_termination());
        assert!(result.is_some());
        assert!(result.unwrap().is_err());
        assert_eq!(reqs.len(), 0);
    }

    #[test]
    fn rpc_error_propagated_for_active_request() {
        let mut reqs = make_active_requests();
        let peer = PeerId::random();
        reqs.insert(1, peer, false, MockRequestItems::new(2), Span::none());

        let result = reqs.on_response(1, rpc_error());
        assert!(result.is_some());
        assert!(result.unwrap().is_err());
        assert_eq!(reqs.len(), 0);
    }

    #[test]
    fn rpc_error_after_early_completion_is_ignored() {
        let mut reqs = make_active_requests();
        let peer = PeerId::random();
        reqs.insert(1, peer, false, MockRequestItems::new(1), Span::none());

        // Complete early
        let result = reqs.on_response(1, response(10));
        assert!(result.is_some());
        assert!(result.unwrap().is_ok());

        // Error after completion — ignored
        assert!(reqs.on_response(1, rpc_error()).is_none());
        assert_eq!(reqs.len(), 0);
    }

    #[test]
    fn invalid_item_transitions_to_errored_state() {
        let mut reqs = make_active_requests();
        let peer = PeerId::random();
        reqs.insert(1, peer, false, MockRequestItems::rejecting(), Span::none());

        // Invalid item triggers error
        let result = reqs.on_response(1, response(42));
        assert!(result.is_some());
        assert!(result.unwrap().is_err());

        // Subsequent responses are ignored (errored state)
        assert!(reqs.on_response(1, response(43)).is_none());

        // Stream termination after error is ignored and cleans up
        assert!(reqs.on_response(1, stream_termination()).is_none());
        assert_eq!(reqs.len(), 0);
    }

    #[test]
    fn unknown_request_id_returns_none() {
        let mut reqs = make_active_requests();
        assert!(reqs.on_response(999, response(10)).is_none());
    }

    #[test]
    fn active_requests_of_peer_returns_matching_ids() {
        let mut reqs = make_active_requests();
        let peer_a = PeerId::random();
        let peer_b = PeerId::random();
        reqs.insert(1, peer_a, false, MockRequestItems::new(1), Span::none());
        reqs.insert(2, peer_b, false, MockRequestItems::new(1), Span::none());
        reqs.insert(3, peer_a, false, MockRequestItems::new(1), Span::none());

        let mut ids = reqs.active_requests_of_peer(&peer_a);
        ids.sort();
        assert_eq!(ids, vec![&1, &3]);
    }

    #[test]
    fn iter_request_peers_returns_all_peers() {
        let mut reqs = make_active_requests();
        let peer_a = PeerId::random();
        let peer_b = PeerId::random();
        reqs.insert(1, peer_a, false, MockRequestItems::new(1), Span::none());
        reqs.insert(2, peer_b, false, MockRequestItems::new(1), Span::none());

        let peers: Vec<_> = reqs.iter_request_peers().collect();
        assert_eq!(peers.len(), 2);
        assert!(peers.contains(&peer_a));
        assert!(peers.contains(&peer_b));
    }

    #[test]
    fn multiple_independent_requests() {
        let mut reqs = make_active_requests();
        let peer = PeerId::random();
        reqs.insert(1, peer, false, MockRequestItems::new(1), Span::none());
        reqs.insert(2, peer, false, MockRequestItems::new(1), Span::none());
        assert_eq!(reqs.len(), 2);

        // Complete request 1 early (items received)
        let r1 = reqs.on_response(1, response(10));
        assert!(r1.is_some());
        assert!(r1.unwrap().is_ok());

        // Request 1 still in map as CompletedEarly (awaiting stream termination)
        assert_eq!(reqs.len(), 2);

        // Stream termination removes request 1
        assert!(reqs.on_response(1, stream_termination()).is_none());
        assert_eq!(reqs.len(), 1);

        // Complete request 2
        let r2 = reqs.on_response(2, response(20));
        assert!(r2.is_some());
        let (items, _) = r2.unwrap().unwrap();
        assert_eq!(items, vec![20]);
    }
}
