# workstream: gloas fork choice

> status: **in progress** | priority: 1

## overview

The Gloas (EIP-7732 ePBS) fork choice is fundamentally different from previous forks.
The key change is that each block in the fork choice tree has a **3-state payload status**
(`PENDING`, `EMPTY`, `FULL`), and the tree branches at each block into empty/full paths.
This is not a minor tweak — it's a structural change to proto_array.

Spec source: https://github.com/ethereum/consensus-specs/blob/master/specs/gloas/fork-choice.md

## current state

- **4 failing fork_choice_reorg tests** (all gloas-specific, minimal preset)
- All other ef_tests pass (73/77, remaining 3 are pre-existing altair failures + 1 KZG env issue)
- Current proto_array implementation uses a simple `payload_revealed: bool` on each node
- This does NOT match the spec's 3-state model with `(root, payload_status)` pairs

### failing tests

```
include_votes_another_empty_chain_with_enough_ffg_votes_previous_epoch
include_votes_another_empty_chain_without_enough_ffg_votes_current_epoch
include_votes_another_empty_chain_with_enough_ffg_votes_current_epoch
simple_attempted_reorg_without_enough_ffg_votes
```

All show the same pattern: head is at slot N+1 when spec expects slot N. The implementation
doesn't correctly model the empty/full branching, so it always selects the "latest" block
instead of respecting the payload status constraints.

### passing tests (4/8 reorg tests pass)

```
delayed_justification_current_epoch
delayed_justification_previous_epoch
simple_attempted_reorg_delayed_justification_current_epoch
simple_attempted_reorg_delayed_justification_previous_epoch
```

## spec analysis: how gloas fork choice works

### ForkChoiceNode is a (root, payload_status) pair

```python
class ForkChoiceNode(Container):
    root: Root
    payload_status: PayloadStatus  # PENDING=0, EMPTY=1, FULL=2
```

This is the fundamental difference. In pre-Gloas, a node is identified by its root.
In Gloas, a node is identified by `(root, payload_status)`. The same block root can
appear as three different nodes in the fork choice tree.

### tree structure: blocks branch into empty/full

```
                    justified_root (PENDING)
                          |
                    +-----+-----+
                    |           |
              root_A (EMPTY)  root_A (FULL)    ← same root, two nodes
                    |           |
              root_B (PENDING) root_C (PENDING) ← different children
                    |           |
                   ...         ...
```

`get_node_children` implements this branching:

```python
def get_node_children(store, blocks, node):
    if node.payload_status == PAYLOAD_STATUS_PENDING:
        # A PENDING node's children are EMPTY and optionally FULL versions of itself
        children = [ForkChoiceNode(root=node.root, payload_status=PAYLOAD_STATUS_EMPTY)]
        if node.root in store.execution_payload_states:
            children.append(ForkChoiceNode(root=node.root, payload_status=PAYLOAD_STATUS_FULL))
        return children
    else:
        # EMPTY/FULL node's children are PENDING nodes of blocks whose parent matches
        return [
            ForkChoiceNode(root=root, payload_status=PAYLOAD_STATUS_PENDING)
            for root in blocks.keys()
            if (blocks[root].parent_root == node.root
                and node.payload_status == get_parent_payload_status(store, blocks[root]))
        ]
```

### get_weight returns 0 for non-PENDING nodes from previous slot

```python
def get_weight(store, node):
    if node.payload_status == PAYLOAD_STATUS_PENDING or \
       store.blocks[node.root].slot + 1 != get_current_slot(store):
        # Normal weight calculation
        ...
    else:
        return Gwei(0)  # ← Non-PENDING previous-slot blocks get zero weight
```

This is critical for reorg resistance: when deciding between EMPTY and FULL for the
previous slot's block, weight is always 0 — the tiebreaker decides.

### is_supporting_vote considers payload_present

```python
def is_supporting_vote(store, node, message):
    if node.root == message.root:
        if node.payload_status == PAYLOAD_STATUS_PENDING:
            return True  # PENDING always gets support
        if message.slot <= block.slot:
            return False  # Messages from before/at the block slot don't support EMPTY/FULL
        if message.payload_present:
            return node.payload_status == PAYLOAD_STATUS_FULL
        else:
            return node.payload_status == PAYLOAD_STATUS_EMPTY
    else:
        # Ancestor check
        ancestor = get_ancestor(store, message.root, block.slot)
        return (node.root == ancestor.root and
                (node.payload_status == PAYLOAD_STATUS_PENDING or
                 node.payload_status == ancestor.payload_status))
```

### LatestMessage now includes payload_present

```python
class LatestMessage:
    slot: Slot
    root: Root
    payload_present: boolean  # ← NEW: derived from attestation.data.index == 1
```

