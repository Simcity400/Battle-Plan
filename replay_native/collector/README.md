# collector

In-process Battle Plan collector DLL.

This crate is intentionally separate from the native replay app. It is loaded
inside `hoi4.exe`, while the recorder and replay UI stay external. The current
ABI exposes version/heartbeat functions plus a Windows message-hook procedure
used for the no-op attach proof.

Production recording target: `hoi4.exe` -> this DLL collector -> compact
raw/event/snapshot stream -> `.battleplan` file through a Rust/native recorder.
Python is allowed as a development harness for attaching, proof gates, manifest
validation, replay validation, and temporary bounded discovery only. Python must
not be required to walk memory hourly/sub-hourly or keep up with speed five.
`replay_native/recorder_host` is the current Rust/native host under construction
for this path; it captures the DLL pipe stream, writes the cadence manifest, and
packs a manifest-covered `.battleplan` container for the raw stream.

A collector segment is complete only after the DLL/Rust path captures it at
hourly-or-better cadence, with no skipped game hours, at speed five or a measured
equivalent. If a segment depends on Python preflight/root discovery, that segment
is temporary debt until the discovery moves into the DLL/Rust host or is proven
one-time, bounded, cached, and outside recording.

On process attach the DLL starts a short worker. In debug/JSON mode it emits a
`collector_started` JSON frame to the sidecar log and, when a recorder is
listening, to the named pipe `\\.\pipe\battle-plan_collector_<pid>`. In compact
gate mode it bypasses full JSON root frames and writes packed records to the
pipe. When loaded inside the supported `hoi4.exe` build, the debug root probe
includes:

- executable code sentinels for the daily-update and long-string offsets;
- live current/start game dates decoded from the game-state root;
- the country database root, bounded tag-table sample, and active
  country-present rows.

Frames include a monotonic producer sequence id. The Python proof harness may
annotate pipe reads with byte count and CRC32 for validation, but production
recording must not depend on Python in the live capture path.

Current cadence contract: bounded proof workers default to 10 ms sampling, but
speed-five country-domain proof runs may request 1 ms sampling or `sample_ms=0`
true no-sleep mode through the native host. JSON frames are debug/proof
visibility only. Production feasibility is measured with the compact DLL/Rust
stream: typed in-memory domain rows/counters, fixed binary record headers, and
Rust-side ordering/decode/skipped-hour validation. Daily update hooks or daily
rows are not replay coverage.

For hourly proof runs, the recorder writes a short-lived
`collector_control.json` beside the DLL before attach. The DLL treats that as a
bounded worker request, samples verified roots at the requested interval, emits
JSONL debug frames or compact binary records over a per-run tokenized named
pipe, and exits before the hook is removed. Bounded worker runs disable sidecar
file writes by default; the pipe is the authoritative transport. This does not
patch HOI4 code. Worker mode is honored only inside `hoi4.exe`; the
controller-side DLL load remains a one-shot attach heartbeat.

The legacy Python session wrapper can reassemble pipe frames into a validated
`collector_frames.jsonl` stream inside a development run directory. The
production replacement is the Rust/native recorder that consumes compact
`collector_stream.bin` records, verifies them, and writes `.battleplan` output
without Python in the hot loop. The raw stream keeps same-hour samples instead
of discarding them; that is the bridge for sub-hour combat, movement, and event
capture.

The root probe is intentionally narrow. It proves attach, transport, build
sentinels, clock decoding, and country identity row emission before any
trampoline hook or broad object traversal is added.

Filtered compact country sessions use `compact_country_frame` to emit a full
initial keyframe followed by DLL-side sparse per-country updates. When a
scheduled-domain filter is present, worker polls do not emit full root snapshots;
they emit only explicitly requested raw domains. A filtered country session does
not emit a separate raw clock delta unless `clock` is requested; scheduled root
snapshots still carry the canonical clock. This is required for speed-five proof
because unrelated late-game combat streams, raw-clock pipe writes, and repeated
all-country unchanged payloads can otherwise consume the hourly budget.

2026-06-30 speed-five country-core status: the collector now merges
`politics_live` and `politics_residuals` reads when both are requested, caches
static definition tokens inside the DLL worker, resets those caches at worker
start, guards cached script-token hits with live string-header checks plus
vtable/type/identity validation where available, and uses a compact country
identity fast path that reuses verified tag/name identity while still checking
the `CCountry` vtable and expected `tag_db_id` every scheduled frame. The
collector also uses frozen per-loop clock samples so the scheduled root frame is
stamped with the same game hour that triggered the boundary. Current release
proof `country_core_no_raw_clock_sample0_speed5_v1` used `sample_ms=0`, advanced
864 game hours in 45 seconds, emitted 865 scheduled root frames, supplied 6474
idea-group rows plus 3722 advisor-token rows, captured `focus_live`, `tech_live`,
`politics_live`, and `politics_residuals`, and skipped 0 game hours. This proves
the compact raw-stream cadence for that slice through the DLL/Rust path; it
predates the later Rust-native `.battleplan` packing checkpoint.

2026-06-30 compact-stream gate status: the Rust host now supports
`--stream-format compact --extraction-perf-gate`. Compact records carry
sequence id, `game_hour_raw`, domain id, record type, payload length, packed
metric payloads, and the first typed replay payload: `fuel_live` country fuel
rows as tag-db-id keyed raw fixed-point `i64` values with explicit `32768`
scale metadata. The host writes `collector_stream.bin`, packs it into
`native_session.battleplan` under role `collector_compact_stream`, verifies the
container, verifies the stored stream byte-for-byte, verifies decodability,
monotonic ordering, byte volume, frame timing, every scheduled domain on every
frame hour, one fuel data record per fuel frame, worker-panic status, and zero
skipped game hours. JSON is no longer a production throughput target.

The full compact country extraction gate now passes live speed five through one
DLL collector/coordinator. The DLL keeps one clock source and one sequence/order
authority, while the compact gate internally splits country extraction into two
read-only lanes with separate dirty caches: fuel/production and the remaining
country-state domains. Current compact binary proofs:

- `compact_fuel_production_speed5_v1`: fuel + production, 20 seconds, 284 game
  hours advanced, 285 frame-end records, 0 skipped, avg frame 23.973 ms, p95
  39.567 ms, max 76.525 ms, 59,280 bytes.
- `compact_country_state_nonprod_speed5_v1`: focus + tech + politics +
  politics residuals + decisions + diplomacy, 20 seconds, 247 game hours
  advanced, 248 frame-end records, 0 skipped, avg frame 42.434 ms, p95
  57.007 ms, max 109.462 ms, 123,008 bytes.
