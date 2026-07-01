# Domain 51 Registration Hook Plan

The assigned compact domain is `51` from `replay_native/domain_registry.json`.
This lane remains lane-local until the orchestrator opens an integration slot.

## Collector hook

1. Move or include `scripted_state_domain51_collector.rs` as the lane-owned
   collector module for `scripted_state_flags_variables_live`.
2. Add a registration-only hook that registers a module descriptor with:
   - id: `51`
   - canonical/wire name: `scripted_state_flags_variables_live`
   - status while landing: `ActiveIntegration`
   - schedulable: `true`
   - requires data record: `true`
3. Keep `domain_registry.rs` as the source of truth. The reserved registry row
   can remain non-schedulable until the module registers the active descriptor;
   no lower domain IDs should be reused.
4. The scheduled-hour collector path passes validated country/global/state
   variable and flag scope reads into `collect_domain51_frame`.
5. Wrap each collected payload with the production H4CR envelope implemented in
   `scripted_state_domain51_stream.rs`: one domain-51 perf record, one domain-51
   data record, and one frame-end record per scheduled game hour.
6. If delayed-event or scripted-GUI roots are still fixture-blocked, pass
   `OpaqueRootState::UnresolvedFixtureBlocked`; this emits only opaque badge
   rows and unresolved status bits, not `STATUS_PARTIAL_DATA`.

## Recorder hook

1. Move or include `scripted_state_compact.rs` and
   `scripted_state_domain51_gate.rs` as the lane-owned recorder decoder/gate.
2. Register domain `51 -> scripted_state_flags_variables_live` through the
   recorder compact-domain registry/module interface. Current
   `recorder_host/src/compact_domain_registry.rs` names the reserved domain, but
   `recorder_host/src/main.rs` has no domain-51 data decoder dispatch.
3. Add a registration-only dispatch arm for domain `51` that calls
   `gate_domain51_payload` for each domain-data record. Do not add broad
   shared entrypoint logic in this lane.
4. Treat parse errors, `STATUS_PARTIAL_DATA`, or `STATUS_SOURCE_UNAVAILABLE` as
   partial/source failures. Treat `STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED` and
   `STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED` as fail-closed badges only.

## Proof hook

1. Run the lane tests in this directory before shared integration.
2. Use `scripted_state_domain51_stream.rs` to emit and gate a lane-local
   H4CR/domain-51 `collector_stream.bin`.
3. Pack the lane-local stream into `native_session.battleplan` with
   `domain51_replay_pack_proof.py`, then verify it with the shared
   `snapshot` `pack_replay verify` command.
4. After the integration slot opens, run live speed-five/current-equivalent
   hourly capture with domain 51 scheduled.
5. Required pass condition: one accepted domain-51 compact data record per
   scheduled hour, zero skipped hours, no partial/source/worker/bridge failure,
   and `.battleplan` decode verification.
