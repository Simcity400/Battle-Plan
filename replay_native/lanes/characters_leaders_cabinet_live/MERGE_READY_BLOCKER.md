# characters_leaders_cabinet_live Merge-Ready Blocker

Registry source of truth:

- `replay_native/DOMAIN_REGISTRY.md`
- `replay_native/domain_registry.json`

Assigned final compact domain:

- `47` = `characters_leaders_cabinet_live`

The old slot-ready packet workflow is canceled. The lane now needs a merge-ready
lane-owned production mini stack plus live DLL/Rust compact proof before shared
registration. Current blocker is registration-slot access, not a domain-id or
fixture-root question.

Bounded blocker gate:

```powershell
python replay_native\lanes\characters_leaders_cabinet_live\validate_merge_ready_blocker.py
```

Expected result:

- registry assigns domain `47` to this lane;
- registry marks domain `46` / `air_variant_archetypes_live` as the active
  integration lane;
- shared collector/recorder files have no domain-47 registration hooks:
  no raw token, domain const, collector append, recorder id mapping, recorder
  name mapping, or recorder decoder;
- lane-local compact domain remains `47`;
- latest live development proof still has `0` appointed rows, which is a real
  measured-empty snapshot;
- nonzero appointed fixture remains valid: ENG transitions `0 -> 3 -> 1` and
  the fixture compact record has `appointed_advisor_rows=3`;
- no `native_session_manifest.json` under `replay_native/runs` mentions
  `characters_leaders_cabinet_live`.

Therefore the lane cannot truthfully produce a speed-five/current-equivalent
hourly DLL/Rust `.battleplan` proof while air owns the shared integration slot.
The next production step, once granted, is registration-only:

1. Register `characters_leaders_cabinet_live -> 47` in the shared collector and
   recorder.
2. Route the existing lane-owned domain-47 encoder/gate through those hooks.
3. Run the compact DLL/Rust extraction gate with domain `47` scheduled.
4. Require zero skipped hours, no worker/source/partial/bridge flags, ordered
   domain `47` perf/data records on every frame hour, Rust decode success, and
   `.battleplan` verification.
