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
- `src/adapter.rs`: lane-local adapter that builds `LaneFrame` from validated
  character, unit-leader, trait-definition, country-politics, advisor, and
  static portrait inputs.
- `src/bin/appointed_fixture_gate.rs`: Rust proof gate for the ENG appointed
  advisor transition fixture.
- `src/bin/adapter_fixture_gate.rs`: Rust proof gate for the lane-local
  `CollectorState`-to-`LaneFrame` adapter shape, including one row from every
  domain-47 row family.
- `src/bin/replay_fixture_verify.rs`: optional-feature Rust proof gate that
  packs the domain-47 compact fixture stream into `.battleplan`, reopens it
  through `snapshot::Container`, runs `verify_all`, and byte-compares the
  compact stream entry.
- `src/bin/replay_record_verify.rs`: optional-feature Rust proof gate that
  packs an existing domain-47 compact record into `.battleplan`, reopens it
  through `snapshot::Container`, runs `verify_all`, and byte-compares the
  compact stream entry.
- `DOMAIN47_REGISTRATION_ONLY.patch`: exact low-conflict shared registration
  and recorder-decoder patch artifact.
- `REGISTRATION_HOOK_PLAN.md`: exact registration-only shared patch plan.

Compact row families now covered by the lane-local codec:

- `character`
- `unit_leader`
- `unit_leader_trait`
- `country_leader_role`
- `active_ruler`
- `advisor_roster`
- `appointed_advisor`
- `character_portrait`

Current-build RTTI roots used by the lane module:

- `CCharacter` RTTI RVA `0x2967E30`
- `CArmyLeader` RTTI RVA `0x293F538`
- `CNavyLeader` RTTI RVA `0x293F650`
- `CCountryLeader` RTTI RVA `0x296A720`
- `CAdvisor` RTTI RVA `0x2990E60`
- `CUnitLeaderTrait` RTTI RVA `0x292F160`

Production paths encoded in `src/extractor.rs` and `src/adapter.rs`:

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
cargo test --manifest-path replay_native\lanes\characters_leaders_cabinet_live\Cargo.toml --features replay-verify
```

Result: `19 passed; 0 failed`.

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
cargo run --quiet --manifest-path replay_native\lanes\characters_leaders_cabinet_live\Cargo.toml --bin adapter_fixture_gate -- replay_native\runs\characters_leaders_cabinet_live_20260701_adapter_fixture_v1
```

Result:

```json
{
  "domain": "characters_leaders_cabinet_live",
  "domain_id": 47,
  "result": "validated",
  "row_count_total": 8,
  "character_rows": 1,
  "unit_leader_rows": 1,
  "unit_leader_trait_rows": 1,
  "country_leader_role_rows": 1,
  "active_ruler_rows": 1,
  "advisor_roster_rows": 1,
  "appointed_advisor_rows": 1,
  "character_portrait_rows": 1,
  "status_flags_or": 0,
  "schema_flags_or": 7,
  "parse_errors": []
}
```

Adapter fixture output:

- `replay_native/runs/characters_leaders_cabinet_live_20260701_adapter_fixture_v1/adapter_fixture_gate.json`
- `replay_native/runs/characters_leaders_cabinet_live_20260701_adapter_fixture_v1/domain47_payload.bin`
- `replay_native/runs/characters_leaders_cabinet_live_20260701_adapter_fixture_v1/domain47_record.bin`

```powershell
cargo run --quiet --manifest-path replay_native\lanes\characters_leaders_cabinet_live\Cargo.toml --features replay-verify --bin replay_record_verify -- replay_native\runs\characters_leaders_cabinet_live_20260701_adapter_fixture_v1\domain47_record.bin replay_native\runs\characters_leaders_cabinet_live_20260701_adapter_fixture_v1
```

Result: `.battleplan` verified and compact stream byte-compared:
`hoi4replay_bytes=3286`, `collector_stream_bytes=422`,
`verified_stream_bytes=422`, `character_portrait_rows=1`,
`parse_errors=[]`.

```powershell
cargo run --quiet --manifest-path replay_native\snapshot\Cargo.toml --bin pack_replay -- verify replay_native\runs\characters_leaders_cabinet_live_20260701_adapter_fixture_v1\native_session.battleplan
```

Result: `verify: OK`, game
`characters_leaders_cabinet_live_domain47_record`, `snapshots=0`,
`unsigned=true`.

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
  "character_portrait_rows": 0,
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

Live-root refresh proof:

```powershell
python replay_native\lanes\characters_leaders_cabinet_live\proof_live_roots.py --force --use-rtti-vtables --out-dir replay_native\runs\characters_leaders_cabinet_live_20260701_live_refresh_v1
```

