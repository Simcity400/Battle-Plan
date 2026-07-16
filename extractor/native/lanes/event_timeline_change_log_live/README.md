# Event Timeline / Change Log Live Lane
> Active lane note: this README is the only active lane-local documentation. Historical plans, proof reports, blocker notes, and registration packets for this lane are archived under ../../../documents/archive/native_lanes/event_timeline_change_log_live/. Current extraction completion status lives in ../../../documents/MASTER_EXTRACTION_CONTRACT.md.


Lane-local production contract for cross-domain replay events.

This crate does not duplicate domain extraction and does not edit shared
collector or recorder entrypoints. It defines a lane-local compact event proof
wire (`249`, deliberately outside the production roster),
fixed event rows, a fail-closed decoder/gate, and a proof CLI that verifies
event-source domain availability from existing compact DLL/Rust recordings.
The proof CLI never creates event rows: it copies and packages the exact stream
it was given only after the production gate proves canonical framing, strict
sequence and closed-hour order, one event DATA/PERF pair with exactly matching
status per hour, exact `FRAME_END` emitted/scheduled counts against the observed
PERF roster, a physical final `FRAME_END`, and each event row's
domain/sequence/hour/row link to an earlier compatible source
DOMAIN_DATA record in that same frame. That earlier DATA must pass the recorder's
canonical per-domain fold decoder and shared DATA-authority decoder, and the
referenced source domain must have exactly one same-frame PERF record with
matching DATA/PERF status authority. Proof-fixture rows are test-only and are
rejected by that production gate.

The row gate also binds each event kind to its permitted source-domain family;
for example, a research event cannot cite a focus row even when the cited row
index is in bounds. `source_row_index` is checked against the shared decoder's
canonical emitted-row count, not merely against payload length.

Certification duration is never inferred from a mutable manifest. The proof
CLI requires `--game-hours <positive advancements>` and checks both the raw
stream and manifest. The contract is exact and inclusive: `N` advancements
require exactly `N + 1` consecutive healthy `FRAME_END` records; both short and
overlong streams fail. The authoritative production preset uses
`--game-hours 48`, which therefore requires exactly 49 closed frames.
The manifest must independently declare the same duration/counts, zero skipped
hours, an identical `hourly_requirement.requested_game_hours`, a healthy hourly
requirement, canonical H4CR metadata, healthy compact
status booleans/domain coverage, and a non-empty replay path.

This lane-local id is **not a production registration**: canonical domain `44`
is already `supply_logistics_attrition_live`, and proof-only `249` is excluded
from the production roster. The proof gate must
not be used to claim event-domain integration until the orchestrator assigns a
non-colliding production id and updates the shared registries. Future integration
must then follow this shape:

1. Reserve a non-colliding compact id as `event_timeline_change_log_live` in the
   collector and recorder domain registries.
2. Let domain owners emit typed source-domain records as they already do.
3. Add one orchestrated eventizer pass after deterministic per-hour domain data
   emission and before `FRAME_END`.
4. Eventizer emits event `DOMAIN_DATA` records only from validated source
   rows; unknown layouts produce no event rows and set a lane-local gap/proof
   flag instead of guessing.
5. Recorder decodes the assigned event domain with this gate and stores the stream in
   `.battleplan` next to the compact collector stream.

The lane proof intentionally marks unsupported source families as missing when
the selected compact run does not include their source domains. That narrows the
remaining integration backlog without claiming domain roots owned by other
lanes.
