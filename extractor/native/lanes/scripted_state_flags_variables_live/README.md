# scripted_state_flags_variables_live lane
> Active lane note: this README is the only active lane-local documentation. Historical plans, proof reports, blocker notes, and registration packets for this lane are archived under ../../../documents/archive/native_lanes/scripted_state_flags_variables_live/. Current extraction completion status lives in ../../../documents/MASTER_EXTRACTION_CONTRACT.md.


Lane scope: opaque scripted/causal state for faithful replay or provenance badges.
This lane intentionally captures keys and raw values, not semantic
mod-specific classifications.

## A38 scripted-GUI residual acquisition

The generic engine-materialized scripted-GUI residual is an independent d84
carrier, not the unresolved d51 `scripted_gui_random` save bag described below.
Its exact-20 static authority, typed carrier, recorder fold, replay view, and UI
evaluator are implemented. The remaining empirical frame can be acquired with
`../../scripts/Invoke-A38ScriptedGuiAcquisition.ps1`; the runner attaches to one
exact-a729 process or launches `--continuelastsave`, retries the read-only
manager scan, and applies `verify_a38_live_probe.py` to every candidate.
Pass `-OwnerFixtureMatrix` to retain all attempts and require the same-process
stable-nonempty, committed-empty, unavailable, and prior-to-current removal
matrix through `verify_a38_owner_fixture.py`; single-capture mode remains
diagnostic convenience only.

An accepted raw capture is intentionally not d84 authority. The in-process
producer must still compile every required engine result and cross the H4CT
winner authorizer. `collector::AuthenticatedA38FrameSource` checks exact manager
order, effective registry inventories, winner identities, and tombstones
derived from an opaque previously committed manager receipt before compact
encoding. The dormant exact-a729 producer now hooks exact owner
the sole shared A25 setter detour, and joins images through the shared generic
sprite-setter observer. It seals a pointer-free current-manager/materialized-
value receipt only after the owner returns, with a strictly increasing
same-process producer sequence. The collector regrades that receipt against
H4CT winners/registry coverage before d84 encoding; arbitrary caller-supplied
current order is not accepted. Production activation remains compile-time
false pending the live fixture matrix. Supplemental exact-a729 lifecycle evidence
proves the rebuild temporarily zeroes the manager before repopulation, so a
header-only zero is unavailable. The diagnostic probe admits raw measured-empty
only when manager generation `+0x68` is stable and non-sentinel around the
complete snapshot; portable publication still requires the in-process
authorizer. The `+0xA50` rows cache resolved element/property names, not final
property values; exact text/image/frame/layout values must be observed at
synchronized widget consumer
boundaries or remain unavailable. See
`../../../documents/dossiers/A38_AUTONOMOUS_LIVE_ACQUISITION_20260720.md`.

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
`extractor/native/runs/scripted_state_flags_variables_live_870ebf_20260701_current_roots_v1/current_build_probe.json`.

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
`extractor/native/runs/scripted_state_flags_variables_live_870ebf_20260701_flag_payload_v1/country_flags_payload_probe.json`.

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
`extractor/native/runs/scripted_state_flags_variables_live_870ebf_20260701_global_state_roots_v1/global_state_root_probe.json`
and
`extractor/native/runs/scripted_state_flags_variables_live_870ebf_20260701_global_state_roots_v1/global_state_root_probe_broad_scan_pre_direct_fix.json`.

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
`extractor/native/runs/scripted_state_flags_variables_live_870ebf_20260701_state_scope_roots_v1/state_scope_root_probe.json`.

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
- legacy opaque delayed-event/scripted-GUI badge kinds for decoding older d51
  evidence; the A13 production carrier never emits those families.

Fail-closed compact status bits reserved lane-locally:

- `STATUS_FLAG_NAMES_PARTIAL`
- `STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED`
- `STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED`

