# characters_leaders_cabinet_live LaneFrame Adapter Plan

Status: lane-local adapter implemented in `src/adapter.rs` and fixture-proven in
`extractor/native/runs/characters_leaders_cabinet_live_20260701_adapter_fixture_v1/`.
Do not apply shared collector edits from this lane packet without the shared
integration owner serializing the hook.

The lane-owned mini stack already exposes:

- `extractor::LaneFrame`
- `adapter::build_lane_frame_from_adapter_input(mem, roots, &AdapterInput)`
- `scheduled::read_native_scheduled_frame(mem, &NativeRootCache, NativeFrameInput)`
- `dll_collector::LaneCollectorModule::collect_frame(game_hour_raw, &LaneFrame)`
- domain-47 compact encoder/decoder gate

The missing production integration is the shared root cache and scheduled
collector call that feeds live roots into lane-local `NativeRootCache` plus
per-frame `NativeFrameInput`. An empty frame or source-unavailable placeholder
is not acceptable.

For shared production, prefer the lane-owned scheduled API over direct adapter
calls. The shared collector should populate `NativeRootCache` outside hook-held
callbacks, build `NativeFrameInput` from current country/politics roots on each
scheduled frame, then call `scheduled::read_native_scheduled_frame`. That
function owns adapter execution, compact payload encoding, Rust gate counters,
and status flag propagation.

## Required Shared Inputs

Use existing shared `CollectorState` values where possible:

- `base` for current module base;
- `date_ptr` and current game hour/date source;
- `country_identity` for live `CCountry*` plus tag database ids;
- scheduled-domain control state and compact health flags.

Add a domain-47 root cache beside the other compact tracker/root caches:

- current-build RTTI/vtable roots for `CCharacter`, `CArmyLeader`,
  `CNavyLeader`, `CCountryLeader`, `CAdvisor`, and `CUnitLeaderTrait`;
- validated warm pins for character, army leader, navy leader,
  country-leader-role, advisor, and unit-leader-trait definition objects;
- static role metadata loaded outside hook callbacks:
  advisor idea token -> slot/bucket/static traits, character token -> portraits
  and role metadata where needed.

Map those shared values into lane-owned adapter structs:

- `ResolvedTypeRoots`: current module base plus RTTI/vtable-derived class roots
  for `CCharacter`, `CArmyLeader`, `CNavyLeader`, `CCountryLeader`,
  `CAdvisor`, and `CUnitLeaderTrait`.
- `AdapterInput.characters`: validated `CCharacter*` warm pins plus optional
  static/display token closure.
- `AdapterInput.unit_leaders`: validated `CArmyLeader*` and `CNavyLeader*`
  warm pins, country tag database id, leader kind, and optional static/display
  token closure.
- `AdapterInput.unit_leader_trait_tokens`: validated `CUnitLeaderTrait*`
  definition roots mapped to trait tokens.
- `AdapterInput.country_leader_roles`: validated `CCountryLeader*` roots,
  country tag database id, and optional token/name closure.
- `AdapterInput.countries`: country bridge rows with `CCountry*`, tag database
  id, ruling-party token/name if available, and country-scoped active politics
  idea tokens for appointed-advisor intersection.
- `AdapterInput.advisors`: validated `CAdvisor*` roots plus static advisor
  idea token, slot, and cabinet ledger/bucket hints from `common/characters`.
- `AdapterInput.portraits`: static character portrait metadata from
  `common/characters`, keyed by `(country_tag_db_id, char_id)`. The adapter
  emits only portraits whose character identity is also known from live rows.

## Adapter Flow

1. Prewarm outside hook-held callbacks.
   - Resolve RTTI/vtables for the current build.
   - Scan candidate pools.
   - Reduce raw aligned hits to validated objects with vtable/type/id guards.
   - Build trait definition token map from `CUnitLeaderTrait`.
   - Build advisor static token metadata from `common/characters`.

2. On each scheduled compact frame, construct `LaneFrame::new(elapsed_wall_ms)`.

3. Populate `characters`.
   - For each validated `CCharacter`, read `+0x0C` char id,
     `+0x68` tag database id, optional display name/token strings, and
     `+0xA8` unit-leader pointer.
   - Fail closed on missing vtable/type/id guards.
   - Do not claim full portrait/name closure until generic character tokens are
     emitted or a static sidecar is registered.

