# scripted_state_flags_variables_live lane

Lane scope: opaque scripted/causal state for faithful replay or provenance badges.
This lane intentionally captures keys and raw values, not semantic
mod-specific classifications.

## Production-ready slice

Country script variables have a current-build validated live root:

- root: `CCountry + 0x218 -> CVariables`
- RTTI gate: `.?AVCVariables@@`
- header: `container + 0x18` node base, `container + 0x20` low `u32` size and
  high `u32` bucket mask
- node grid: `0x30` byte stride; `std::string` key at `node + 0x00`;
  raw fixed-point `i64` value at `node + 0x20`; scale `100000`
- completeness gate: accept only a phase whose distinct decoded key count
  equals the header size with no duplicate key

The lane-local extractor in `country_variables_extractor.rs` preserves the same
fail-closed contract and keeps values as raw `i64` so the compact stream does
not lose precision.

## Current 870ebf live validation

Proof artifact:
`replay_native/runs/scripted_state_flags_variables_live_870ebf_20260701_current_roots_v1/current_build_probe.json`.

The lane-local live probe gates the exact executable SHA-256
`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`, resolves
`CGameState` from `module_base + 0x3316E90`, validates the country table at
`CGameState + 0x310` and count at `CGameState + 0x31C`, and accepts only
country rows whose vtable equals `module_base + 0x27D0E90` and whose
`CCountry + 0x08` tag-db id equals the table index.

Result on live `hoi4.exe` pid `45880`:

- country count: `440`; validated country rows: `434`
- `CCountry + 0x218 -> CVariables`: validated on all 434 validated rows:
  `98` measured non-empty, `336` measured-empty, `0` unmeasured; `5062`
  variable entries measured in emitted rows
- first non-empty sample: Greater Germanic Reich, `CVariables`
  `0x22BDA941C70`, declared size `284`, exact phase `8`
- `CCountry + 0x230 -> CFlagManager`: root validated by RTTI
  `.?AVCFlagManager@@` on all 434 validated rows; flag payload/token/date
  mapping remains unresolved and must stay fail-closed for named flags
- bounded Ghidra text sweep corroborated older `CVariables`/`param_1+0x218`
  static hints in `ghidra_hoi4_proj/dump_world/_WORLD.txt` and
  `sv_FUN_140535a40_140535a40.c`, but current-build acceptance is based on the
  live hash-gated RTTI and table evidence above

## Current 870ebf country flag payload validation

Proof artifact:
`replay_native/runs/scripted_state_flags_variables_live_870ebf_20260701_flag_payload_v1/country_flags_payload_probe.json`.

The lane-local payload probe validates the `CFlagManager` row shape on current
`870ebf`:

- `CFlagManager + 0x08`: payload row base
- `CFlagManager + 0x10`: low `u32` slot/capacity count, high `u32` live entry
  count
- row stride: `0x30`
- row `+0x00`: token subrecord vtable `0x7FF6AD6FB318`; row `+0x08` low `u32`
  token id
- row `+0x10`: date subrecord vtable `0x7FF6AD6F2388`; row `+0x18` low `u32`
  Jomini DateHour
- row `+0x20`: value subrecord vtable `0x7FF6AD6F23B8`; row `+0x28` low `u32`
  flag value
- token dictionary records use vtable `0x7FF6AE05D170`, token id at
  `record + 0x08`, and token text pointer at `record + 0x10`

Result on live `hoi4.exe` pid `45880`:

- country rows scanned: `434`
- non-empty `CFlagManager` payloads: `107`; measured-empty managers: `327`
- country flag rows decoded: `936`
- dates decoded: `936/936`
- names resolved: `934/936`; ambiguous names: `0`; unresolved names: `2`
- unresolved tokens: `0x8E87` for United Kingdom row 31
  (`1946.7.14.4`, value `1`) and `0xECF2` for Imperial Japan row 7
  (`1936.5.20.1`, value `1`); both have token-dictionary records whose text
  pointers failed read, so these two rows must remain token-only/badged

Rejected payload misreads:

- using the low `u32` of `CFlagManager + 0x10` as entry count: rejected because
  it reaches spare/sentinel rows when low > high; high `u32` validates as the
  live entry count
- reading token/value as full `u64`: rejected because the high dword is
  auxiliary/padding on many otherwise resolved rows; the low `u32` maps to the
  token dictionary and value

## Current 870ebf global root validation

