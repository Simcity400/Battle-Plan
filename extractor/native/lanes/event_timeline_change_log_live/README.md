# Event Timeline / Change Log Live Lane

Lane-local production contract for cross-domain replay events.

This crate does not duplicate domain extraction and does not edit shared
collector or recorder entrypoints. It defines a compact event domain (`44`),
fixed event rows, a fail-closed decoder/gate, and a proof CLI that verifies
event-source domain availability from existing compact DLL/Rust recordings.

Integration slot patch plan:

1. Reserve compact domain id `44` as `event_timeline_change_log_live` in the
   collector and recorder domain registries.
2. Let domain owners emit typed source-domain records as they already do.
3. Add one orchestrated eventizer pass after deterministic per-hour domain data
   emission and before `FRAME_END`.
4. Eventizer emits domain `44` `DOMAIN_DATA` records only from validated source
   rows; unknown layouts produce no event rows and set a lane-local gap/proof
   flag instead of guessing.
5. Recorder decodes domain `44` with this gate and stores the stream in
   `.battleplan` next to the compact collector stream.

The lane proof intentionally marks unsupported source families as missing when
the selected compact run does not include their source domains. That narrows the
remaining integration backlog without claiming domain roots owned by other
lanes.
