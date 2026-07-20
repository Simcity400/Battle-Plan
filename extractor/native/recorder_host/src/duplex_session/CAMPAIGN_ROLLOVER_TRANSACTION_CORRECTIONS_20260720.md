# Campaign rollover transaction corrections (2026-07-20)

This closeout corrects the dormant CRP1/CRA1 campaign-rollover contract without
activating the rollover capability in the product auto-attach profile.

## Corrected contract

- CRP1 now carries both campaign-pair digests and exact full CID1 payload
  hashes. The campaign-pair digest is CID1's digest of
  `(game_unique_id, game_unique_seed)`; the full payload hash also covers the
  observation hour and witnesses.
- `WholePairChanged` requires different campaign-pair digests.
- `SamePairTimeRollback` requires equal campaign-pair digests and distinct full
  CID1 payload hashes. A rollback therefore no longer incorrectly requires the
  complete CID1 bytes to be unchanged.
- The CRP1 v1 payload is 168 bytes: fixed fields through byte 32, two 32-byte
  campaign-pair digests, two 32-byte full-payload hashes, then the 8-byte
  proposal nonce.
- Recorder rollover state can be initialized only from an already durable and
  transmitted initial CFA1 admission, not from a caller-supplied hash.
- An accepted CRP1 mints a private move-only rebind authority only after the old
  stream drain, durable seal, new-sink open, and accepting CRA1 transmission.
  The identity dispatcher consumes that authority to journal and send the
  second CFA1, then atomically rebinds its logical-stream identity. Its ordinary
  first-record path remains first-only.
- Any second CID1 mismatch poisons the control session and dispatcher before a
  second durable journal entry or identity rebind.

Offline unit coverage was added for the 168-byte CRP1 layout, pair-digest versus
full-hash validation, collector same-pair rollback proposal construction, the
successful durable second-CFA1 transaction, and a mismatched second CID1 that
must fail before journaling or rebinding. Per repository policy, this lane did
not run builds, tests, Cargo, verifiers, or Ghidra; central validation remains a
primary-agent action.

## Deliberately unresolved product seams

These seams require product integration work and were not silently represented
as complete:

1. Implement the production `RecorderRolloverStore` over the real H4CR storage
   owner: contiguous drain through the advertised sequence, durable old-stream
   seal, durable new logical-stream creation, and durable rejection recording.
2. Connect the H4CR source/router lifecycle so the old source stops exactly at
   the authenticated boundary and the new source presents sequence-zero CID1
   before any world record.
3. Integrate CRP1 with the single H4CT receive router. The heartbeat path must
   not consume asynchronous CRP1 as a wrong-kind frame; heartbeat/control
   sequencing must be explicitly suspended and resumed around the rollover
   transaction.
4. Connect main-host storage manifests, path ownership, recovery, and process
   lifecycle handoff for multiple logical streams.
5. Complete the independent stream-fold and replay-decoder consumer work for
   multi-logical-stream capture sets.
6. Keep `AUTO_ATTACH_DUPLEX_V1` unchanged until the storage, router, heartbeat,
   recovery, and end-to-end evidence gates are complete. The dormant
   `AUTO_ATTACH_DUPLEX_ROLLOVER_V1` profile remains test/composition-only.