- `compact_all_country_parallel_coverage_speed5_v2`: all current compact
  country domains together, 30 seconds, 332 game hours advanced, 333 frame-end
  records, 0 skipped, full per-hour domain coverage, no worker panic, avg frame
  38.151 ms, p95 48.743 ms, max 156.203 ms, 213,120 bytes.
- `compact_fuel_replay_container_speed5_v1`: all current compact country
  domains plus typed `fuel_live` replay data records and Rust-native
  `.battleplan` packing, 30 seconds, 291 game hours advanced, 292 frame-end
  records, 0 skipped, full per-hour domain coverage, no worker panic, 292 fuel
  data records for 292 fuel frames, 11,611 typed fuel rows, avg frame
  44.869 ms, p95 57.064 ms, max 116.131 ms, 667,672 raw stream bytes, and a
  verified 86,012-byte `native_session.battleplan`.
- `compact_division_equipment_wide_speed5_20260630_v2`: fuel, production,
  focus, tech, politics, politics residuals, decisions, diplomacy, divisions,
  division equipment, land combats, map control, map detail, and weather
  together, 60 seconds, 113 game hours advanced, 114 unique frame hours, 0
  skipped, full per-hour domain coverage, ordered and decodable stream, no
  worker panic/source-unavailable/partial/bridge flags, avg frame 26.648 ms,
  p95 28.680 ms, max 74.934 ms, 5,777,247 raw stream bytes, and a verified
  1,694,343-byte `native_session.battleplan`. The new `division_equipment`
  domain emitted 114 typed data records with 155,040 division rows and 348,368
  equipment-entry units through the DLL-native anchored division root.
- `compact_wide_semantic_counters_speed5_20260630_v1`: the same 14-domain
  production-shaped compact bundle, rebuilt with recorder semantic counters,
  advanced 105 game hours in 60 seconds with 106 unique frame hours, 0 skipped,
  full per-hour domain/data-record coverage, ordered and decodable stream, and
  no worker-panic/source-unavailable/partial/bridge flags. It measured avg frame
  44.608 ms, p95 64.820 ms, max 103.623 ms, wrote 4,695,510 compact bytes, and
  verified a 1,315,765-byte `.battleplan`. The manifest now reports decoded
  proof counters for replay-critical units including 5,194 stockpile rows,
  2,578 military-line rows, 1,856 naval-line rows, 1,474 construction rows,
  1,206 active research-slot rows, 91 timed-decision rows, 16,588 moving
  division rows, 5,948 in-combat division rows, 6,160 division-equipment entry
  rows, 6,533 land-combat participant rows, 718 state-resource rows, 6,968
  building rows, and 451 active weather-region rows.
- `compact_army_orders_wide_speed5_20260630_v2`: adds compact `army_orders` to
  the wide bundle for 15 scheduled domains, 60 seconds, 105 game hours
  advanced, 106 unique frame hours, 0 skipped, full per-hour domain/data-record
  coverage, ordered and decodable stream, and no worker-panic,
  source-unavailable, partial-data, or bridge flags. It measured avg frame
  47.094 ms, p95 60.541 ms, max 121.066 ms, wrote 6,386,939 compact bytes, and
  verified a 2,031,491-byte `.battleplan`. The DLL derives active CArmy roots
  from anchored live division parents and emitted 106 typed `army_orders` data
  records with 43,290 observed army rows, 42,226 observed live orders, 436
  changed order rows, 19 delete rows, and 966 exact target-state rows. This is
  DLL-native for active armies reachable through live divisions; full OOB roots,
  empty armies, and deeper non-type-5 order geometry remain open.
- `compact_land_oob_weather_870ebf_speed5_20260630_v1`: refreshes the active
  collector profile for the current 2026-06-30 vanilla executable hash
  `870EBF1F615262FACCA48A3B947775C41F0A5338F3068C40958209F5C41C65AD` and proves
  the updated land-command vtables through the compact DLL/Rust path. It
  scheduled `divisions`, `division_equipment`, `army_orders`, `oob_command`,
  and `weather_live` together for 30 seconds, advanced 57 game hours with 58
  unique frame hours and 0 skipped, had full domain/data-record coverage,
  ordered and decodable records, no worker-panic/source-unavailable/partial or
  bridge flags, `division_root_source = "dll_native"`, avg frame 19.299 ms, p95
  22.352 ms, max 23.016 ms, 2,800,072 compact bytes, and a verified
  `.battleplan`. The new compact `oob_command` domain emitted active army and
  army-group rows, leader/skill rows, owner rows, member division/army rows,
  engine member counts, and missing-member-count diagnostics. This is not full
  OOB completion: empty armies, fleets, task forces, theatres, and roots not
  reachable through live divisions remain open.
- `compact_country_870ebf_speed5_20260701_v2`: refreshes the current-build
  country-state vtable validators for the same executable hash and proves the
  eight compact country domains together without JSON root frames. It scheduled
  `fuel_live`, `production_live`, `focus_live`, `tech_live`, `politics_live`,
  `politics_residuals`, `decisions_live`, and `diplomacy_live` for 45 seconds,
  advanced 86 game hours with 87 unique frame hours and 0 skipped, had full
  domain/data-record coverage, ordered and decodable records, no worker-panic,
  source-unavailable, partial-data, or bridge flags, avg frame 14.702 ms, p95
  17.691 ms, max 23.922 ms, 640,154 compact bytes, and a verified
  `.battleplan`. The run emitted real typed rows for the previously empty
  current-build domains: 1,005 active research-slot rows, 1,832 popularity rows,
  356 politics-residual country rows, 397 decision-country rows, 50 timed
  decision rows, and 5,727 stockpile rows.
- `compact_full_current_870ebf_speed5_20260701_v2`: refreshes the current-build
  map and land-combat validators, then proves all 16 implemented compact domains
  together on the same executable hash. It scheduled the eight country domains,
  `divisions`, `division_equipment`, `army_orders`, `oob_command`,
  `land_combats`, `map_control`, `map_detail`, and `weather_live` for 60
  seconds, advanced 108 game hours with 109 unique frame hours and 0 skipped,
  had full domain/data-record coverage, ordered and decodable records, no
  worker-panic/source-unavailable/partial or bridge flags, avg frame 35.455 ms,
  p95 38.570 ms, max 87.869 ms, 6,985,286 compact bytes, and a verified
  `.battleplan`. The run emitted real typed units across the full bundle,
  including 52,781 division rows, 10,040 division-equipment entry rows, 410
  army-order rows, 594 active land-OOB rows, 1,818 land-combat rows, 14,765
  land-combat participant rows, 1,081 map-control state rows, 13,414
  map-control province rows, 718 state-resource rows, 2,417 state-building
  rows, 4,721 province-building rows, and 480 weather-region rows.
