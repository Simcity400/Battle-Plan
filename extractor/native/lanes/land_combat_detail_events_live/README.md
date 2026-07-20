# land_combat_detail_events_live lane-local stack
> Active lane note: this README is the only active lane-local documentation. Historical plans, proof reports, blocker notes, and registration packets for this lane are archived under ../../../documents/archive/native_lanes/land_combat_detail_events_live/. Current extraction completion status lives in ../../../documents/MASTER_EXTRACTION_CONTRACT.md.


Domain 55 is assigned to `land_combat_detail_events_live`.
Shared collector scheduling intentionally forces domain 21 `land_combats` when
domain 55 is requested, because detail events derive from the same in-frame
combat snapshot and logical generation tracker.

This lane proves the production-shaped subset derived from the accepted
domain-21 `CLandCombat` root and the exact-gated terminal writer hook:

- phase lifecycle rows: started, updated, ended;
- participant delta rows: joined, left-unclassified;
- exact per-side counts, damage accumulators, progress, and tactic identity;
- exact reserve-member identities in engine order;
- exact country-to-country `SDamageDone` attribution;
- pointer-free terminal combat records joined to `(combat_id,generation)`.

Schema v7 also reserves fail-closed, pointer-free transport for the last two X2
observations. `src/x2_carriers.rs` contains the exact synchronous width-call
carrier and the current-loss writer contract seam. Both activation constants
remain false until their retained human-input control cards are satisfied, so
the production collector emits empty v7 observation families today.

The terminal writer is separately production-enabled. Its exact-10 replacement
authority hashes are pinned in `src/a729_hook.rs`; installation remains
all-or-nothing behind the normal-writer, overrun-writer, and allocator entry
sentinels. The width producer and leaf exact-10 hashes are pinned in
`src/x2_carriers.rs`, but authority re-export does not replace the required
runtime transition control and therefore does not activate width.

The left-side participant row is deliberately named unclassified. The validated
domain-21 root can prove a division disappeared from a side vector, but it does
not distinguish retreat, overrun, casualty removal, or combat resolution.

Still unavailable/fail-closed for the master row:

- current-epoch manpower casualty amounts;
- current-epoch equipment loss amounts;
- retreat versus overrun classification;
- combat width;
- terrain;
- combat modifiers.

The retained-evidence census and minimal remaining capture boundaries live in
`../../../re_evidence/X2_LAND_COMBAT_RESIDUALS/REPORT.md` and
`HUMAN_INPUT_REQUIREMENTS.json` in that directory. Exact `SDamageDone`
attribution must not be relabelled as manpower or equipment loss.

## Files

- `src/lib.rs`: compact domain-55 encoder, transition builder, decoder/gate, and
  tests.
- `src/x2_carriers.rs`: dormant exact-build width and loss carrier seams with
  byte/identity gates, bounded queues, and pointer-erasing projectors.
- `src/bin/land_combat_detail_lane_proof.rs`: lane-local compact/replay proof
  harness.
- `src/bin/land_combat_detail_stream_validate.rs`: read-only standalone product
  validator for an existing compact stream.
- `probe_870ebf_land_detail_roots.py`: bounded read-only live probe for the
  current 870ebf land-combat root and field census.

The lane gate consumes a full compact stream, not an isolated DATA record. It
requires sequence 1 followed by exact non-wrapping +1 steps, canonical record
envelopes, consecutive hours, exactly one domain-55 DATA and PERF record per
closed hour, and a final canonical FRAME_END. The D55 DATA/PERF pair must agree
exactly on status, elapsed-wall timestamp, and all four PERF count fields; each
PERF count must equal the total decoded row count across every schema family,
including the v7 width/current-loss families. Payloads owned by other domains remain opaque. Within D55, participant
joins/leaves must form a complete delta set whose side/division identities and
old/new count direction reconcile exactly. `gate_stream_for_product`
additionally requires an explicitly positive minimum game-hour advancement
count; the inclusive authority requirement is therefore that many advancements
plus one consecutive closed FRAME_END.

## Proof commands

```powershell
cargo test --manifest-path extractor\native\lanes\land_combat_detail_events_live\Cargo.toml
cargo run --manifest-path extractor\native\lanes\land_combat_detail_events_live\Cargo.toml --bin land_combat_detail_lane_proof -- --out-dir extractor\native\runs\land_combat_detail_events_live_lane_stack_870ebf_20260701_v1
cargo run --manifest-path extractor\native\lanes\land_combat_detail_events_live\Cargo.toml --bin land_combat_detail_stream_validate -- --stream <compact.bin> --minimum-game-hour-advancements 9
python extractor\native\lanes\land_combat_detail_events_live\probe_870ebf_land_detail_roots.py --pid 45880 --out-dir extractor\native\runs\land_combat_detail_events_live_roots_870ebf_20260701_v1
```

The Rust proof writes and verifies `native_session.battleplan`. The Python probe
is discovery/validation only and is not the live hot extraction path.