Proof artifacts:
`replay_native/runs/scripted_state_flags_variables_live_870ebf_20260701_global_state_roots_v1/global_state_root_probe.json`
and
`replay_native/runs/scripted_state_flags_variables_live_870ebf_20260701_global_state_roots_v1/global_state_root_probe_broad_scan_pre_direct_fix.json`.

The direct-root probe validates these current-build roots:

- `CGameState + 0x980 -> CVariables`: direct qword at `0x22BA5F49450`
  resolves to `0x22BDBBCFA30`, RTTI `.?AVCVariables@@`, vtable
  `0x7FF6AD7AF0D0`; parser gate measured `695` global variable entries and
  matched `256/260` save-oracle world variable keys.
- `CGameState + 0x258 -> CFlagManager`: direct qword at `0x22BA5F48D28`
  resolves to `0x22BDBB4FB70`, RTTI `.?AVCFlagManager@@`, vtable
  `0x7FF6AD936BF8`; parser gate measured `102` global flag rows using the
  same token/date/value row layout as country `CFlagManager`.

The preserved broad scan artifact resolves all `102` global flag names and
finds `42` save-oracle world-flag overlaps. Its old direct-root rejection is
invalidated by the corrected chunked direct-reference scanner in
`global_state_root_probe.py`.

## Current 870ebf state root validation

Proof artifact:
`replay_native/runs/scripted_state_flags_variables_live_870ebf_20260701_state_scope_roots_v1/state_scope_root_probe.json`.

The state-scope probe resolves state identity through the current production
state path without touching shared entrypoints:

- `CGameState + 0x2B0`: CProvince pointer array
- `CGameState + 0x2BC`: CProvince count, observed `13414`
- `CProvince` vtable: `module + 0x295AF68`
- `CProvince + 0xC0 -> CState`
- `CState` vtable: `module + 0x29206C0`
- `CState + 0x58`: state id

Result on live `hoi4.exe` pid `45880`:

- valid province rows: `10272`
- validated state rows: `1081`
- `CState + 0x7F8 -> CVariables`: validated on all `1081` states; `363`
  non-empty, `718` measured-empty, `676` variable entries
- `CState + 0x540 -> CFlagManager`: validated on all `1081` states; `479`
  non-empty states, `806` flag rows; rows decode with the same
  token/date/value payload shape as country/global `CFlagManager`

Rejected state candidates:

- embedded `CVariables` object in `CState[0x0..0x4000]`: `0` qword-aligned
  vtable hits
- embedded `CFlagManager` object in `CState[0x0..0x4000]`: `0`
  qword-aligned vtable hits
- shifted duplicate pointer offsets such as `CState + 0x1188` and
  `CState + 0xED0`: rejected as production roots because they fail the
  all-state measured/measured-empty gate

The lane-local compact schema in `scripted_state_compact.rs` reserves production
domain id `51`. Lower candidate ids are rejected/reserved for other lanes:
`44` supply, `45` focus/research/decisions, `46` air, `47` characters, `48`
construction, and `49`/`50` naval combat.

It supports:

- global, country, and state scopes;
- raw fixed-point script variables;
- named flags when key/value/date are known;
- token-only flags when only the live engine token is known;
- opaque delayed-event/scripted-GUI rows once roots are pinned.

Fail-closed compact status bits reserved lane-locally:

- `STATUS_FLAG_NAMES_PARTIAL`
- `STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED`
- `STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED`

Domain-51 lane-owned mini stack:
`scripted_state_domain51_collector.rs`, `scripted_state_compact.rs`,
`scripted_state_domain51_gate.rs`, and `scripted_state_domain51_stream.rs`.

Proof artifact:
`replay_native/runs/scripted_state_flags_variables_live_870ebf_20260701_domain51_ministack_v1/domain51_ministack_proof.json`.

The mini stack uses final registry domain id `51`, encodes validated
country/global/state variables and flags, emits token-only flag rows when names
do not resolve, and treats delayed-event/scripted-GUI missing roots as
fail-closed opaque badge rows rather than `STATUS_PARTIAL_DATA`. Validated
scope read failures still set `STATUS_PARTIAL_DATA`.

H4CR stream and `.battleplan` proof artifact:
`replay_native/runs/scripted_state_flags_variables_live_870ebf_20260701_domain51_h4cr_v1/domain51_replay_pack_proof.json`.