- `compact_ships_native_870ebf_20260701_010232`: proves compact `ships`
  through a DLL-native anchored naval path on the same executable hash. The
  collector walks `CGameState+0x310 -> CCountry+0x278 -> CFleet*`, then bounded
  `CFleet` task-force references, validated `CTaskForce` bodies, and validated
  `CShip` rows. The 20-second ships-only speed-five gate advanced 39 game hours
  with 40 unique frame hours and 0 skipped, had ordered/decodable compact
  records, no worker-panic/source-unavailable/partial or bridge flags, avg
  frame 33.733 ms, p95 35.008 ms, max 35.618 ms, 147,677 compact bytes, and a
  verified `.battleplan`. The stream emitted 2,692 changed ship rows, including
  65 delete rows, while observing 55,195 ship rows and 6,318 task-force rows.
- `compact_full_plus_ships_870ebf_20260701_010320`: reruns the current-build
  compact bundle with `ships` included as the 17th domain. It scheduled the
  eight country domains, `divisions`, `division_equipment`, `army_orders`,
  `oob_command`, `land_combats`, `map_control`, `map_detail`, `weather_live`,
  and `ships` for 30 seconds, advanced 53 game hours with 54 unique frame hours
  and 0 skipped, had full domain/data-record coverage, ordered and decodable
  records, no worker-panic/source-unavailable/partial or bridge flags, avg frame
  47.441 ms, p95 57.250 ms, max 73.141 ms, 5,237,329 compact bytes, and a
  verified `.battleplan`. `ships` emitted 3,133 changed rows while observing
  74,160 ship rows and 8,473 task-force rows; ship read p95 was 43.519 ms.
- `compact_oob_ships_native_870ebf_20260701_nav_oob_v2`: widens compact
  `oob_command` with DLL-native naval hierarchy rows from the same anchored
  country/fleet/task-force/ship path. The 20-second focused gate scheduled
  `oob_command` plus `ships`, advanced 38 game hours with 39 frame-end records
  and 0 skipped, had ordered/decodable compact records, no worker-panic/
  source-unavailable/partial or bridge flags, avg frame 50.756 ms, p95
  52.189 ms, max 52.223 ms, 216,303 compact bytes, and a verified
  `.battleplan`. `oob_command` emitted 914 changed rows and captured real naval
  hierarchy counters: 122 fleet rows, 202 task-force rows, 414 task-force member
  ids, 1,615 ship member ids, 3,939 observed fleet rows, and 6,268 observed
  task-force rows.
- `compact_full_plus_naval_oob_870ebf_20260701_v1`: reruns the full 17-domain
  compact bundle with the widened naval OOB schema. It advanced 53 game hours
  with 54 frame-end records and 0 skipped, had full domain/data-record coverage,
  ordered and decodable records, no worker-panic/source-unavailable/partial or
  bridge flags, avg frame 59.167 ms, p95 70.233 ms, max 80.460 ms, 4,655,364
  compact bytes, and a verified `.battleplan`. `oob_command` emitted 950 changed
  rows, including 124 fleet rows, 195 task-force rows, 404 task-force member ids,
  1,625 ship member ids, 5,441 observed fleet rows, and 8,638 observed
  task-force rows; OOB read p95 was 55.044 ms.
- `compact_oob_ships_owner_870ebf_20260701_v4`: proves owner-attributed compact
  fleet/task-force OOB rows without returning to bridge or JSON-root throughput.
  The 20-second focused gate scheduled `oob_command` plus `ships`, advanced 39
  game hours with 40 frame-end records and 0 skipped, had ordered/decodable
  compact records, no worker-panic/source-unavailable/partial or bridge flags,
  avg frame 49.321 ms, p95 50.727 ms, max 50.949 ms, 415,231 compact bytes, and
  a verified `.battleplan`. `oob_command` captured 945 changed rows, 924 owner
  rows, 135 fleet rows, 210 task-force rows, 419 task-force member ids, 1,604
  ship member ids, 463 missing-member-count rows, 4,400 observed fleet rows, and
  6,886 observed task-force rows; OOB read p95 was 44.204 ms.
- `compact_full_plus_oob_owner_870ebf_20260701_v2`: reruns the full 17-domain
  compact bundle after adding fleet/task-force owner attribution. It advanced 53
  game hours with 54 frame-end records and 0 skipped, had full domain/data-record
  coverage, ordered and decodable records, no worker-panic/source-unavailable/
  partial or bridge flags, avg frame 58.891 ms, p95 72.759 ms, max 87.120 ms,
  4,651,183 compact bytes, and a verified `.battleplan`. `oob_command` captured
  997 changed rows and 964 owner rows, including 152 fleet rows, 233 task-force
  rows, 536 task-force member ids, 1,661 ship member ids, 474
  missing-member-count rows, 5,940 observed fleet rows, and 9,243 observed
  task-force rows; OOB read p95 was 59.421 ms.

This completes the extraction-only compact performance gate for the current
implemented compact domains and starts replay payload capture with typed
records plus Rust-native `.battleplan` packing. `fuel_live`, `production_live`,
`focus_live`, `tech_live`, `politics_live`, `politics_residuals`,
`decisions_live`, `diplomacy_live`, `divisions`, `division_equipment`,
`army_orders`, active land `oob_command`, `land_combats`, `map_control`,
`map_detail`, `weather_live`, and compact `ships` are live-proven through the
DLL/Rust compact path at speed five with zero skipped game hours for the
applicable verified build profile. Compact `oob_command` now includes DLL-native
owner-attributed fleet and task-force hierarchy rows for the verified `870ebf`
build. The next step is not more JSON widening; it is native compact coverage
for direct per-ship owner/name fields, naval leaders, missions, theatre/region
state, naval/air combat telemetry, and richer combat/order details that are
still raw-only, bridge-backed, or unimplemented.

`ships` now has a reserved compact domain (`domain_id=29`) and Rust recorder
decoder/schema coverage for per-ship hull telemetry, including CShip instance
id, optional task-force id, hull definition token, category enum, raw
strength/org/XP, sunk-convoy count, and deletion rows. For the verified
`870ebf` vanilla build profile, the collector now emits this domain from the
DLL-native country-rooted fleet/task-force/ship path with
`ship_root_source="dll_native"` and no discovery debt. The old bridge-gated raw
ship proofs remain development evidence only; they are no longer the current
compact production path for ship telemetry.

