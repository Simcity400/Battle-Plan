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

## Production integration added in the following wave

- `duplex_session/rollover_fs.rs` is now the concrete H4CR storage owner. Its
  data and control handles share one guarded state: complete records remain
  contiguous, buffered records may drain only through the advertised CRP1
  boundary, the old file is synced and renamed before the next empty file is
  created and synced, and the exact candidate CID1 is synced before it can be
  handed to the second CFA1 barrier. The durable JSON state retains each sealed
  stream's frontier, CID1 digests, and whole-file SHA-256; recovery recomputes
  all of them. Immutable rejection receipts preserve refused proposals.
- The H4CT worker now has one rollover-aware receive router. It completes the
  outstanding heartbeat first; only a `PausedForControl` proof authorizes the
  immediately following CRP1. CIF1 remains routed and durably acknowledged in
  the same loop. The Win32 combined runner and its split production control
  driver expose that same operation, so heartbeat and rollover never race two
  reads on one channel.
- `replay_decoder::open_rollover_stream_set` admits a set only when logical ids
  are exactly `0..N`, every member begins with placement-valid IdentityV1 CID1,
  and both CID1 digests exactly match the durable recorder descriptor. No world
  hour is exposed until all members pass this pre-admission gate.

## Product selection and remaining activation gate

- The active collector owner now classifies an authenticated whole-pair change
  or same-pair clock rollback before world emission and executes the existing
  CRP1/CRA1/new-CID1/second-CFA1 coordinator. Sequence numbering and first-world
  proof gating restart only after the second CFA1 is accepted. Uncertain reads
  and one-component conflicts remain terminal.
- Recorder main selects the filesystem rollover store for the rollover
  capability, seals every logical stream, and re-admits the exact ordered set
  from durable state before packaging. Each member passes the ordinary summary,
  boundary, fold, container-write, and from-disk verification gates. The package
  set descriptor is published atomically only after all member manifests and
  packages exist; completed raw-state recovery can mechanically recreate a
  missing raw descriptor after a crash.
- The shipped replay opener recognizes the package-set descriptor and requires
  an explicit logical-stream selection. It verifies the hashed raw descriptor,
  every CRP1 transition, the selected manifest/package pair, and refuses
  recovery members. It never flattens same-pair rollback hours into a fabricated
  monotonic timeline; the shared decoder exposes set hours with their logical
  stream id.
- `AUTO_ATTACH_DUPLEX_V1` and the separate rollover live-evidence latch remain
  false. These integration changes do not authorize activation without an
  authenticated live lifecycle receipt covering the complete two-barrier path.
