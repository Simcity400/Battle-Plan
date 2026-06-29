# collector

In-process Battle Plan collector DLL.

This crate is intentionally separate from the native replay app. It is loaded
inside `hoi4.exe`, while the recorder and replay UI stay external. The current
ABI exposes version/heartbeat functions plus a Windows message-hook procedure
used for the no-op attach proof.

On process attach the DLL starts a short worker that emits a `collector_started`
JSON frame to the sidecar log and, when a recorder is listening, to the named
pipe `\\.\pipe\battle-plan_collector_<pid>`. When loaded inside the supported
`hoi4.exe` build, the frame includes a verified root probe:

- executable code sentinels for the daily-update and long-string offsets;
- live current/start game dates decoded from the game-state root;
- the country database root, bounded tag-table sample, and active
  country-present rows.

Frames include a monotonic producer sequence id. The Python recorder annotates
pipe reads with byte count and CRC32 before folding them into replay provenance
and manifest diagnostics.

Current cadence contract: bounded proof workers sample cheaply at 10 ms, the
recorder keeps one canonical replay snapshot per unique game hour, and the raw
`collector_frames.jsonl` stream preserves duplicate same-hour frames for later
high-frequency domains. Daily update hooks or daily rows are not replay
coverage.

For hourly proof runs, the recorder writes a short-lived
`collector_control.json` beside the DLL before attach. The DLL treats that as a
bounded worker request, samples verified roots at the requested interval, emits
JSONL frames over a per-run tokenized named pipe, and exits before the hook is
removed. Bounded worker runs disable sidecar JSONL file writes by default; the
pipe is the authoritative transport. This does not patch HOI4 code. Worker mode
is honored only inside `hoi4.exe`; the controller-side DLL load remains a
one-shot attach heartbeat.

The recorder-side session wrapper reassembles those pipe frames into a validated
`collector_frames.jsonl` stream inside the run directory. The replay snapshot
store still writes one canonical snapshot per game hour, but the raw stream
keeps same-hour samples instead of discarding them. That is the bridge for
sub-hour combat, movement, and event capture.

The root probe is intentionally narrow. It proves attach, transport, build
sentinels, clock decoding, and country identity row emission before any
trampoline hook or broad object traversal is added.

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

Fallback country-scalar / record bridge:

- the Python recorder may run `live_capture.collector_country_bridge` as a
  preflight step before bounded worker recording;
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
  `appointed_advisor_details` and `appointed_advisor_slots`, does not sweep
  `CAdvisor` heap objects in the hot path, omits the leaves when the contract
  is missing or invalid, preserves measured-empty `[]` as real, and is
  live-proven by `collector_live_politics_decisions_probe` at speed five;
