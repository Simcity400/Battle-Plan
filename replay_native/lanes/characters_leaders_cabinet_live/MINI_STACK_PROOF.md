# characters_leaders_cabinet_live lane-local mini stack

Status: lane-local production mini stack built for final domain `47`. Shared
collector/recorder entrypoints were not edited.

Implemented lane-owned files:

- `Cargo.toml`: standalone Rust lane crate.
- `src/lib.rs`: lane exports for the DLL collector wrapper, extractor contract,
  compact encoder, and decoder gate.
- `src/dll_collector.rs`: DLL-facing module wrapper that converts a lane
  `LaneFrame` into a wrapped compact domain-47 record.
- `src/extractor.rs`: current-build roots, production offsets, cabinet bucket
  semantics, active-ruler pointer walk, advisor raw identity gate, assigned
  trait vector gate, appointed-advisor active-ideas intersection, and frame
  encoder.
- `src/bin/appointed_fixture_gate.rs`: Rust proof gate for the ENG appointed
  advisor transition fixture.
- `src/bin/replay_fixture_verify.rs`: optional-feature Rust proof gate that
  packs the domain-47 compact fixture stream into `.battleplan`, reopens it
  through `snapshot::Container`, runs `verify_all`, and byte-compares the
  compact stream entry.
- `DOMAIN47_REGISTRATION_ONLY.patch`: exact low-conflict shared registration
  and recorder-decoder patch artifact.
- `REGISTRATION_HOOK_PLAN.md`: exact registration-only shared patch plan.

Current-build RTTI roots used by the lane module:

- `CCharacter` RTTI RVA `0x2967E30`
- `CArmyLeader` RTTI RVA `0x293F538`
- `CNavyLeader` RTTI RVA `0x293F650`
- `CCountryLeader` RTTI RVA `0x296A720`
- `CAdvisor` RTTI RVA `0x2990E60`
- `CUnitLeaderTrait` RTTI RVA `0x292F160`

Production paths encoded in `src/extractor.rs`:

- Active ruler:
  `CCountry+0xF90 -> CPoliticalStatus+0xD0 -> CPoliticalParty+0x70
  vector<CCountryLeader*> -> CCountryLeader+0x08 -> CCharacter`.
- Advisor roster identity:
  `CAdvisor+0x10 -> CCharacter`, then `CCharacter+0x68` tag database id and
  `CCharacter+0x0C` character id.
- Appointed advisor/cabinet state:
  active politics idea tokens intersected with the current advisor roster
  `idea_token`; no `CAdvisor` hired bool is used.
- Assigned unit-leader traits:
  `leader+0xDC8` data pointer, `leader+0xDD0` capacity,
  `leader+0xDD4` size, elements are `CUnitLeaderTrait*`.
- Unit-leader skill:
  `*(leader+0xE60)+0x1B8`.

Rust proof commands:

```powershell
cargo test --manifest-path replay_native\lanes\characters_leaders_cabinet_live\Cargo.toml
```

Result: `14 passed; 0 failed`.

```powershell
cargo run --quiet --manifest-path replay_native\lanes\characters_leaders_cabinet_live\Cargo.toml --bin appointed_fixture_gate
```

Result:

```json
{
  "domain": "characters_leaders_cabinet_live",
  "domain_id": 47,
  "result": "validated",
  "transition_counts": {"before": 0, "after_add": 3, "after_remove": 1},
  "payload_bytes": 289,
  "record_bytes": 321,
  "advisor_roster_rows": 3,
  "appointed_advisor_rows": 3,
  "schema_flags_or": 7,
  "parse_errors": []
}
```

Proof output:

- `replay_native/runs/characters_leaders_cabinet_live_20260701_lane_module_v1/appointed_fixture_gate.json`

```powershell
cargo run --manifest-path replay_native\lanes\characters_leaders_cabinet_live\Cargo.toml --features replay-verify --bin replay_fixture_verify -- replay_native\runs\characters_leaders_cabinet_live_20260701_replay_fixture_v1
```

Result:

```json
{
  "result": "verified",
  "domain": "characters_leaders_cabinet_live",
  "domain_id": 47,
  "hoi4replay_bytes": 3080,
  "collector_stream_bytes": 321,
  "payload_bytes": 289,
  "row_count_total": 6,
  "advisor_roster_rows": 3,
  "appointed_advisor_rows": 3,
  "schema_flags_or": 7,
  "status_flags_or": 0,
  "verified_stream_bytes": 321,
  "parse_errors": []
}
```

Independent container check:

```powershell
replay_native\target_proof18\release\pack_replay.exe verify replay_native\runs\characters_leaders_cabinet_live_20260701_replay_fixture_v1\native_session.battleplan
```

Result: `verify: OK`, game
`characters_leaders_cabinet_live_domain47_fixture`, `snapshots=0`,
`unsigned=true`.

Replay proof output:

- `replay_native/runs/characters_leaders_cabinet_live_20260701_replay_fixture_v1/native_session.battleplan`
- `replay_native/runs/characters_leaders_cabinet_live_20260701_replay_fixture_v1/collector_stream.bin`
- `replay_native/runs/characters_leaders_cabinet_live_20260701_replay_fixture_v1/native_session_manifest.json`
- `replay_native/runs/characters_leaders_cabinet_live_20260701_replay_fixture_v1/replay_fixture_verify.json`

Remaining exact live blocker:

- The lane-local stack can produce and `.battleplan`-verify domain-47 compact
  records.
- Current registry modules list id 47 as `Reserved` and `schedulable=false`.
- `DOMAIN47_REGISTRATION_ONLY.patch` contains the exact low-conflict shared
  registry/decoder patch. `git apply --check --verbose` passes against the
  current worktree; the patch is not applied because the shared collector
  hot-loop hook would overlap active shared edits.
- Live speed-five/current-equivalent DLL-only proof still requires shared
  collector hot-loop ownership because `collector/src/lib.rs` and
  `recorder_host/src/main.rs` are already actively dirty from other lanes.