Domain-51 lane-owned mini stack:
`scripted_state_domain51_collector.rs`, `scripted_state_compact.rs`,
`scripted_state_domain51_gate.rs`, and `scripted_state_domain51_stream.rs`.

The stream verifier remains a dependency-free, direct-`rustc` binary. It uses
source-relative `#[path]` modules for the canonical H4CR parser, base status
flags, and build-profile bounds under `../../compact_contract/src`, so these
commands work from the repository root without Cargo or `--extern`:

```powershell
$source = 'extractor\native\lanes\scripted_state_flags_variables_live\scripted_state_domain51_stream.rs'
$verifier = Join-Path $env:TEMP 'scripted_state_domain51_stream.exe'
$tests = Join-Path $env:TEMP 'scripted_state_domain51_stream_tests.exe'
rustc --edition=2021 $source -o $verifier
& $verifier --verify-json 'C:\path\to\collector_stream.bin' --min-game-hours 48
rustc --edition=2021 --test $source -o $tests
& $tests --nocapture
```

The lane-local unresolved delayed-event and scripted-GUI bits remain legacy
decode badges; the shared framing parser does not reinterpret those domain-51
status flags. The A13-only production carrier requests neither family.
Certification duration is explicit and inclusive: `--min-game-hours N` means
the raw stream must contain `N + 1` consecutive healthy closed hours. The gate
accepts unrelated canonical domains in the complete collector stream, but
requires exactly one domain-51 PERF/DATA pair in every closed hour, preserves
the DATA payload's delta row counts/status as PERF authority, and requires the
physical final record to be a healthy authoritative `FRAME_END` with no
trailing record or unframed bytes. `FRAME_END` emitted/scheduled counts must
also equal the independently observed non-probe PERF/DATA domain roster;
reserved diagnostic domain 250 PERF records require a canonical PDU1 trailer
and do not inflate that roster, while domain-250 DATA is forbidden. Unrelated
domain PERF status contributes to the exact globally interpreted FRAME_END
status OR. Every bit in the canonical fatal mask (including partial-data) and
every unknown status bit is rejected by this standalone production gate; it
does not grant lane-local expected-partial exceptions.
The verifier exits nonzero and emits its JSON report to stderr on any failed
authority check; `"ok": false` is never a successful CLI result.

Proof artifact:
`extractor/native/runs/scripted_state_flags_variables_live_870ebf_20260701_domain51_ministack_v1/domain51_ministack_proof.json`.

The mini stack uses final registry domain id `51`, encodes validated
country/global/state variables and flags, emits token-only flag rows when names
do not resolve, and treats delayed-event/scripted-GUI missing roots as
fail-closed opaque badge rows rather than `STATUS_PARTIAL_DATA`. Validated
scope read failures still set `STATUS_PARTIAL_DATA`.

H4CR stream and `.battleplan` proof artifact:
`extractor/native/runs/scripted_state_flags_variables_live_870ebf_20260701_domain51_h4cr_v1/domain51_replay_pack_proof.json`.

The stream proof emits a lane-local H4CR `collector_stream.bin` with one
domain-51 perf record, one domain-51 data record, and one frame-end record per
sample hour. The lane gate decodes both domain-51 payloads, reports zero skipped
hours, and keeps delayed-event/scripted-GUI unresolved-root badges out of
partial/source failure. The replay proof packs the stream into
`native_session.battleplan` and verifies that the embedded
`collector_compact_stream` bytes match the source stream. The shared
collector/recorder registration slot is wired for live proof.

Shared live proof artifact:
`extractor/native/runs/domain51_shared_live_20260701T163828/native_session_manifest.json`.
It scheduled only `scripted_state_flags_variables_live`, emitted `162`
domain-51 compact data records across `162` frame hours, advanced `161` game
hours with `0` skipped hours, had no partial/source/worker/bridge failures, and
passed shared `pack_replay verify` for `native_session.battleplan`.

