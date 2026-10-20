# dashboard-air-operations

Backend-only keyed products for air mission registrations and the current annual-ring
statistics bucket. Both use the shared block pre-pass, continuation catalogue and exact
coalesced runs in [dashboard-keyed](../dashboard-keyed/README.md). Queries select one
country's requested pages; no request walks a branch. No installed content are written.

## Registration

The lead registers `products::air_missions::AirMissions` (`air-missions`) and
`products::air_statistics::AirStatistics` (`air-statistics`), their `query` functions,
`types()`, and `FAMILY` statics. Observers are `AirMissionsObserver` and
`AirStatisticsObserver` in those modules. Stores have the same names as the product
keys. Wire types derive TS and the crate implements ReplyWarnings. Product loaders
keep the shared prepared-only refusal.

Wire shapes:

- Mission history: `{window, missions:[{id,name,icon}], regions:[{id,name}], theatres?, warnings}`.
  Names are `TAG@slot|region|m<bit>` (signed plane count) and
  `TAG@slot|region|wings` (unique registered handles). Mission choices include bits
  present in the selected published window, including zero-aircraft registrations.
- Wing detail: `{hour, readHour, region, wings:[{id,name,type,planes,missions,efficiency,refusals}], warnings}`.
  Parameters add the necessary `region` to the task's request sketch; `hour` and
  `region` are required with `view:"wings"`. Full handles are decimal strings.
  Wings retain registration order, once per handle; raw selected bits are listed
  even when their signed mask does not admit a count. Numbers survive naming failures.
- Statistics: `{window, channels:[{id,ours,enemy,divisor}], regions, theatres?, warnings}`.
  Names are `TAG@slot|region|c<0..23>`, `|slot` and `|wrapped`. The observer prepares
  all 24 channels; queries return **only the selected pair plus ring state**. Channel
  IDs are 0..11 in recorded schema order, not GUI button order. Building channels
  use divisor 1,000; all others use 100,000. Every raw word is decimal text in runs.
  The ships pair resolves two independent localisation keys, preserving its asymmetric
  meanings. Labels come from the native detail localisations with an empty value
  argument; authored punctuation is kept rather than parsed as English.

`window.known` separates absent from not-recorded. A missing mission column at a
known hour is not an admitted mission; a recorded zero has a run. A missing statistics
row is absent, never zero. Equal values hold across supplying reads; `window.reads`
dates those reads. Ring changes are observed state, never subtracted into events.
Country names use TAG@slot, so tag reuse does not merge series. Statistics freeze
raw-reference -> slot -> tag bindings at their actual supplying read and carry them
in the saved continuation. An hour whose identity differs from (or cannot show)
the read-time binding leaves the carried attribution unknown; it is known again
once the identity agrees with that binding or a fresh read rebinds it. Unrelated
country changes do not affect the binding. Refused attribution is
unknown, not an absent row or zero. A request may use a bare tag or an exact
TAG@slot. Wing detail refuses a bare tag ambiguous at that hour.

Mission counts share `replay_derive::air_mission_counts` with viewer-counters:
ordered registrations, pool owner rather than volunteer/effective owner, signed-mask
gate, negative-sum restart and wrapping addition. Unique wings are independent of
the overlapping columns. Preparation is sparse in registrations/allocated histories,
not countries times every content region. The 5,000-country test verifies sparse
catalogues and selected-country query size.

## Content and reached details

Strategic-region definitions and localisation use the recorded verified stack with
shared stack/source caches (including failed reads). Names never use a vanilla roster
or copied text. Region names follow air-superiority's content reader/localisation law.
Missing names are counted while raw runs survive. Stored wing/theatre names respect
the recording-language guard. Wing type names join the recorded pool subunit to the
content's subunit database and its localisation.

Wing detail uses `progressive::frame_at` and the shared frame cache. A read hour is
returned only from that frame's slot evidence or a published READ run; no first-hour
read date is fabricated. It can answer a reached frame beyond the history prefix.

Theatre rosters do support region membership through group -> wing -> mission region.
The addendum offers no dated theatre shape: returning a history-wide union would be
wrong after reassignment. Therefore `theatres` is returned only at an explicit mission
`hour` or a one-hour history window; broader histories carry a named counted refusal.
Missing roster operands omit that optional field, independently of numeric history.
The lead should request reached-hour choices, or approve a separate dated choice API.

Efficiency uses `replay_derive::aircraft_count`, the same current sortie law and
base/mission-country joins as the viewer. It is raw fixed5 decimal text
(100,000 = one), not coverage or retained effective aircraft. The recorded
console override replaces the law unclamped and needs no other sortie operands
or content. Ordinary values preserve the game's wrapping/truncation, modifier
feature gates and final clamp; missing operands give null and a named, counted
refusal. Individual zero-plane wings still need their actual efficiency; only
the viewer's weighted hover can skip their zero contribution.

Each mission choice's `icon` is `{key, frame}` for the shared `sprite-artwork`
route: the sprite the regional map's mission entry template's `button` names,
read through the recorded stack's GUI registry (so a mod's GUI or sprite wins),
at zero-based frame `bit` (the game's one-based frame bit + 1). No sprite name is
built from a mission token. A missing template, button or sprite leaves `icon`
null with a named, counted refusal, and the column shows its name only.

Queries check cancellation and the shared complete-reply budget; oversized replies
refuse, never truncate. Cold history returns the same shape with an empty window;
the lead's keyed dispatcher adds coverage/pending. Frontend layout and responsiveness
checks belong to the lead; this crate contains no UI or registration edits.

## Tests

```powershell
$env:CARGO_TARGET_DIR = 'lane-airnavy'
# Set BATTLE_PLAN_GAME_DIR to the read-only supported installation for the content test.
cargo test -p dashboard-air-operations
```
Tests cover native counting parity, overlapping missions/unique wings, zero and signed
wrapping counts, pool ownership, absent/unknown/gaps, saved continuation, daily supplying
reads, read-time statistics bindings through tag/slot reuse and saved continuation,
all raw channels/scales/ring metadata, query selection/prefix/cold state,
5,000-country sparsity, tag reuse, cached reached-wing details, exact efficiency
overrides and viewer parity (including missing fuel and zero-plane wings), counted
naming failures, theatre membership, layered mod region/text replacement, cancellation
and the exported product contracts.

Content tests initialize executable metadata from `BATTLE_PLAN_GAME_DIR`; their authored
files are synthetic and kept outside Git. Every test compiles in every build.

## Lead integration checklist (off-limits here)

- Add the dashboard's dependency/re-export and register both Product types, their
  content-check dispatch and `types()` declarations. Workspace members already use
  `crates/*`; add a workspace dependency alias if the dashboard uses one.
- Add `Keyed<AirMissionsObserver>` and `Keyed<AirStatisticsObserver>` in
  `crates/dashboard/src/prepared/pipeline/independent/keyed.rs`, with their `FAMILY`
  entries, refusal gates and section labels. The generic keyed fold/publish owns
  continuation and all hourly/daily supplying reads.
- Route `air-missions` and `air-statistics` in prepared query dispatch/cache handling
  to these modules' `query(&dashboard_keyed::query::Query)` functions. The lead's
  dispatcher marks coverage/pending. Keep wings as reached-hour transient detail.
- Regenerate the TypeScript contract and wire widgets/settings/picker cards. No UI,
  generated contract, dashboard or dashboard-keyed files were edited here.
- Broader theatre histories need dated choices (recommended) or separate reached-hour
  choice requests. The wings efficiency reply shape is unchanged by the shared law.