Updated in `update_latest_messages`:
```python
payload_present = attestation.data.index == 1
store.latest_messages[i] = LatestMessage(slot=slot, root=root, payload_present=payload_present)
```

### get_ancestor returns ForkChoiceNode (not just root)

```python
def get_ancestor(store, root, slot):
    block = store.blocks[root]
    if block.slot <= slot:
        return ForkChoiceNode(root=root, payload_status=PAYLOAD_STATUS_PENDING)
    # Walk up the chain
    parent = store.blocks[block.parent_root]
    while parent.slot > slot:
        block = parent
        parent = store.blocks[block.parent_root]
    return ForkChoiceNode(
        root=block.parent_root,
        payload_status=get_parent_payload_status(store, block),
    )
```

### get_parent_payload_status: derived from block hashes

```python
def get_parent_payload_status(store, block):
    parent = store.blocks[block.parent_root]
    parent_block_hash = block.body.signed_execution_payload_bid.message.parent_block_hash
    message_block_hash = parent.body.signed_execution_payload_bid.message.block_hash
    return PAYLOAD_STATUS_FULL if parent_block_hash == message_block_hash else PAYLOAD_STATUS_EMPTY
```

A child block declares whether its parent was FULL or EMPTY by comparing block hashes:
- If child's `bid.parent_block_hash == parent's bid.block_hash` → parent was FULL
- Otherwise → parent was EMPTY

### on_block: selects pre-state based on parent payload status

```python
def on_block(store, signed_block):
    block = signed_block.message
    if is_parent_node_full(store, block):
        state = copy(store.execution_payload_states[block.parent_root])
    else:
        state = copy(store.block_states[block.parent_root])
    # ... rest of block processing
```

### payload attestation handling in fork choice

PTC votes are stored per-block as boolean vectors (`payload_timeliness_vote[root]`),
NOT accumulated on nodes. The fork choice uses `is_payload_timely(root)` which checks
if the PTC vote count exceeds `PAYLOAD_TIMELY_THRESHOLD` (PTC_SIZE/2 = 256) AND
the execution payload state exists locally.

## ef_test structure for fork choice

Test format: https://github.com/ethereum/consensus-specs/tree/master/tests/formats/fork_choice

Each test has:
- `anchor_state.ssz_snappy` + `anchor_block.ssz_snappy` — initialize the store
- `steps.yaml` — sequential steps: `tick`, `block` (valid/invalid), `attestation`, `on_payload_info`, `checks`
- `block_<hash>.ssz_snappy` — individual block files
- `attestation_<hash>.ssz_snappy` — individual attestation files

The `checks` step validates: `head` (slot+root), `justified_checkpoint`, `finalized_checkpoint`,
`proposer_boost_root`, and optionally `get_proposer_head` and `should_override_forkchoice_update`.

Test runner: `testing/ef_tests/src/cases/fork_choice.rs`
Test definitions: `testing/ef_tests/tests/tests.rs` (line ~1011 for `fork_choice_reorg`)

The reorg tests that fail use only `tick`, `block`, `attestation`, and `checks` steps —
no `on_payload_info` or payload attestation steps. The failures are purely from the core
fork choice algorithm not correctly implementing the `(root, payload_status)` model.

## implementation plan

The fix requires significant changes to proto_array. Options:

### option A: model payload_status in proto_array nodes

Add `payload_status` to `ProtoNode` and create virtual EMPTY/FULL children when traversing.
This is the most architecturally aligned with the spec but requires deep changes to
proto_array's iteration, weight propagation, and best-descendant tracking.

### option B: parallel fork choice for gloas

Keep proto_array for pre-Gloas forks. Implement a separate `GloasForkChoice` that
uses the spec's dict-based model directly. Less efficient but correct-by-construction
and easier to validate against the spec.

### option C: extend proto_array with virtual nodes

Add synthetic EMPTY/FULL nodes as real proto_array entries (with a flag marking them
as virtual). Each block creates 1-3 nodes in the array. This keeps proto_array's O(n)
weight propagation but doubles/triples the node count.

## current implementation gaps

1. **No payload_status on nodes** — nodes have `payload_revealed: bool`, not 3-state
2. **No empty/full branching** — `get_node_children` not implemented
3. **No payload_present in LatestMessage** — votes don't track payload preference
4. **get_weight doesn't return 0 for non-PENDING previous-slot** — breaks reorg resistance
5. **get_ancestor doesn't return payload_status** — breaks is_supporting_vote
6. **on_block doesn't select pre-state by parent payload status** — uses wrong state
7. **No execution_payload_states store** — missing the FULL state tracking

## log

- 2026-02-14: investigation complete, 4/8 reorg tests failing, root cause identified
- 2026-02-14: spec analysis complete, documented all spec functions