Opaque-roots full closeout artifact:
`extractor/native/runs/scripted_state_flags_variables_live_20260701_opaque_roots_full_final/FINAL_REPORT.md`.
The refreshed proof revalidated current-build country/global/state variable and
flag roots, then ran a shared domain-51-only compact capture for `43` frame
hours. The lane-owned Rust stream gate decoded `43` domain-51 data records with
`0` skipped hours and no partial/source/worker/bridge failures; `pack_replay
verify` accepted the resulting `native_session.battleplan`. The companion
scope-container audit found exactly the manifest's world/country/state
flag/variable bags plus `countries.*.delayed_event` and
`countries.*.scripted_gui_random`; there are no additional manifest-listed
province/unit/character/combat script flag or variable containers for A13 to
chase. The old whole-concept classification bundled unrelated lifecycle and
UI-materialization work into d51. Current ownership assigns those to d64/A19
and d84/A38, so they are not A13 completion blockers.

Delayed-event blocker proof:
`extractor/native/runs/scripted_state_flags_variables_live_870ebf_20260701_delayed_event_blocker_v1/delayed_event_fixture_audit.json`.

The blocker proof rejects `saved_event_target` as a proxy for queued country
`delayed_event` payloads. The current save-oracle output and save keytrees have
zero `delayed_event` key/scalar hits, while `coverage_audit/memory_manifest.json`
classifies `countries.*.delayed_event` as `save_parsed = no` and
`saved_event_target` as a distinct top-level world key already closed by
`parse_world`. Live delayed-event root work remains blocked until a positive
non-empty `countries.*.delayed_event` fixture or equivalent engine invariant is
available.

Opaque-root positive fixture contracts:
`extractor/native/runs/scripted_state_flags_variables_live_870ebf_20260701_opaque_fixture_contracts_v1/OPAQUE_ROOT_FIXTURE_CONTRACTS.md`.

The contract proof uses `coverage_audit/recursive_paths_country.txt` to pin the
raw save shapes needed for future validation:

- `countries.<id>.delayed_event`, including `event`, `days`, `hours`,
  `months`, `originator`, and `scope`
- `countries.<id>.scripted_gui_random`

These contracts remain historical evidence for the separate d64/A19 and d84/A38
owners. A13 neither emits their rows nor uses their absence to set
`STATUS_PARTIAL_DATA` or `STATUS_SOURCE_UNAVAILABLE` on the validated
country/global/state variable and flag snapshot.

## Current production disposition

A13/d51 now owns and production-decodes the complete current global, country,
and state variable/flag snapshot:

- variables retain exact signed fixed5 `i64` values;
- flags retain exact nonzero engine token, DateHour, and `u32` value;
- a flag name is used only when authenticated; otherwise numeric token identity
  remains available and no text is guessed. The current in-process collector
  does not perform a dictionary-text join, so current production flags are
  intentionally token-only;
- independently acquired variable and flag rosters must agree exactly and every
  scope must be measured or measured-empty;
- the entire family is rejected if acquisition crosses the frame's engine-hour
  boundary or any root/header/row/object identity tears;
- compact rows are canonicalized by semantic identity, the recorder applies
  exact baseline/delta/tombstone lifecycle, and replay exposes global, country,
  and state exact values and flag states.

Delayed-event lifecycle is d64/A19. Engine-materialized scripted-GUI runtime
state is d84/A38. They are intentionally excluded from the A13 carrier and are
not A13 production blockers. Other scope kinds positively observed in the
engine scope resolver remain unavailable rather than being aliased to one of
the three supported scopes.

Current exact-20 static evidence, production wiring, capture contract, and
human-input disposition are recorded in
`../../../documents/dossiers/A13_SCRIPTED_STATE_PRODUCTION_CLOSEOUT_20260720.md`.
The machine-readable capture contract is
`../../../documents/dossiers/A13_RUNTIME_CAPTURE_CONTRACT_20260720.json`, with
offline grader `verify_a13_runtime_capture.py`.
