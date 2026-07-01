# Weather Remaining Blockers Closed - 2026-07-01

Completion status: COMPLETE for the remaining enum/particle blocker-rejection
slice. Whole weather concept/domain 24 remains NOT COMPLETE until the
sandstorm bit is shared-registered and proven in a DLL `.battleplan` stream.

Backlog reduced: yes.

Exact backlog item removed or narrowed:

- Raw enum-name parity for `SWeatherPerRegion +0x78` values `3..7` is
  conclusively rejected as the wrong production candidate. `+0x78` is not the
  stable weather-name enum.
- Close-zoom particle/entity parity is narrowed to an exact blocker: no clean
  particle/entity root exists under the scanned `CWeatherManager` body; the only
  particle-looking manager candidate is rejected below.
- Other map-visible weather drivers remain covered by the existing direct
  weather outputs and static game-file assets, except the already-known
  sandstorm shared-registration gap.

Files changed:

- `extractor/native/lanes/weather_live/weather_remaining_blocker_probe.py`
- `extractor/native/lanes/weather_live/WEATHER_REMAINING_BLOCKERS_CLOSED_20260701.md`
- `extractor/native/runs/weather_live_full_replay_residuals_870ebf_20260701_v2/weather_remaining_blocker_probe.json`
- refreshed proof outputs:
  `weather_particle_vector_probe.json`,
  `weather_manager_vector_inventory.json`,
  `weather_predicate_static_probe.json`,
  `weather_name_static_table_probe.json`,
  `compact_replay_gate.weather_live.json`

Roots/offsets/functions validated or rejected:

