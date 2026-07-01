# characters_leaders_cabinet_live Merge-Ready Blocker

Registry source of truth:

- `replay_native/DOMAIN_REGISTRY.md`
- `replay_native/domain_registry.json`

Assigned final compact domain:

- `47` = `characters_leaders_cabinet_live`

The old slot-ready packet workflow is canceled. The lane now has a lane-owned
mini stack, lane-local `CollectorState`-shape adapter, and final domain id, but
it is not merge-ready. Current blockers are shared registration plus the missing
shared root cache and scheduled collector call that feed live collector roots
into the lane adapter.

Bounded blocker gate:

```powershell
python replay_native\lanes\characters_leaders_cabinet_live\validate_merge_ready_blocker.py
```

Expected result:

- registry assigns domain `47` to this lane;
- registry marks domain `46` / `air_variant_archetypes_live` as the current
  validated slice with shared token hook/full air lifecycle still open;
- registry marks domain `47` as
  `lane_ready_shared_adapter_registration_blocked_remaining_matrix`;
- shared collector/recorder files have no domain-47 registration hooks:
  no raw token, domain const, collector append, recorder id mapping, recorder
  name mapping, or recorder decoder;
- lane-local compact domain remains `47`;
- lane-local adapter fixture emits one row from every row family, including
  `character_portrait`, with `status_flags_or=0` and `parse_errors=[]`;
- lane-local live refresh v2 emits a real current-process domain-47 compact
  record with live `record_game_hour_raw=60823344`, `28116` rows,
  `status_flags_or=0`, `schema_flags_or=7`, `parse_errors=[]`, and
  `.battleplan` verification;
- live refresh v2 also proves the Python root walk is not a production cadence
  path: it took `507.542` wall seconds while the running game advanced `745`
  game hours (`60822599` -> `60823344`);
- lane-local live clock cadence probe validates the current game-hour root
  separately with `advanced_hours=10`, `skipped_hours=0`, and
  `zero_skipped_hours_observed=true`;
- lane-local native scheduled benchmark validates the non-Python hot-loop
  strategy at live-ish scale: `120` release-mode frames, `28247` rows each,
  `avg_read_time_us=49304`, `max_read_time_us=102401`, `status_flags_or=0`,
  `schema_flags_or=7`, `parse_errors=[]`, `appointed_rows=3`, and
  `portrait_rows=128`; its last compact record was `.battleplan` verified;
- lane-local cadence budget proof compares the cached-root scheduled cost to
  the live clock probe: `observed_hour_wall_us=587700`,
  `avg_fraction_of_observed_hour=0.0839`,
  `max_fraction_of_observed_hour=0.1742`;
- latest live development proof still has `0` appointed rows, which is a real
  measured-empty snapshot;
- nonzero appointed fixture remains valid: ENG transitions `0 -> 3 -> 1` and
  the fixture compact record has `appointed_advisor_rows=3`;
- no shared-recorder `native_session_manifest.json` under `replay_native/runs`
  claims `characters_leaders_cabinet_live`. Lane-local proof manifests with
  `characters_leaders_cabinet_live.*` schemas are allowed evidence, not shared
  registration.

Therefore the lane is no longer blocked by fixture-only proof, compact
serialization, Rust decode, `.battleplan` packing, or a lane-owned native
scheduled hot-loop implementation. It still cannot truthfully produce a
speed-five/current-equivalent hourly DLL/Rust proof from the one injected DLL
collector. The remaining production step is not schema research or another
Python full-root scan; it is the narrow shared integration work below:

1. Register `characters_leaders_cabinet_live -> 47` in the shared collector and
   recorder.
2. Add the shared collector root cache and scheduled call that populate
   `scheduled::NativeRootCache` and per-frame `scheduled::NativeFrameInput`
   from live `CollectorState` country/politics roots and the validated
   character/leader/advisor roots. The exact lane-local adapter contract is
   `replay_native/lanes/characters_leaders_cabinet_live/LANEFRAME_ADAPTER_PLAN.md`;
   the smallest shared hook plan is
   `replay_native/lanes/characters_leaders_cabinet_live/DOMAIN47_SHARED_ADAPTER_HOOK_PLAN.md`.
3. Route the lane-owned domain-47 encoder/gate through those hooks.
4. Run the compact DLL/Rust extraction gate with domain `47` scheduled.
5. Require zero skipped hours, no worker/source/partial/bridge flags, ordered
   domain `47` perf/data records on every frame hour, Rust decode success, and
   `.battleplan` verification.
