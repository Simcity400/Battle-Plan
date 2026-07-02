# land_combat_detail_events_live lane-local stack
> Active lane note: this README is the only active lane-local documentation. Historical plans, proof reports, blocker notes, and registration packets for this lane are archived under ../../../documents/archive/native_lanes/land_combat_detail_events_live/. Current extraction completion status lives in ../../../documents/MASTER_EXTRACTION_CONTRACT.md.


Domain 55 is assigned to `land_combat_detail_events_live`.
Shared collector scheduling intentionally forces domain 21 `land_combats` when
domain 55 is requested, because detail events derive from the same in-frame
combat snapshot and logical generation tracker.

This lane currently proves only the production-shaped subset that can be derived
from the accepted domain-21 `CLandCombat` root:

- phase lifecycle rows: started, updated, ended;
- participant delta rows: joined, left-unclassified.

The left-side participant row is deliberately named unclassified. The validated
domain-21 root can prove a division disappeared from a side vector, but it does
not distinguish retreat, overrun, casualty removal, or combat resolution.

Still open for the master row:

- casualty amounts;
- equipment loss amounts;
- retreat versus overrun classification;
- tactics;
- reserves;
- combat width;
- terrain;
- combat modifiers.

## Files

- `src/lib.rs`: compact domain-55 encoder, transition builder, decoder/gate, and
  tests.
- `src/bin/land_combat_detail_lane_proof.rs`: lane-local compact/replay proof
  harness.
- `probe_870ebf_land_detail_roots.py`: bounded read-only live probe for the
  current 870ebf land-combat root and field census.

## Proof commands

```powershell
cargo test --manifest-path extractor\native\lanes\land_combat_detail_events_live\Cargo.toml
cargo run --manifest-path extractor\native\lanes\land_combat_detail_events_live\Cargo.toml --bin land_combat_detail_lane_proof -- --out-dir extractor\native\runs\land_combat_detail_events_live_lane_stack_870ebf_20260701_v1
python extractor\native\lanes\land_combat_detail_events_live\probe_870ebf_land_detail_roots.py --pid 45880 --out-dir extractor\native\runs\land_combat_detail_events_live_roots_870ebf_20260701_v1
```

The Rust proof writes and verifies `native_session.battleplan`. The Python probe
is discovery/validation only and is not the live hot extraction path.