Country-object heap discovery is not part of normal capture. The experimental
whole-heap vtable scan is behind the explicit `country_object_scan` control flag,
defaults off, and is not speed-five safe. Runtime country/economy capture must be
root-anchored or hook-derived, cached before the hot loop, and reduced to bounded
pointer validation plus direct reads during recording.

The normal DLL country-scalar source is the `CGameState+0x310` tag-db-id-indexed
`CCountry*` table. Each candidate object is accepted only after heap/alignment
checks, country vtable validation, object-owned `tag_db_id` validation, tag-table
decode, stable name checks, and duplicate rejection. The older bounded
`CountryDataBase` object-local scan still exists as fallback evidence; on the
current live Portugal process it fails closed as
`country_database_root_insufficient`. Political power is read on the same
validated country-object path by following `CCountry+0xF90` to
`CPoliticalStatus`, validating the political-status vtable and
`CPoliticalStatus+0xC8` country backlink, then decoding fixed-point
`CPoliticalStatus+0xE0`.

Temporary country-scalar / record bridge debt:

- the Python proof harness may run `live_capture.collector_country_bridge` as a
  preflight step before bounded worker recording, but this is not production
  complete for any segment that depends on it;
- normal recording first reuses bounded same-process caches; `country_array.json`
  stores the located record-array token, `country_objects.json` is for
  object-backed scalars, and `country_records.json` is fallback evidence for
  future record-only fields;
- object-cache hits revalidate each cached country object against the country
  vtable, its own `tag_db_id`, and the live tag table before any row is passed
  to the DLL;
- record-cache hits are validated against current object rows and each record's
  back-pointer; record rows merge additively and must not shrink the object
  bridge;
- when the object cache is missing, `--allow-country-cold-scan` is an explicit
  bootstrap/cache-populate mode. It may run the slow country-object vtable scan
  and reverse record scan once, then normal recording should return to the
  bounded cache path;
- preflight rows are written only to the temporary `collector_control.json` as a
  compact `tag_db_id:country_object_ptr` string plus a separate
  `tag_db_id:country_record_ptr` string;
- manifests record bridge status and row count, not raw addresses;
- the DLL rejects malformed, duplicate, unaligned, or unexpected-tag rows;
- accepted rows are still revalidated and scalar values are re-read for every
  emitted frame;
- `country_object_scan` remains false for this path.

The record bridge is no longer required for political power. It remains a
bootstrap/fallback route and should only become a normal dependency again if a
replay-critical country value proves to exist only in the record array.

Scheduled country politics:

- scheduled snapshot frames may claim `politics_live` when the validated country
  object source is still valid;
- politics reads are rooted at the already validated `CCountry*`, then follow
  `CCountry+0xF90` to `CPoliticalStatus`;
- the collector validates the political-status vtable and owner backlink,
  validates each `CPoliticalParty*` through the party vtable and its
  `CIdeologyGroup`, reads popularity from `party+0x88`, and requires the
  ruling-party pointer at `CPoliticalStatus+0xD0` to be a member of the parties
  vector for the low-confidence flag to remain false;
- active idea tokens are read from the `CPoliticalStatus+0x50` vector by exact
  size, with each `CIdea*` gated by the known CIdea vtables before reading
  `CIdea+0x18`;
- scheduled snapshot frames may also claim `politics_residuals` from the same
  validated `CPoliticalStatus`: `last_election` is decoded from the stable
  Jomini DateHour `i32` at `+0xA0`, and `elections_allowed` is read as a stable
  `0/1` byte at `+0xEC`;
- appointed-advisor extraction is emitted only when collector control supplies
  a validated static advisor contract sourced from `game_defs` /
  `common/characters`; the collector filters the live active-ideas vector,
  enriches hired tokens with static `{slot, ledger, character}` metadata as
  `appointed_advisor_details`, `appointed_advisor_slots`, and
  `appointed_advisor_ledgers`, does not sweep `CAdvisor` heap objects in the
  hot path, omits the leaves when the contract is missing or invalid, preserves
  measured-empty `[]` as real, and is live-proven by current strict proof
  `replay/snapshots/_collector_session_politics_advisor_gate/20260629T212028Z_pid43412`.
  That run passed `--require-politics-advisor-residuals`, supplied 3722
  vanilla `common/characters` advisor tokens, advanced 12 game hours in 46.047
  wall seconds, verified replay output, emitted 13 scheduled
  `politics_residuals` frames, and observed 4602 country rows with all four
  appointed-advisor fields present. The live values were measured-empty in this
  run, so non-empty appointed-advisor values still need a fixture with a hired
  advisor;
- typed laws, non-law active ideas, and national spirits are emitted when
  collector control supplies the static `common/ideas` idea-group contract. The
  collector classifies the live active-ideas vector into `laws` groups
  (`economy`, `trade_laws`, `mobilization_laws`), ordered broad `ideas_other`,
  complete `idea_groups`, active cabinet/designer `advisor_ideas`, and narrower
  `national_spirits` for `common/ideas` category `country` without requiring a
  save. Current strict proof
  `replay/snapshots/_collector_session_politics_idea_gate/20260629T211245Z_pid43412`
  passed `--require-politics-idea-classification`: it supplied 6474 idea-group
  rows, advanced 12 game hours in 45.797 wall seconds, verified replay output,
  emitted 12 scheduled `politics_live` frames, folded 4248 active-idea/law/
  idea-group/non-law/national-spirit rows plus 1104 advisor-idea rows, and kept
  worker-poll frames free of politics payloads;
- current no-cold proof on pid 43412 advanced 12 game hours, stored 13 verified
  snapshots, emitted 12 scheduled `politics_live` and 12 scheduled
  `politics_residuals` frames, observed up to 356 politics and residual rows per
  frame, and kept worker-poll frames free of politics payloads;
- worker-poll frames stay light and must not carry politics payloads.

Scheduled country decisions:

- scheduled snapshot frames may claim `decisions_live` when the validated
  country object source is still valid;
- decisions are rooted at `CCountry+0xFA0 -> CDecisionStatus`; the collector
  validates the status vtable and owner back-pointer before accepting a row;
- `status_present` carries non-empty decision-status vector counts, and
  `timed_live` decodes active `CTimedDecision` rows from the `+0xA0` list using
  `CDecision+0x120` definition names, nonnegative days, state, and state name;
- measured-empty `timed_live = []` is a real result, protected by the Python
  frame adapter only for tags actually measured by `decisions_live`;