- Build validated:
  `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.
- Live roots validated again: `CGameState = u64[module_base +0x3316E90]`,
  `CWeatherManager = u64[CGameState +0x688]`, region vector
  `CWeatherManager +0x40`, province vector `CWeatherManager +0x10`.
- `set_weather` fixture callback validated at `0x140287090`; it requires a
  selected province and applies through `CWeatherManager +0x688/+0x40`.
- `set_weather` accepts `clear`, `rain`, `storm`, `snow`, `blizzard`,
  `sandstorm`; no arctic-water or mud region-name fixture exists there.
- Setter functions validated:
  `clear=0x140F0B9D0`, `rain=0x140F0BA00`, `storm=0x140F0BA90`,
  `snow=0x140F0BA60`, `blizzard=0x140F0B9A0`,
  `sandstorm=0x140F0BA30`.
- Setter write proof:
  `rain` writes `[rcx+0x108]=1`, `storm` writes `[rcx+0x110]=1`,
  `snow` writes `[rcx+0x118]=1`, `blizzard` writes `[rcx+0x120]=1`,
  `sandstorm` writes `[rcx+0x128]=1`; none writes `SWeatherPerRegion +0x78`.
- Live rejection of `+0x78` as name enum: current frame has `+0x78` counts
  `0:4`, `1:284`, `2:17`; value `1` simultaneously has `rain_light=70`,
  `rain_heavy=36`, `snow=1`, `sandstorm=2`, `arctic_water=1`; value `2`
  simultaneously has `rain_light=7`, `rain_heavy=2`, `arctic_water=2`.
- Static predicate evidence still validates named flag offsets, not enum names:
  `rain_light +0x108`, `rain_heavy +0x110`, `snow +0x118`,
  `blizzard +0x120`, `arctic_water +0x130`, `sandstorm +0x128`.
- Static ordered enum-name table rejected: no complete string cluster/pointer
  table; `weather_name_static_table_probe` reports `pointer_table_evidence =
  not_found`.
- Close-zoom particle/entity root rejected candidates:
  `CWeatherManager+0x5B8` has `size=64`, `cap=0`, no inline vtable stride, and
  heterogeneous qword pointer samples (`CEquipmentType`, `CPdxParticleType`,
  null, garbage-like values), so it is not a clean weather particle/entity
  vector root.
- Manager scan proof: `57` vector-like candidates over `0x2000` manager bytes;
  clean particle/entity roots outside known weather arrays: `[]`.
- Weather terrain modifier manager candidates at `+0x78`, `+0x90`, `+0xA8`,
  `+0xC0`, `+0xD8`, `+0xF0`, `+0x108` classify as static/config modifier
  containers, not close-zoom particle/entity roots.

Compact schema changes: none in this continuation. The previous lane-local v3
sandstorm payload remains the only schema change and still requires shared
registration before production shipment.

Rust decoder/gate changes: none in this continuation. Existing Rust replay gate
was rerun against the production weather stream.

Proof commands/results:

```powershell
python extractor\native\lanes\weather_live\weather_particle_vector_probe.py --pid 45880 --out-dir extractor\native\runs\weather_live_full_replay_residuals_870ebf_20260701_v2
```

Result: errors `[]`; static weather particle assets validated; `front_records =
5614`, `effect_entity_records = 318`; effect entity records are
`CModifier=301`, `0x0=17`, clean weather/particle entity records `0`.

```powershell
python extractor\native\lanes\weather_live\weather_manager_vector_inventory.py --pid 45880 --scan-bytes 0x2000 --out-dir extractor\native\runs\weather_live_full_replay_residuals_870ebf_20260701_v2
```

Result: errors `[]`; `candidate_count = 57`; no clean particle/entity root
survived the targeted rejection pass.

```powershell
python extractor\native\lanes\weather_live\weather_predicate_static_probe.py --out-dir extractor\native\runs\weather_live_full_replay_residuals_870ebf_20260701_v2
```

Result: errors `[]`; `7` predicate RTTI/vtables located; named region flag
offsets validated; enum constants not fully proven.

```powershell
python extractor\native\lanes\weather_live\weather_name_static_table_probe.py --out-dir extractor\native\runs\weather_live_full_replay_residuals_870ebf_20260701_v2
```

Result: errors `[]`; complete ordered enum-name pointer table not found.

```powershell
python extractor\native\lanes\weather_live\weather_remaining_blocker_probe.py --pid 45880 --out-dir extractor\native\runs\weather_live_full_replay_residuals_870ebf_20260701_v2
```

Result: errors `[]`; `raw_enum_name_parity =
candidate_rejected_plus_0x78_is_not_weather_name_enum`;
`close_zoom_particle_entity_parity =
blocked_exact_render_entity_root_or_controlled_camera_fixture_required`;
`unblocked_weather_residual_work_remaining = false`.

```powershell
cargo run --manifest-path extractor\native\lanes\compact_replay_verification_gates\Cargo.toml -- --run-dir extractor\native\runs\weather_visual_drivers_live_870ebf_integrated_20260701_v1 --require-domain weather_live --min-game-hours 5 --write-report extractor\native\runs\weather_live_full_replay_residuals_870ebf_20260701_v2\compact_replay_gate.weather_live.json
```

Result: `passed=true`, errors `[]`, warnings `[]`; existing
`native_session.battleplan` verified for `native_6124_18be0901914bf9fc`.

DLL-only: yes for the existing production weather stream; no new DLL stream was
captured in this blocker/rejection continuation; sandstorm still needs shared
registration.

Compact stream: yes for existing domain 24; no additional compact record was
needed for the rejected/blocked residuals.

Rust decoded: yes for the existing production weather stream via the replay
gate. No new Rust decoder was needed for the rejected `+0x78` enum candidate or
blocked particle root.

`.battleplan` verified: yes for the existing production domain-24 stream; no
for the sandstorm bit specifically until shared registration.

Speed-five/current-equivalent hourly-or-lower: yes for the existing production
weather run: `50` game hours advanced, `51` unique hours, hourly requirement
met.

Zero skipped hours: yes, `0`.

Partial/source/worker/bridge failures: no for the verified production weather
run (`source_unavailable_seen=false`, `partial_data_seen=false`,
`bridge_unavailable_seen=false`, `worker_panic_seen=false`).
