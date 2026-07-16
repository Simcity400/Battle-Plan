# Construction Buildings State Damage Live Lane

This lane keeps active extractor, decoder, gate, and fixture files here. Historical
integration plans and lane status notes were moved to
`../../../documents/archive/native_lanes/construction_buildings_state_damage_live/`.

Current extraction completion status lives in
`../../../documents/MASTER_EXTRACTION_CONTRACT.md`.

## Lane-local replay-pack proof

`construction_buildings_state_damage_replay_pack_proof` is a host-shaped,
lane-local fixture generator and verifier. It is not a live-source capture and
must never be used as production certification evidence. Both its top-level
proof and the session manifest embedded in its `.battleplan` output declare
`artifact_kind = lane_local_host_shaped_fixture`,
`production_certifying = false`, and `source_capture_verified = false`.

The optional positional duration is
`requested-game-hour-advancements` in the exact range `1..=71`; invalid values
are rejected rather than clamped. A request for `N` advancements emits and
requires exactly `N + 1` consecutive closed frames, including the initial
frame. The verifier requires canonical record flags and sequence continuity,
DOMAIN_PERF before DOMAIN_DATA, exact D48 PERF/DATA status equality, and a final
physical-EOF FRAME_END whose emitted/scheduled counts and status equal the
observed owned PERF authority.
