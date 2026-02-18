# devnet health checks

Checks an agent should run when debugging a devnet or verifying a code change. These replace assertoor (which doesn't understand gloas yet) and use the beacon/EL APIs directly.

The script `scripts/kurtosis-run.sh` runs checks 1-3 automatically. The rest are for manual debugging when something fails.

## automated checks (in kurtosis-run.sh)

### 1. sync status

```
GET /eth/v1/node/syncing
```

- `is_syncing` should be `false`
- `head_slot` should advance every 6s (one slot)
- If `head_slot` stalls for 3+ polls (36s), the chain is stuck

### 2. finality

```
GET /eth/v1/beacon/states/head/finality_checkpoints
```

- `finalized.epoch` should reach `3` (two full epochs after gloas fork at epoch 1)
- `current_justified.epoch` should be ahead of `finalized.epoch` by 1
- If justified but not finalizing: attestation or fork choice issue

### 3. block production

```
GET /eth/v1/beacon/headers/head
```

- `header.message.slot` should match `head_slot` from sync status
- If head_slot advances but header_slot doesn't: blocks are being skipped

## manual checks (for debugging failures)

### 4. fork version

```
GET /eth/v1/beacon/states/head/fork
```

Check that after slot 8 (epoch 1):
- `current_version` is the gloas fork version
- `previous_version` is the fulu fork version
- `epoch` is `1`

If the fork version is wrong, the fork transition didn't happen.

### 5. block contents post-fork

```
GET /eth/v2/beacon/blocks/{slot}
```

For a slot >= 8 (post-gloas):
- Response should have `version: "gloas"` in the header
- Block body should contain `signed_execution_payload_header` (the ePBS bid)
- `execution_payload` should be empty/absent (payload is in the envelope, not the block)

### 6. execution payload envelope

The self-build envelope is not exposed via standard beacon API. Check CL logs for:
- `"Processing self-build envelope"` — envelope was created
- `"Calling newPayload"` or `"newPayload response"` — EL validated it
- `"on_execution_payload"` — fork choice processed it

If envelopes aren't being created, check:
- `get_execution_payload` — is it returning a valid payload for gloas?
- `build_self_build_envelope` — is it wrapping the payload correctly?
- `process_self_build_envelope` — is it using the right state?

### 7. EL sync status

```bash
curl -X POST -H "Content-Type: application/json" \
  --data '{"jsonrpc":"2.0","method":"eth_syncing","id":1}' \
  http://<el-rpc-url>
```

- Should return `false` (not syncing)
- If syncing: EL is behind, check engine API communication

### 8. EL block number

```bash
curl -X POST -H "Content-Type: application/json" \
  --data '{"jsonrpc":"2.0","method":"eth_blockNumber","id":1}' \
  http://<el-rpc-url>
```

- Should advance over time
- If stuck: `newPayload` is failing or not being called
- Compare with CL head_slot — EL blocks should roughly track CL slots (minus missed/empty slots)

### 9. validator duties

```
GET /eth/v1/validator/duties/proposer/{epoch}
```

- Should return proposer assignments for each slot in the epoch
- All 128 validators should appear across duties
- If empty: validator keys not loaded or VC not connected

### 10. peer count

```
GET /eth/v1/node/peers
```

- Single-node devnet: 0 peers is expected
- Multi-node: should have peers, if 0 then discovery/networking issue

### 11. attestation performance

Check CL logs for:
- `"Published attestation"` — attestations are being created
- Missing attestations: validators not attesting, check VC logs
- Late attestations: timing issues, check slot boundaries

### 12. transaction inclusion (with spamoor)

```bash
curl -X POST -H "Content-Type: application/json" \
  --data '{"jsonrpc":"2.0","method":"eth_getBlockByNumber","params":["latest",false],"id":1}' \
  http://<el-rpc-url>
```

- `transactions` array should be non-empty if spamoor is running
- If always empty: transactions not reaching the EL mempool, or not being included in payloads

## check order for debugging

When the devnet fails, check in this order:

1. **health.log** — did slots advance? did it stall? at what slot?
2. **CL logs** — search for `ERROR` and `WARN`, especially around the stall slot
3. **Fork boundary** (slot 8) — did the fork transition succeed?
4. **Block production** — are blocks being produced post-fork? search for `Produced block` or `Block production failed`
5. **Envelope processing** — search for `self_build_envelope`, `newPayload`, `on_execution_payload`
6. **EL logs** — search for `ERROR`, check `newPayload` responses
7. **VC logs** — is the VC connected? is it getting duties? is it producing attestations?

## finding logs in dump

After a failed run, logs are in `/tmp/kurtosis-runs/<RUN_ID>/dump/`:

```
dump/
  cl-1-lighthouse-geth--<uuid>/output.log    # CL (vibehouse) logs
  el-1-geth-lighthouse--<uuid>/output.log    # EL (geth) logs
  vc-1-geth-lighthouse--<uuid>/output.log    # VC (lighthouse VC) logs
  spamoor--<uuid>/output.log                 # spamoor logs
  dora--<uuid>/output.log                    # dora block explorer logs
```