- `collector_live_politics_decisions_probe` found non-empty active timed
  decisions for 7 countries while Portugal had measured-empty `timed_live = []`.

Scheduled country focus:

- scheduled snapshot frames may claim `focus_live` when the validated country
  object source is still valid;
- focus-tree id reads follow `CCountry+0x1370 -> CFocusTree` and decode the
  tree id from the definition string at `+0x08`;
- focus status reads follow `CCountry+0x1380` to the focus submanager, then
  decode the completed-focus compact vector at `+0x40/+0x48/+0x4C`, current
  national focus at `+0x10`, current continuous focus at `+0x18`, focus ids at
  `+0x18`, and progress days from `+0x38`;
- completed-focus `[]` and `completed_count = 0` are measured values, not
  missing data. The Python frame adapter records the TAGs measured by
  `focus_live` so save backfill cannot overwrite a real measured-empty list;
- `current` and `current_continuous` are treated as mutually exclusive by the
  adapter. `paused` remains save-only until a live byte is pinned;
- live proof `collector_live_focus_probe` advanced 2 game hours with zero
  skipped hours, emitted 3 scheduled `focus_live` frames, 1065 country focus
  rows, and zero focus payloads on worker-poll frames; Portugal measured
  `focus_tree = portuguese_focus`, `completed_count = 22`, current
  `POR_standardization`, and progress `69`;
- worker-poll frames stay light and must not carry focus payloads.

Scheduled country research:

- scheduled snapshot frames may claim `tech_live` when the validated country
  object source is still valid;
- the first DLL research slice is rooted at `CCountry+0xF60 ->
  CTechnologyStatus`; the collector validates the technology-status vtable,
  owner backlink at `+0x130`, completed-tech vector header
  `+0x40/+0x48/+0x4C`, and the bounded research-slot pointer vector
  `+0xA0/+0xA8/+0xAC`;
- emitted leaves are `completed_tech_count`, `research_slot_count`, and
  `active_research[]` rows `{slot_index, tech_handle_raw, tech_def_index_raw,
  progress_raw, progress_units, live_progress_raw, live_progress_units,
  base_required_points_raw, base_required_points_units, tech_name?}`. The
  Python frame adapter folds them into
  `countries.*.research.tech_count`, `.research_slots`, and `.in_progress_live`;
- `progress_units = progress_raw / 1000` is emitted directly from the validated
  slot field. `tech_name` is emitted only when `tech_def_filter` supplies a
  validated live tech-definition handle map; missing names are omitted instead
  of guessed;
- moving research progress comes from `CResearchSlot+0x18 -> CTechnology+0x198`
  as `live_progress_raw/live_progress_units`. `progress_units` is still emitted
  for compatibility but is not accepted as moving progress;
- base research denominator comes from the active tech object
  `CResearchSlot+0x18 -> CTechnology+0x160 -> CTechnologyTemplate+0x3e0`,
  scaled through the engine's `TECH_BASE_COST` helper. It is emitted as
  `base_required_points_raw/base_required_points_units`; it is not an effective
  cost or research-speed modifier claim;
- current no-cold proof
  `replay/snapshots/_collector_session_research_denominator_gate/20260629T192125Z_pid43412`
  used the tech-definition cache, emitted 37 scheduled `tech_live` frames
  across 36 game hours, captured 15687 named active research-slot rows and 15687
  live base-denominator rows with up to 424 rows per frame, produced 346
  derived ETA rows after a positive daily progress delta, verified replay
  output, and kept research payloads off worker polls. A current-game rerun
  `replay/snapshots/_collector_session_research_denominator_gate/20260629T204418Z_pid43412`
  hit the tech-definition cache in 0.109s, emitted 38 scheduled `tech_live`
  frames across 36 game hours, captured 16562 named active research-slot rows,
  16126 DLL-base progress rows, and 708 ETA rows, verified replay output, and
  kept research payloads off worker polls;
- worker-poll frames stay light and must not carry research payloads.

Scheduled special projects:

- scheduled snapshot frames may claim `projects_live` when the profile
  sentinels match and either `special_projects_bridge` supplies validated roots
  or the explicit fallback `special_projects_scan = true` is supplied in
  `collector_control.json`;
- the current proof path is bridge-first debt: Python preflight resolves or
  validates cached `facility:<id>:<province>:0xptr`,
  `project:<id>:<static_def_handle>:0xptr`, and `scientist:<char_id>:0xptr`
  rows, the control file passes only that ephemeral bridge to the DLL, and the
  DLL revalidates vtables/types/identity before every scheduled read;
- the older DLL project slice can still scan for `CProjectFacility`, `CProject`,
  and scientist-backed `CCharacter` objects once per collector worker when
  explicitly requested, but that heap sweep remains disabled for normal runs;
- emitted top-level `special_projects` rows have `source = "C"` and include
  facilities, scientists, and currently worked project rows. Facility active
  project handles and project static-def handles are raw engine ids, not
  resolved tokens;
- the slice intentionally omits project progress/completion, owner attribution,
  and resolved project definition names until those roots/joins are separately
  pinned;
- the Python adapter rejects pointer/tag-db diagnostic keys, folds only frames
  that explicitly claim `projects_live`, and preserves measured-empty `[]`
  payloads when the DLL emits them;
- preflight/session manifests record bridge status, row counts, cache status,
  and `address_manifest_policy = "omitted"`; raw addresses are never written to
  durable manifests;
- the 2026-06-29 pid 43412 no-cold acceptance proof hit
  `special_projects_cache`, advanced 12 game hours, verified replay output,
  emitted 12 scheduled `projects_live` frames, and kept worker-poll frames free
  of `special_projects` payloads. A current-game rerun reused the scoped
  `_preflight_cache` from the same `out-root`, hit `special_projects_cache` in
  0.375s, advanced 12 game hours in 38.969 wall seconds, verified replay output,
  supplied 21548 bridge rows, emitted 14 scheduled `projects_live` frames, and
  kept worker-poll frames free of `special_projects` payloads. The latest
  strict-payload proof
  `replay/snapshots/_collector_session_project_roots_bootstrap/20260629T224333Z_pid43412`
  hit the no-cold cache with 21496 bridge rows, advanced 12 game hours in
  40.781 wall seconds, verified replay output, emitted 12 scheduled
  `projects_live` frames, folded 288 facility rows, 348 worked-project rows,
  and 0 scientist rows, kept worker-poll frames free of `special_projects`
  payloads, and passed `--require-projects-live-payload`;
- worker-poll frames stay light and must not carry `special_projects`.

Scheduled country fuel:

- scheduled snapshot frames may claim `fuel_live` when the validated country
  object source is still valid;
- fuel reads are rooted at the already validated `CCountry*`, then follow
  `CCountry+0x1580` to the fuel-status object and decode current/max plus
  fuel gain/cost as signed 64-bit fixed point divided by 32768;
- malformed, non-heap, negative, or implausibly large fuel values are omitted
  per country instead of guessed; gain/cost may be signed and are range bounded;
- worker-poll frames stay light and must not carry fuel payloads;

Scheduled country production:

- scheduled snapshot frames may claim `production_live` when the validated
  country object source is still valid;
- production reads are rooted at the already validated `CCountry*`, then follow
  `CCountry+0xF68` to the production manager;
- stockpile reads use the compact vector at manager `+0x220/+0x228/+0x22C`
  and gate every non-null entry through the equipment-variant vtable
  `module+0x293AFB8` plus type `70` on the current 2026-06-30 vanilla build;
- military/naval production lines use the compact pointer vector at
  manager `+0x58/+0x60/+0x64` and accept line type `56` or `57`;
- unavailable stockpile or line vectors are omitted, while measured-empty `{}`,
  `[]`, and `0` survive as measured values;
- non-null bad-gated stockpile entries, duplicate variant ids, broken vector
  headers, and implausible fixed-point values fail closed for that sub-vector;
- live proof on pid 43412 advanced 12 game hours in 45.734 wall seconds, stored
  13 verified snapshots, emitted 12 scheduled `production_live` frames and 12
  `fuel_live` frames, and observed up to 356 production country rows, 20
  military lines, 12 naval lines, and 146 stockpile entries per frame;
- worker-poll frames stay light and must not carry production payloads.

Recorder-side map-control contract:

- collector frames may carry `states` rows for `state_ownership` /
  `state_control` and `provinces` rows for `province_control`;
- the Python adapter folds only domains explicitly claimed by the frame;
- non-TAG owner/controller values are rejected before reaching snapshots;
- province control is mirrored into `map.control_by_province` for replay UI use;
- session manifests summarize maximum country/state/province row counts across
  the raw frame stream;
- the control file may carry `map_control_state_roots`,
  `map_control_province_bridge`, and `map_control_controller_bridge`;
- the Rust side parses those control contracts, rejects malformed/duplicate/
  out-of-range rows, and reports validity/row counts in frame control metadata;
- scheduled frames now prefer the static `CGameState+0x2B0` `CProvince*`
  array for map control: `CProvince+0xC0` yields state objects for
  `state_ownership` / `state_control`, and `CProvince+0x188` yields province
  controller tag-db ids;
- the same scheduled state-object path now emits `state_detail` resource
  deposits by reading `CState* + 0x30` to the static definition and its
  resource table at `def+0xF8/+0x100/+0x104`; these are raw state deposits, not
  country resource production/export/import totals;
- current no-cold proof on pid 43412 emitted 12 scheduled frames each for
  `state_ownership`, `state_control`, and `province_control` across 12 game
  hours, with 1081 state rows and 10272 province rows per frame;
- current state-resource compact proof
  `compact_state_detail_probe_speed5_20260630_v2` advanced 38 game hours with 0
  skipped, emitted 39 ordered/decodable `map_detail` data records, captured
  42,159 state-detail rows, saw no source-unavailable/partial/parse failures,
  and kept p95 extraction at 2.363 ms. The DLL resource reader now treats a
  non-zero resource-vector capacity lower than size as the logical row count,
  matching the already-proven Python/Ghidra research for the verified
  `size=6/cap=5` trailing `NO_ID` sentinel shape;
- the older `map_control_state_roots`, dense province array, and controller
  bridges remain fallbacks if the static root is unavailable; worker-poll
  frames stay light.

State-root preflight:

- `live_capture.collector_map_bridge` can resolve state-object roots before a
  recording session and cache them by process identity;
- cache hits validate each cached root's vtable and state id, then re-read owner
  and controller ids live;
- diagnostics report row counts and cache status only, never raw addresses;
- the live Portugal proof resolved 1081 state rows cold in about 28.5s and from
  cache in about 0.2s;
- cached state roots are scheduled `state_ownership` / `state_control` input;
- the dense province-control array root remains unresolved/too slow for normal
  recording until its relocation/cache path is proven.

Land-combat raw stream status:

- `live_capture.collector_frames` and the Rust collector define and test a
  bounded raw `land_combats` domain-delta/event contract:
  `land_combat:<combat_id>:<generation>` keys, province/age scalars, nullable
  attacker/defender division-id lists, and no raw address leakage.
- The Rust DLL now samples active land combats from the pinned manager root
  `CGameState+0x260`: vector data @ +0x08, count @ +0x14, manager slot to body
  delta +0x10. The old Python heap-vtable scan is retained only as a research
  cross-check.
- The live reader validates the manager, body vtable/type/id, front object,
  registry identity, side vtables/counts, and division participant references
  before emitting samples. Body addresses stay internal and are represented only
  by the generation component of the replay key.
- Invalid manager slots are skipped when at least one active combat row validates,
  matching the existing Python reader's fail-closed partial enumeration. If no
  combat rows validate, manager/layout failures emit bounded `confidence:"gap"`
  markers instead of disappearing silently.
- Combat starts/participant or age updates, generation splits, and ends now emit
  explicit `event` frames alongside the existing `domain_delta` samples/deletes,
  so downstream replay code does not have to infer lifecycle transitions from
  deltas alone. A 2026-06-29 no-cold live proof against pid 43412 advanced 12
  game hours in 39.266 wall seconds, verified replay output, emitted 2485 raw
  domain-delta frames, 2419 raw event frames, 42 `land_combat_started`, 2368
  `land_combat_updated`, and 9 `land_combat_ended` events, and reported zero
  raw gap frames. The stricter raw military proof
  `replay/snapshots/_collector_session_raw_military_gate/20260629T194359Z_pid43412`
  also passed `--require-land-combat-raw-lifecycle`, advancing 12 game hours in
  62.688 wall seconds and observing 160 raw land-combat deltas, 8 starts, 151
  updates, 1 end, and zero raw gap domains.
- Remaining promotion work is richer combat-tick/participant canonical events
  and raw `land_combats` ingestion into the replay timeline/app.

Army-order stream status:

- The production-shaped compact path now captures active army orders without
  the Python army bridge. In compact mode, the DLL derives CArmy roots from
  anchored live division bodies (`CDivision+0xB0`), validates CArmy/order
  vtables and handles, and emits typed `army_orders` payloads through the
  Rust recorder. `compact_army_orders_wide_speed5_20260630_v2` live-proved this
  at speed five with 105 game hours advanced, 0 skipped, 106 data records,
  43,290 observed army rows, 42,226 observed live orders, 436 changed order
  rows, 19 delete rows, and 966 target-state entries. This covers active armies
  reachable through live divisions; empty armies, full OOB roots, and deeper
  non-type-5 order geometry remain open.
- `live_capture.collector_frames` and the Rust collector also define and test a
  bounded raw `army_orders` domain-delta/event contract:
  `army_order:<army_id>:<generation>` keys, army id, slot index, order type,
  nullable target state ids, target-state count, target status, and no raw
  address leakage.
- The Rust DLL accepts `army_body_bridge` rows in `collector_control.json` as
  `army_id:body_ptr` entries. The parser rejects malformed, duplicate,
  unaligned, out-of-range, or oversized payloads and reports row count/validity
  in control metadata.
- The Python proof harness can populate that bridge with
  `live_capture.collector_session --preflight-army-orders`; by default it is
  cache-first and fails closed unless `--allow-army-cold-scan` explicitly allows
  the slower army vtable sweep. This bridge is retained for historical/debug raw
  fixtures only and is not the production compact path for active army orders.
  Manifests store sanitized counts/status only.
- Worker sampling does not sweep the heap. For each supplied army row, the DLL
  revalidates the `CArmy` body by vtable, type, and id, then reads the active
  order vector at `CArmy+0x98` at a 250 ms floor.
- The first stream emits `army_order_added`, `army_order_updated`, and
  `army_order_removed`. Type-5 target-state vectors are exact when they fit the
  bounded inline payload; larger valid vectors emit `target_status:"overflow"`
  with a count; non-type-5 geometry emits `target_status:"unsupported"`.
- `army_order_updated` is covered by a Rust tracker regression for
  same-address/same-army target-state mutation. The stricter 2026-06-29 raw
  military gate live-proved order presence at speed five with 547 raw
  `army_orders` deltas and 547 `army_order_added` events over 12 game hours,
  but correctly failed the `--require-army-order-raw-updates` gate because no
  existing order changed during the capture window. A later cache-backed
  fixture run advanced 36 game hours in 45.266 wall seconds, resolved 580 army
  bodies from cache in 0.078s, verified replay output, emitted 519 army-order
  events, observed one `army_order_removed`, and reported zero raw gap domains.
  Raw-stream inspection found no same-army/same-slot replacement near that
  removal, so replacement-as-update remains unproven for that fixture. A later
  48-hour cache-backed speed-five run resolved 496 army bodies from cache in
  0.078s, verified replay output, emitted 463 adds and 13 removals with zero raw
  gap domains, and still failed `--require-army-order-raw-updates`. Promotion now
  requires a live fixture that redraws/resizes an already-sampled type-5
  offensive order during the capture window. A second 49-hour fixture window
  resolved 425 army bodies from cache in 0.078s, verified replay output, emitted
  390 adds and one removal with zero raw gap domains, and also failed the update
  gate.
- Remaining promotion work is a live speed-five order-edit fixture, semantic
  event splits, native full OOB/empty-army roots, non-type-5 geometry, and
  timeline ingestion.

Naval-combat raw stream status:

- `live_capture.collector_frames` and the Rust collector define and test a
  bounded raw `naval_air_combats` domain-delta/event contract:
  `naval_combat:<combat_id>:<generation>` keys, nullable location, side TAG
  lists, side ship counts, optional commander tokens, optional positioning
  values, side validation statuses, and no raw address leakage.
- The Rust DLL accepts `naval_combat_body_bridge` rows in
  `collector_control.json` as `combat_id:body_ptr` entries. The parser rejects
  malformed, duplicate, unaligned, out-of-range, or oversized payloads and
  reports row count/validity in control metadata.
- The Python proof harness can populate that bridge with
  `live_capture.collector_session --preflight-naval-combat`. This runs the
  existing `CombatReader.enumerate_naval_combats()` path before attach, writes
  only ephemeral bridge rows into the temporary control file, and stores
  sanitized counts/status in manifests. This is production debt until native
  root/hook discovery replaces the Python preflight.
- Worker sampling does not sweep the heap. For each supplied bridge row, the DLL
  revalidates the naval-combat body, registry, side, member, and leader vtables
  before emitting a row. Invalid bridge/layout reads become bounded
  `confidence:"gap"` rows instead of guessed combat data.
- The current stream emits side-level combat summaries and explicit
  `naval_combat_started`, `naval_combat_updated`, and `naval_combat_ended`
  events. The same bridge also feeds `naval_combat_members` domain-delta rows,
  one per active `CFEXMember`, keyed by combat generation, side, and either
  warship instance id or convoy group/member/slot. Rows include display
  identity, owner TAG, designed max-strength raw value, battle-slot index,
  state bitmask, and engagement-weight raw value, with no body/member address
  leakage.
- Current CShip hull HP/org/XP now have a DLL-native compact `ships` stream for
  the verified `870ebf` build. The collector walks the country fleet vector,
  validates fleet/task-force/ship vtables and handle type/id fields, and emits
  pointer-free compact rows with definition, category, strength/org/experience
  raw values, sunk-convoy counts, task-force membership, and delete rows.
- The older `ship_body_bridge` raw stream and
  `live_capture.collector_session --preflight-ships` proof harness remain
  development/oracle tooling only. They must not be used as the production
  speed-five recording path now that the compact collector has a native
  anchored source. Manifests store sanitized counts/status only.
- Targets, aggregate attack rows, damage ticks, hit counters, sinking events,
  ship names, owner attribution, missions, and UI damage totals remain follow-up
  streams pending live proof or separate command/OOB sources.
- Live proof on pid 43412: a no-cold 12-game-hour run resolved 3 active naval
  combats during preflight in 26.031s outside the hot loop, emitted raw
  `naval_air_combats` and `naval_combat_members`, verified replay output,
  reported zero raw gap frames, and observed 3 `naval_combat_started` plus 360
  `naval_combat_updated` events. `naval_combat_ended` still needs a fixture
  where a bridged naval battle ends during the capture window.
- Live proof on pid 43412:
  `replay/snapshots/_collector_session_air_ship_oob_gate/20260629T210118Z_pid43412`
  passed the strict `--require-ship-raw-telemetry` gate. The no-cold run hit
  `ship_body_cache` in 0.125s, supplied 1309 bridge rows, emitted 22249 raw
  `ships` deltas / 22248 object rows, advanced 12 game hours in 57.297 wall
  seconds, verified replay output, and reported no raw gap domains. This proves
  the DLL extraction path; full-game recording still needs raw-stream
  coalescing/backpressure because combined ship/OOB sidecars can be large.