Result: `validated`, `errors=[]`, elapsed wall seconds `383.455`.

Decoded live compact row counts:

```json
{
  "row_count_total": 27803,
  "character_rows": 13158,
  "unit_leader_rows": 1249,
  "unit_leader_trait_rows": 3024,
  "country_leader_role_rows": 1938,
  "active_ruler_rows": 55,
  "advisor_roster_rows": 8379,
  "appointed_advisor_rows": 0,
  "character_portrait_rows": 0,
  "status_flags_or": 0,
  "schema_flags_or": 7,
  "parse_errors": []
}
```

Live-record replay pack proof:

```powershell
cargo run --quiet --manifest-path replay_native\lanes\characters_leaders_cabinet_live\Cargo.toml --features replay-verify --bin replay_record_verify -- replay_native\runs\characters_leaders_cabinet_live_20260701_live_refresh_v1\domain47_record.bin replay_native\runs\characters_leaders_cabinet_live_20260701_live_refresh_v1
```

Result: `.battleplan` verified and compact stream byte-compared:
`hoi4replay_bytes=197137`, `collector_stream_bytes=742411`,
`verified_stream_bytes=742411`, `character_portrait_rows=0`,
`parse_errors=[]`.

Independent container check:

```powershell
cargo run --quiet --manifest-path replay_native\snapshot\Cargo.toml --bin pack_replay -- verify replay_native\runs\characters_leaders_cabinet_live_20260701_live_refresh_v1\native_session.battleplan
```

Result: `verify: OK`, game
`characters_leaders_cabinet_live_domain47_record`, `snapshots=0`,
`unsigned=true`.

Live-refresh proof output:

- `replay_native/runs/characters_leaders_cabinet_live_20260701_live_refresh_v1/live_roots_proof.json`
- `replay_native/runs/characters_leaders_cabinet_live_20260701_live_refresh_v1/domain47_record.bin`
- `replay_native/runs/characters_leaders_cabinet_live_20260701_live_refresh_v1/native_session.battleplan`
- `replay_native/runs/characters_leaders_cabinet_live_20260701_live_refresh_v1/collector_stream.bin`
- `replay_native/runs/characters_leaders_cabinet_live_20260701_live_refresh_v1/native_session_manifest.json`
- `replay_native/runs/characters_leaders_cabinet_live_20260701_live_refresh_v1/replay_record_verify.json`

Current live-refresh v2 proof:

```powershell
python replay_native\lanes\characters_leaders_cabinet_live\proof_live_roots.py --force --use-rtti-vtables --out-dir replay_native\runs\characters_leaders_cabinet_live_20260701_live_refresh_v2
```

Result: `validated`, `errors=[]`, elapsed wall seconds `507.542`.
The record now uses the live game hour instead of a dummy hour:
`record_game_hour_raw=60823344`, decoded end sample `1943.4.22.1`.
The proof started at `60822599` / `1943.3.22.0`, so the Python root walk
advanced across `745` game hours while producing one record.

Decoded live v2 compact row counts:

```json
{
  "row_count_total": 28116,
  "character_rows": 13290,
  "unit_leader_rows": 1288,
  "unit_leader_trait_rows": 3164,
  "country_leader_role_rows": 1941,
  "active_ruler_rows": 55,
  "advisor_roster_rows": 8378,
  "appointed_advisor_rows": 0,
  "character_portrait_rows": 0,
  "status_flags_or": 0,
  "schema_flags_or": 7,
  "parse_errors": []
}
```

Live v2 replay pack proof:

```powershell
cargo run --quiet --manifest-path replay_native\lanes\characters_leaders_cabinet_live\Cargo.toml --features replay-verify --bin replay_record_verify -- replay_native\runs\characters_leaders_cabinet_live_20260701_live_refresh_v2\domain47_record.bin replay_native\runs\characters_leaders_cabinet_live_20260701_live_refresh_v2
```

Result: `.battleplan` verified and compact stream byte-compared:
`hoi4replay_bytes=199761`, `collector_stream_bytes=749547`,
`verified_stream_bytes=749547`, `parse_errors=[]`.

Independent container check:

```powershell
cargo run --quiet --manifest-path replay_native\snapshot\Cargo.toml --bin pack_replay -- verify replay_native\runs\characters_leaders_cabinet_live_20260701_live_refresh_v2\native_session.battleplan
```

Result: `verify: OK`, game
`characters_leaders_cabinet_live_domain47_record`, `snapshots=0`,
`unsigned=true`.

Live clock cadence probe:

```powershell
python replay_native\lanes\characters_leaders_cabinet_live\live_cadence_probe.py --force --duration-s 20 --poll-s 0.05 --target-advanced-hours 10 --out-dir replay_native\runs\characters_leaders_cabinet_live_20260701_live_cadence_probe_v2
```

Result: `validated_clock_cadence_observed`, `advanced_hours=10`,
`skipped_hours=0`, `ok_sample_count=112`. This proves the running game clock can
be sampled without skipped hours in lane-local proof tooling, while the full
Python root walk remains far too slow for hourly production capture.

Native scheduled adapter benchmark:

```powershell
cargo run --release --manifest-path replay_native\lanes\characters_leaders_cabinet_live\Cargo.toml --bin scheduled_adapter_benchmark -- --frames 120 --out-dir replay_native\runs\characters_leaders_cabinet_live_20260701_scheduled_adapter_benchmark_v1
```

Result: `validated`, `frames=120`, `native_hot_loop_only=true`,
`last_row_count=28247`, `appointed_rows=3`, `portrait_rows=128`,
`avg_read_time_us=49304`, `max_read_time_us=102401`, `status_flags_or=0`,
`schema_flags_or=7`, `parse_errors=[]`.

Replay verification for the last native scheduled frame:

```powershell
cargo run --quiet --manifest-path replay_native\lanes\characters_leaders_cabinet_live\Cargo.toml --features replay-verify --bin replay_record_verify -- replay_native\runs\characters_leaders_cabinet_live_20260701_scheduled_adapter_benchmark_v1\domain47_record_last.bin replay_native\runs\characters_leaders_cabinet_live_20260701_scheduled_adapter_benchmark_v1
```

Result: `.battleplan` verified, `row_count_total=28247`,
`collector_stream_bytes=787241`, `verified_stream_bytes=787241`,
`appointed_advisor_rows=3`, `character_portrait_rows=128`, `parse_errors=[]`.

Independent container check:

```powershell
cargo run --quiet --manifest-path replay_native\snapshot\Cargo.toml --bin pack_replay -- verify replay_native\runs\characters_leaders_cabinet_live_20260701_scheduled_adapter_benchmark_v1\native_session.battleplan
```

Result: `verify: OK`, game
`characters_leaders_cabinet_live_domain47_record`, `snapshots=0`,
`unsigned=true`.

Native scheduled cadence budget:

```powershell
python replay_native\lanes\characters_leaders_cabinet_live\scheduled_cadence_budget.py --benchmark replay_native\runs\characters_leaders_cabinet_live_20260701_scheduled_adapter_benchmark_v1\scheduled_adapter_benchmark.json --cadence replay_native\runs\characters_leaders_cabinet_live_20260701_live_cadence_probe_v2\live_cadence_probe.json --out-dir replay_native\runs\characters_leaders_cabinet_live_20260701_scheduled_cadence_budget_v1
```

Result: `validated`, `observed_hour_wall_us=587700`,
`avg_fraction_of_observed_hour=0.0839`,
`max_fraction_of_observed_hour=0.1742`; average scheduled cost is under one
quarter of the observed live hour and max scheduled cost is under one half.

This benchmark is not shared DLL capture. It proves the lane-owned native
scheduled API can consume cached roots, reread volatile values, encode/gate
domain-47 payloads, and verify the compact stream at row scale without Python in
the scheduled loop.

Remaining exact live blocker:

- The lane-local stack can produce and `.battleplan`-verify domain-47 compact
  records, including a fresh current-process live-root compact record and a
  release-mode native scheduled cached-root benchmark.
- Current registry source of truth lists id 47 as
  `lane_ready_shared_adapter_registration_blocked_remaining_matrix`; shared
  collector and recorder registry modules still leave id 47 as `Reserved` and
  `schedulable=false`.
- `DOMAIN47_REGISTRATION_ONLY.patch` contains the exact low-conflict shared
  registry/decoder patch. `git apply --check --verbose` passes against the
  current worktree; the patch is not applied because it only covers registry,
  dependency, and recorder decode/gate hooks.
- Live speed-five/current-equivalent DLL-only proof still requires shared
  collector hot-loop ownership plus the shared root cache and scheduled call
  that translates live `CollectorState` / country-politics /
  character-root data into `scheduled::NativeRootCache` and
  `scheduled::NativeFrameInput`.
- The current Python live-root proof is explicitly not hourly-capable:
  live-refresh v2 took `507.542` wall seconds while the game advanced `745`
  hours. This narrows the remaining live-production path to the shared
  root-cache/scheduled native adapter, not another Python full-root scan.