- typed laws, non-law active ideas, and national spirits are emitted when collector control
  supplies the static `common/ideas` idea-group contract. The collector
  classifies the live active-ideas vector into `laws` groups
  (`economy`, `trade_laws`, `mobilization_laws`), ordered broad `ideas_other`,
  and narrower `national_spirits` for `common/ideas` category `country` without
  requiring a save. `collector_live_politics_decisions_probe` supplied 6474
  idea-group rows, advanced 2 game hours at speed five with zero skipped hours,
  and measured Portugal laws as `civilian_economy`, `export_focus`, and
  `volunteer_only`;
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
  progress_raw}`. The Python frame adapter folds them into
  `countries.*.research.tech_count`, `.research_slots`, and
  `.in_progress_live`;
- active research rows are raw engine rows only. Tech-token resolution,
  progress percentage, and ETA remain gated on validated tech-def registry,
  static research cost, and live speed inputs;
- worker-poll frames stay light and must not carry research payloads.

Scheduled special projects:

- scheduled snapshot frames may claim `projects_live` only when the profile
  sentinels match and `special_projects_scan = true` is supplied in
  `collector_control.json`;
- the first DLL project slice scans for `CProjectFacility`, `CProject`, and
  scientist-backed `CCharacter` objects once per collector worker, pins
  validated identities, then revalidates and rereads those pins on scheduled
  frames instead of sweeping the heap in the hot path;
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
- this cold scan is disabled by default because the first live attempt exceeded
  its wall cap before any manifest/frame output and the HOI4 process was no
  longer present afterward. Replace it with a bounded root/cache path before
  enabling it in normal or speed-five recording;
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
  `module+0x292B218` plus type `70`;
- military/naval production lines use the compact pointer vector at
  manager `+0x58/+0x60/+0x64` and accept line type `56` or `57`;
- unavailable stockpile or line vectors are omitted, while measured-empty `{}`,
  `[]`, and `0` survive as measured values;
- non-null bad-gated stockpile entries, duplicate variant ids, broken vector
  headers, and implausible fixed-point values fail closed for that sub-vector;
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
  deltas alone. A 2026-06-29 live probe against the running vanilla game advanced
  two game hours with 406 sampled land-combat deltas, 406 lifecycle events, and
  no land-combat gap frames.
- Remaining promotion work is speed-five soak and raw `land_combats` ingestion
  into the replay timeline/app.

Army-order raw stream status:

- `live_capture.collector_frames` and the Rust collector define and test a
  bounded raw `army_orders` domain-delta/event contract:
  `army_order:<army_id>:<generation>` keys, army id, slot index, order type,
  nullable target state ids, target-state count, target status, and no raw
  address leakage.
- The Rust DLL accepts `army_body_bridge` rows in `collector_control.json` as
  `army_id:body_ptr` entries. The parser rejects malformed, duplicate,
  unaligned, out-of-range, or oversized payloads and reports row count/validity
  in control metadata.
- The recorder can populate that bridge with
  `live_capture.collector_session --preflight-army-orders`; by default it is
  cache-first and fails closed unless `--allow-army-cold-scan` explicitly allows
  the slower army vtable sweep. Manifests store sanitized counts/status only.
- Worker sampling does not sweep the heap. For each supplied army row, the DLL
  revalidates the `CArmy` body by vtable, type, and id, then reads the active
  order vector at `CArmy+0x98` at a 250 ms floor.
- The first stream emits `army_order_added`, `army_order_updated`, and
  `army_order_removed`. Type-5 target-state vectors are exact when they fit the
  bounded inline payload; larger valid vectors emit `target_status:"overflow"`
  with a count; non-type-5 geometry emits `target_status:"unsupported"`.
- Remaining promotion work is live speed-five proof while issuing orders,
  semantic event splits, non-type-5 geometry, periodic bridge refresh for new
  armies, and timeline ingestion.

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
- The recorder can populate that bridge with
  `live_capture.collector_session --preflight-naval-combat`. This runs the
  existing `CombatReader.enumerate_naval_combats()` path before attach, writes
  only ephemeral bridge rows into the temporary control file, and stores
  sanitized counts/status in manifests.
- Worker sampling does not sweep the heap. For each supplied bridge row, the DLL
  revalidates the naval-combat body, registry, side, member, and leader vtables
  before emitting a row. Invalid bridge/layout reads become bounded
  `confidence:"gap"` rows instead of guessed combat data.
- The current stream emits side-level combat summaries and explicit
  `naval_combat_started`, `naval_combat_updated`, and `naval_combat_ended`
  events. Per-ship combat participants, damage ticks, hit counters, and sinking
  events remain separate follow-up streams so large battles do not overflow the
  raw JSON frame envelope.
- Remaining promotion work is a live active-naval-combat fixture, a
  root/hook-derived discoverer for combats born after preflight, per-ship
  combat rows, speed-five soak, and timeline ingestion.

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
- The recorder can populate that bridge with
  `live_capture.collector_session --preflight-air-wings`; by default it is
  cache-first and fails closed unless `--allow-air-wing-cold-scan` explicitly
  allows the slower `NavalAirReader.enumerate_air_wings()` vtable sweep.
  Manifests store sanitized counts/status only.
- Worker sampling does not sweep the heap. For each supplied bridge row, the
  DLL revalidates the air-wing body vtable/type/id, pool vtable, owner-chain
  country vtable, owner tag id, live tag-table decode, optional mission-region
  vtables, and sane count/effective-plane bounds at a 250 ms floor.
- The current stream emits `air_wing_observed`, `air_wing_updated`, and
  `air_wing_lost`. `effective_planes` is the live active/effective plane count
  used for later aggregation; it is not the final displayed air-superiority
  percent.
- Remaining promotion work is speed-five live proof while changing missions,
  exact airbase/variant joins, derived region aggregate rows, air-loss/attrition
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