OOB command raw stream status:

- Land command membership and historical naval command membership have a
  bridge-gated raw `oob_command` stream. The Rust DLL accepts `oob_command_bridge` rows in
  `collector_control.json` as `kind:id:body_ptr` entries. The parser rejects
  malformed, duplicate, unaligned, out-of-range, or oversized payloads and
  reports row count/validity in control metadata.
- The Python proof harness can populate that bridge with
  `live_capture.collector_session --preflight-oob-command`; by default it is
  cache-first and fails closed unless `--allow-oob-command-cold-scan`
  explicitly allows the slower `oob_command_live.enumerate_command_objs()`
  vtable sweep. This raw/oracle path remains development debt; the production
  compact path no longer depends on it for active land OOB or verified
  fleet/task-force membership on the `870ebf` build.
- Current emitted rows are `oob_command:army_group:<army_group_id>`,
  `oob_command:army:<army_id>`, `oob_command:fleet:<fleet_id>`, and
  `oob_command:task_force:<task_force_id>`. Rows expose army-group member
  armies, army member divisions, army parent groups, army leader ids, bounded
  army leader skill values, assigned army leader trait tokens, engine
  member-count diagnostics, fleet task-force membership, task-force parent
  fleet, and bounded CShip instance-id membership from the live
  `CTaskForce+0x338/+0x344` vector. Owner, names, missions, locations, theatre
  membership, leader portraits, temporary/captured trait state, naval leaders,
  global commander rosters, and save-shaped nested OOB records are not yet
  claimed.
- Compact `oob_command` no longer depends on the raw naval bridge for fleet and
  task-force hierarchy in the verified `870ebf` build. The production compact
  path reuses the DLL-native `CGameState+0x310 -> CCountry+0x278 -> CFleet* ->
  CTaskForce/CShip` walk, emits kind `fleet` rows with task-force members and
  kind `task_force` rows with parent fleet plus ship members, and passed the
  focused plus full speed-five/current-equivalent gates listed above with zero
  skipped hours and no source/partial/bridge failures. Fleet rows now carry the
  explicit owning country object tag-db id from the country-rooted walk, and
  task-force rows inherit that owner from the parent fleet; the focused owner
  gate captured 924 owner rows and the full 17-domain owner gate captured 964
  owner rows with zero skipped hours. Validation-failed task-force member reads
  preserve the engine count as `missing_member_count` instead of packing an
  undecodable logical member-count mismatch.
- Remaining promotion work is a live active-naval-combat fixture, a
  root/hook-derived discoverer for combats born after preflight, speed-five
  soak, exact damage/sinking semantics, and timeline ingestion.
- Live proof on pid 43412: an explicit cold bootstrap populated
  `oob_command_roots`, then a no-cold 12-game-hour run resolved 4704 still-live
  OOB roots from cache in 0.109s, emitted raw `oob_command`, verified replay
  output, and reported no raw gap domains. The stricter raw military gate later
  captured 17705 `oob_command` deltas across 12 game hours with zero raw gap
  domains after an intentionally allowed 135.703s cold preflight. Treat that
  cold discovery time as cache/root-refresh work, not normal hot-loop cost.
  Army leader-detail proof is stricter than generic OOB membership:
  `--require-oob-command-leader-detail` requires exact army rows with
  leader id/status, skill, trait list, and exact trait status co-present. The
  no-cold run
  `replay/snapshots/_collector_session_oob_leader_gate/20260629T215306Z_pid43412`
  advanced 12 game hours in 54.110 wall seconds, verified replay output,
  emitted 19842 raw `oob_command` deltas with zero raw OOB gaps, and captured
  7485 exact army leader-detail rows with 15911 trait tokens.

Air-wing raw stream status:

- `live_capture.collector_frames` and the Rust collector define and test a
  bounded raw `air_wings` domain-delta/event contract:
  `air_wing:<TAG>:<wing_id>:<generation>` keys, owner TAG, wing id, generation,
  count, mission mask, nullable mission region, effective active planes, and no
  raw address or tag-db-id leakage.
- The Rust DLL accepts `air_wing_body_bridge` rows in `collector_control.json`
  as `owner_tag_db_id:wing_id:body_ptr` entries. The parser rejects malformed,
  duplicate, unaligned, out-of-range, or oversized payloads and reports row
  count/validity in control metadata.
- The Python proof harness can populate that bridge with
  `live_capture.collector_session --preflight-air-wings`; by default it is
  cache-first and fails closed unless `--allow-air-wing-cold-scan` explicitly
  allows the slower `NavalAirReader.enumerate_air_wings()` vtable sweep. This is
  production debt until native discovery replaces the Python preflight.
  Manifests store sanitized counts/status only.
- Worker sampling does not sweep the heap. For each supplied bridge row, the
  DLL revalidates the air-wing body vtable/type/id, pool vtable, owner-chain
  country vtable, owner tag id, live tag-table decode, optional mission-region
  vtables, and sane count/effective-plane bounds at a 250 ms floor.
- The current stream emits `air_wing_observed`, `air_wing_updated`, and
  `air_wing_lost`. `effective_planes` is the live active/effective plane count
  used for later aggregation; it is not the final displayed air-superiority
  percent.
- Current pid 43412 fixture status: live-proven by no-cold cache-backed run
  `replay/snapshots/_collector_session_air_wing_gate/20260629T205254Z_pid43412`.
  The run advanced 12 game hours in 55.875 wall seconds, verified replay output,
  hit `air_wing_body_cache` in 0.094s, supplied 585 bridge rows, emitted 11115
  raw `air_wings` deltas, observed 585 `air_wing_observed` and 10530
  `air_wing_updated` events, and reported zero raw gaps.
- Remaining promotion work is live proof while changing missions, exact
  airbase/variant joins, derived region aggregate rows, air-loss/attrition
  proof, and timeline ingestion.

Runtime rules:

- read-only observation only;
- no save-file dependency;
- unsupported builds must fail closed before domain readers run;
- no heap-wide discovery in the live recording loop;
- hook callbacks must stay tiny and hand work to transport/worker code.

Backpressure rule for future hooks:

- the hook path enqueues bounded descriptors only;
- the worker thread owns serialization and pipe/shared-memory writes;
- diagnostics and optional high-frequency deltas are dropped before scheduled
  snapshots;
- dropped scheduled snapshots become explicit manifest gaps;
- transport stalls fail closed instead of blocking the game thread.