4. Populate `unit_leaders` and `unit_leader_traits`.
   - Validate `CArmyLeader` / `CNavyLeader` vtable and `+0x08 == 4713`.
   - Read id, token, display name, XP raw, overall skill via
     `*(leader+0xE60)+0x1B8`, stat bytes, and character id copy.
   - Read assigned trait membership from `leader+0xDC8/+0xDD0/+0xDD4`.
   - Reject `leader+0x288` for assigned traits.

5. Populate `country_leader_roles` and `active_rulers`.
   - Role row edge: `CCountryLeader+0x08 -> CCharacter`.
   - Active ruler edge:
     `CCountry+0xF90 -> CPoliticalStatus+0xD0 -> CPoliticalParty+0x70
     vector<CCountryLeader*> -> CCountryLeader+0x08 -> CCharacter`.
   - Mark active ruler ambiguous if the ruling-party leader vector size is not
     exactly `1`.

6. Populate `advisor_roster`.
   - Validate `CAdvisor` vtable.
   - Read `CAdvisor+0x10 -> CCharacter`, `+0x30` idea token, and `+0x80` slot.
   - Bucket from exact slot/ledger/idea-token suffix into the existing enum:
     `political_advisor`, `army_chief`, `navy_chief`, `air_chief`,
     `high_command`, `theorist`, or `unknown`.
   - Drop duplicate or identity-less rows rather than guessing.

7. Populate `appointed_advisors`.
   - Reuse the shared politics active-ideas path from `CPoliticalStatus`.
   - Intersect active idea tokens with the validated advisor roster idea tokens.
   - Do not sweep `CAdvisor` for a hired boolean in the hourly hot path.

8. Encode compact data through the lane-owned scheduled API:

   ```rust
   scheduled::read_native_scheduled_frame(&mem, &cache, frame_input)
   ```

   For isolated lane fixtures, `LaneCollectorModule::collect_frame` can still
   wrap a prebuilt `LaneFrame`:

   ```rust
   LaneCollectorModule::collect_frame(game_hour_raw, &frame)
   ```

9. For shared production capture, append the payload through the shared
   coordinator's `append_compact_record` helper and add a matching
   `DOMAIN_PERF` record in deterministic sequence after domain `46`.

The adapter fixture proves this shape with one row in every row family:
character, unit leader, unit-leader trait, country-leader role, active ruler,
advisor roster, appointed advisor, and character portrait. It is not shared DLL
capture; it validates that the remaining shared work is input translation and
scheduling rather than lane-local frame construction.

## Failure Semantics

Set compact health flags instead of emitting guessed data:

- `STATUS_BRIDGE_UNAVAILABLE` if required country/politics bridge roots are
  unavailable;
- `STATUS_SOURCE_UNAVAILABLE` if character/advisor/leader roots cannot be
  validated;
- `STATUS_PARTIAL_DATA` if any required row class is omitted after root
  validation begins;
- `STATUS_WORKER_PANIC` only for actual worker failure.

Production acceptance requires all of those flags to remain clear for the live
proof run.

## Registration Patch Boundary

The shared patch may add:

- collector/recorder Cargo dependency on this lane crate;
- domain-47 active/schedulable registry descriptors;
- scheduled-domain token parsing;
- a tracker/root cache field;
- one compact worker call that produces `LaneFrame`;
- one recorder decoder adapter that calls the lane gate.

The shared patch must not inline domain-47 row parsing into
`collector/src/lib.rs` or duplicate the injected DLL collector/coordinator.

## Proof Gate After Integration

Run the shared live proof with only this scheduled domain enabled:

```powershell
cargo build --release --manifest-path extractor\native\Cargo.toml -p collector -p recorder_host

extractor\native\recorder_host\target\release\recorder_host.exe `
  --stream-format compact `
  --extraction-perf-gate `
  --scheduled-domain characters_leaders_cabinet_live `
  --duration-s 60 `
  --out-dir extractor\native\runs\characters_leaders_cabinet_live_live_domain47_v1
```

Accept only if:

- domain `47` has data every scheduled game hour;
- Rust decode has zero parse errors;
- `.battleplan` verifies;
- speed-five/current-equivalent cadence is hourly-or-better;
- zero game hours are skipped;
- no worker/source/partial/bridge failures are present.