The stream proof emits a lane-local H4CR `collector_stream.bin` with one
domain-51 perf record, one domain-51 data record, and one frame-end record per
sample hour. The lane gate decodes both domain-51 payloads, reports zero skipped
hours, and keeps delayed-event/scripted-GUI unresolved-root badges out of
partial/source failure. The replay proof packs the stream into
`native_session.battleplan` and verifies that the embedded
`collector_compact_stream` bytes match the source stream. The shared
collector/recorder registration slot is wired for live proof; remaining
completion claims must stay scoped to the validated slice.

Shared live proof artifact:
`replay_native/runs/domain51_shared_live_20260701T163828/native_session_manifest.json`.
It scheduled only `scripted_state_flags_variables_live`, emitted `162`
domain-51 compact data records across `162` frame hours, advanced `161` game
hours with `0` skipped hours, had no partial/source/worker/bridge failures, and
passed shared `pack_replay verify` for `native_session.battleplan`.

Delayed-event blocker proof:
`replay_native/runs/scripted_state_flags_variables_live_870ebf_20260701_delayed_event_blocker_v1/delayed_event_fixture_audit.json`.

The blocker proof rejects `saved_event_target` as a proxy for queued country
`delayed_event` payloads. The current save-oracle output and save keytrees have
zero `delayed_event` key/scalar hits, while `coverage_audit/memory_manifest.json`
classifies `countries.*.delayed_event` as `save_parsed = no` and
`saved_event_target` as a distinct top-level world key already closed by
`parse_world`. Live delayed-event root work remains blocked until a positive
non-empty `countries.*.delayed_event` fixture or equivalent engine invariant is
available.

Opaque-root positive fixture contracts:
`replay_native/runs/scripted_state_flags_variables_live_870ebf_20260701_opaque_fixture_contracts_v1/OPAQUE_ROOT_FIXTURE_CONTRACTS.md`.

The contract proof uses `coverage_audit/recursive_paths_country.txt` to pin the
raw save shapes needed for future validation:

- `countries.<id>.delayed_event`, including `event`, `days`, `hours`,
  `months`, `originator`, and `scope`
- `countries.<id>.scripted_gui_random`

Until a fixture satisfies those contracts, delayed-event and scripted-GUI roots
must remain opaque fail-closed badge rows only. These unresolved roots must not
set `STATUS_PARTIAL_DATA` or `STATUS_SOURCE_UNAVAILABLE` for the validated
country/global/state variable and flag subset.

## Explicit non-claims

These are not production decoded by this lane yet:

- country flags: `CCountry + 0x230 -> CFlagManager` payload, date, value, and
  most names are validated for current `870ebf`; two observed rows remain
  token-only because their token dictionary text pointers failed read.
- global variables/flags: current `870ebf` roots are pinned at
  `CGameState + 0x980 -> CVariables` and `CGameState + 0x258 -> CFlagManager`;
  shared DLL/Rust integration emits the validated slice.
- state flags/variables: current `870ebf` roots are pinned at
  `CState + 0x7F8 -> CVariables` and `CState + 0x540 -> CFlagManager`; shared
  DLL/Rust integration emits token/date/value rows, and unresolved names remain
  token-only/badged until a dictionary pass.
- delayed events: `saved_event_target` is rejected as a validation proxy, and
  the current save oracle has no non-empty `countries.*.delayed_event` fixture;
  keep `STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED` fail-closed.
- scripted GUI open state: save/audit rows establish why it matters for causal
  replay, but no live production root is pinned here.

## Integration patch plan

1. Keep domain id `51` as `scripted_state_flags_variables_live` in the registry.
   Promote it from reserved/non-schedulable to active/schedulable only through
   the orchestrator-owned registration slot.
2. In the collector scheduled-hour path, iterate the existing validated country
   identity rows and call the lane extractor on each `CCountry` object.
3. Add global and state scope reads from the validated roots, then wrap the
   domain-51 payload in the H4CR stream shape proven by
   `scripted_state_domain51_stream.rs`.
4. Add the recorder decode gate by moving or including `scripted_state_compact.rs`
   and `scripted_state_domain51_gate.rs`, then add only the domain-51 decoder
   dispatch/registration hook in shared recorder code.
5. Promote country, global, and state roots from the lane-local proof artifacts;
   keep delayed events and scripted GUI state fail-closed until separate proof
   artifacts validate exact roots and zero guessed data.
