# Domain 57 Integration Packet

Lane: `session_multiplayer_metadata_live`
Domain id: `57`
Replay-layer request: `R4`
Target integration status: `ValidatedPartialSlice`, schedulable

This packet pre-authors the shared integration for the validated partial slice
only:

- Include: human player to country TAG rows from `CGameState` and per-hour pause
  state from the clock singleton.
- Exclude: game speed, player lobby/persona fields, player key, MP session slot,
  host/connected flags, hotjoin/chat/tag-switch/session-event hooks.

The live-root proof is lane-local: `live_probe_session.py` and
`live_probe_session_strict.json`, validated read-only on 2026-07-02.

## Packet File List

- `INTEGRATION_PACKET.md`: this shared integration packet.
- `fold_r2_projection.rs`: standalone draft replay fold module and tests.
- Existing evidence kept as-is: `README.md`, `live_probe_session.py`,
  `live_probe_session_strict.json`, `src/lib.rs`, `src/bin/proof.rs`.

No shared files are edited by this packet.

## Schema Home Decision

Do not write domain 57 into `provenance.mod`: schema section 2.1 is active
launcher/mod identity and rejects personal paths. Domain 57 is live session
state, not mod identity.

Do not write `countries[*].human_name`: schema section 3.13 defines
`human_name` as the MP lobby/persona display name from `players.json`.
The validated root only proves that a country slot is human-controlled and
resolves the TAG. It does not source the persona name, Steam id, player key, or
host connection state.

Do not claim `world.scenario.speed` or `world.scenario.session` as live capture
fields: schema section 18 describes save/world scenario metadata, and speed is
explicitly unpinned for this lane.

Honest home for R4 is an R2-projected domain sidecar until the replay schema adds
a native session/player block:

```jsonc
"_r2_session_multiplayer_metadata_live": {
  "players_by_country": {
    "state": "known|unavailable",
    "rows": [
      {
        "country_tag": "IRE",
        "tag_db_id": 29,
        "is_human_controlled": true,
        "human_status_raw": 1,
        "human_name": { "state": "not_sourced" },
        "player_key_hash": { "state": "not_sourced" },
        "session_slot": { "state": "not_sourced" },
        "host": { "state": "not_sourced" },
        "connected": { "state": "not_sourced" },
        "valid_from_hour_raw": 60828184
      }
    ]
  },
  "pause": {
    "state": "known|unavailable",
    "is_paused": true
  }
}
```

Pause is a world-scoped per-hour state. It should be folded as state for the
snapshot hour, not as a fabricated user command event. A future schema pass can
move this sidecar into `world.session` or a top-level `session` block; until then
the R2 sidecar keeps provenance explicit.

Three-state semantics:

- `known`: value was read from a validated root and passed sentinels/profile
  checks.
- `unavailable`: root/profile/sentinel/read validation failed, or a validated
  player-country scan produced zero human rows.
- `not_sourced`: field exists in the compact or target schema family but is not
  validated for the partial slice.

## Shared Build Profile Additions

Target file when shared edits are allowed:
`extractor/native/compact_contract/src/build_profile.rs`.

Add these constants near the other domain-specific profile constants. The
`CGameState` static slot is already present as `CG_STATE_DATE_OFFSET`; keep a
domain-57 semantic alias so lane code imports one named profile surface.

```rust
/// Domain 57 CGameState singleton slot, validated by the 2026-07-02
/// `session_multiplayer_metadata_live/live_probe_session.py` passive probe.
/// Oracles: shared daily-update and long-string sentinels passed; strict SP
/// probe observed exactly one human row, TAG IRE, name Ireland, gs/slot index 29.
pub const DOMAIN57_CGAMESTATE_SINGLETON_RVA: usize = CG_STATE_DATE_OFFSET;

/// Domain 57 CGameState country count. The 2026-07-02 probe observed 440 and
/// validated the loop shape from `get_human_controlled_countries_string`.
pub const DOMAIN57_CGAMESTATE_COUNTRY_COUNT: usize = 0x394;

/// Domain 57 CGameState human-control byte array pointer. A nonzero byte marks
/// the country slot as human-controlled; SP oracle was index 29 -> IRE.
pub const DOMAIN57_CGAMESTATE_HUMAN_STATUS_ARRAY: usize = 0x388;

/// Domain 57 CGameState country handle array pointer. Each qword handle uses
/// `handle+0x08` as the country slot index and `handle+0x10` as the localized
/// country name string used only as validation evidence.
pub const DOMAIN57_CGAMESTATE_COUNTRY_HANDLE_ARRAY: usize = 0x310;

/// Domain 57 CGameState TAG record array pointer. TAG records are indexed by
/// `handle+0x08` and use stride 0x20 with the 3-byte TAG at record+0x00.
pub const DOMAIN57_CGAMESTATE_SLOT_TAG_ARRAY: usize = 0x358;
pub const DOMAIN57_COUNTRY_HANDLE_SLOT_INDEX: usize = 0x08;
pub const DOMAIN57_COUNTRY_HANDLE_LOCALIZED_NAME: usize = 0x10;
pub const DOMAIN57_COUNTRY_TAG_RECORD_STRIDE: usize = 0x20;
pub const DOMAIN57_COUNTRY_TAG_RECORD_TAG: usize = 0x00;

/// Domain 57 clock singleton slots, validated by the 2026-07-02 passive probe.
/// Oracle: both static slots pointed to the same object and vtable matched
/// `module+0x2952510`; paused read was [1, 1, 1] at `clock+0x6C1`.
pub const DOMAIN57_CLOCK_SINGLETON_SLOT_A_RVA: usize = 0x33172E0;
pub const DOMAIN57_CLOCK_SINGLETON_SLOT_B_RVA: usize = 0x33172E8;
pub const DOMAIN57_CLOCK_VTABLE_RVA: usize = 0x2952510;
pub const DOMAIN57_CLOCK_IS_PAUSED: usize = 0x6C1;
```

Game speed gets no profile constant in this packet. The probe found the setter
chain, but no fixed read root is pinned.

## Collector Wiring Plan

The shared collector should implement one memory reader for this partial slice.
It must import all profile offsets from `compact_contract::build_profile` and
must not keep lane-local copies.

Preflight gates before any dereference:

1. Resolve `module_base`.
2. Verify `DAILY_UPDATE_SENTINEL` at
   `module_base + C_GAME_STATE_DAILY_UPDATE_OFFSET`.
3. Verify `GET_LONG_STRING_SENTINEL` at
   `module_base + GET_LONG_STRING_OFFSET`.
4. Read `cgame_state = u64[module_base + DOMAIN57_CGAMESTATE_SINGLETON_RVA]`.
5. Reject if the pointer is null, unreadable, or outside the collector's
   conservative process address guard.

Player-country chain, exactly as validated:

1. Read `count = i32[cgame_state + 0x394]`; require `1..=4095`.
2. Read roots:
   `status_ptr = u64[cgame_state + 0x388]`,
   `handle_ptr = u64[cgame_state + 0x310]`,
   `slot_base = u64[cgame_state + 0x358]`.
3. Reject the player-country source if any root is null or unreadable.
4. Iterate `i in 0..count`; read `status = u8[status_ptr + i]`.
5. Skip `status == 0` as AI/non-human.
6. For nonzero status, read `handle = u64[handle_ptr + i*8]`; skip row on null
   or unreadable handle.
7. Read `slot_index = i32[handle + 0x08]`; skip row if outside `0..count`.
8. Read `tag = char[3]` from `slot_base + slot_index*0x20`; skip row if invalid
   (`---`, non-ASCII, not exactly 3 upper-case letters).
9. Emit one `PlayerCountryRow`:
   - `country_tag = tag`
   - `tag_db_id = slot_index as u32`
   - `flags = PLAYER_FLAG_HUMAN`
   - `valid_from_hour_raw = frame hour`
   - `player_name = ""`
   - `player_key_hash = 0`
   - `session_slot = u32::MAX`
   - no `PLAYER_FLAG_HOST`, no `PLAYER_FLAG_CONNECTED`

Per-row skip counters to expose in diagnostics:

- `country_slots_scanned`
- `ai_status_zero_rows`
- `human_status_nonzero_rows`
- `handle_null_or_unreadable_rows`
- `slot_index_oob_rows`
- `tag_unreadable_rows`
- `tag_invalid_rows`
- `player_country_rows_emitted`

Empty vs unavailable:

- Single-player should produce exactly one human row. The strict proof observed
  IRE/Ireland at index 29.
- Multiplayer should produce N human rows using the same shape.
- Zero human rows after the roots and count are valid is implausible, not an
  empty result. Badge the source as unavailable and emit no player rows.

Pause chain, exactly as validated:

1. Read `clock_a = u64[module_base + 0x33172E0]`.
2. Read `clock_b = u64[module_base + 0x33172E8]`.
3. Require both nonzero and equal.
4. Read `vtable = u64[clock_a]`; require
   `vtable == module_base + 0x2952510`.
5. Read `is_paused = u8[clock_a + 0x6C1]`; require `0` or `1`.
6. Emit a pause state row for this hour with `value_i32 = is_paused`.

The vtable identity check is mandatory. If it fails, pause is unavailable even
if `clock+0x6C1` reads a plausible byte.

## Recorder Decode And Fold Plan

Decode:

- Reuse the lane payload gate for `PlayerCountryRow` and `SessionEventKind::Pause`.
- Accept only `Pause` event rows with `value_i32` 0 or 1.
- Reject or ignore `GameSpeed`, `Chat`, `SystemMessage`, `Alert`,
  `Notification`, `Hotjoin`, and `TagSwitch` for this integration pass unless
  their source flags are separately validated later.
- Treat `player_name`, `player_key_hash`, `session_slot`, `HOST`, and
  `CONNECTED` as `not_sourced` even if fixture payloads contain values.

Fold:

- Build `_r2_session_multiplayer_metadata_live.players_by_country` from
  player-country rows when `SOURCE_FLAG_SESSION_ROOT_VALIDATED` and the
  player-country root evidence bit are set.
- Do not write `countries[*].human_name`.
- Do not infer AI countries from the absence of a player row.
- Fold pause into `_r2_session_multiplayer_metadata_live.pause` as the per-hour
  world state for the selected snapshot hour.
- If player roots pass but emit zero human rows, set
  `players_by_country.state = "unavailable"`.
- If pause clock roots fail or the vtable check fails, set
  `pause.state = "unavailable"`.

The draft pure fold module is `fold_r2_projection.rs`; run it with:

```powershell
rustc --edition 2021 --test extractor/native/lanes/session_multiplayer_metadata_live/fold_r2_projection.rs -o $env:TEMP\domain57_fold_r2_tests.exe
& $env:TEMP\domain57_fold_r2_tests.exe
```

## Descriptor And Registry Drafts

Target file when shared edits are allowed:
`extractor/native/compact_contract/src/lib.rs`.

Current descriptor:

```rust
domain(
    57,
    "session_multiplayer_metadata_live",
    "session_multiplayer_metadata_live",
    CompactDomainStatus::Reserved,
    false,
    true,
    &[],
),
```

Replace with:

```rust
domain(
    57,
    "session_multiplayer_metadata_live",
    "session_multiplayer_metadata_live",
    CompactDomainStatus::ValidatedPartialSlice,
    true,
    true,
    &[],
),
```

Test expectations to update:

- In `extractor/native/collector/src/domain_registry.rs`, remove `57` from the
  `reserved_lane_ids_are_assigned_but_not_scheduled` loop and assert
  `registry.id_for_scheduled_name("session_multiplayer_metadata_live") == Some(57)`.
- In `extractor/native/recorder_host/src/compact_domain_registry.rs`, assert
  `id_for_scheduled_name("session_multiplayer_metadata_live") == Some(57)` and
  `requires_data_record(57)`.

Derived mirror row for `extractor/native/domain_registry.json`:

```json
{ "id": 57, "name": "session_multiplayer_metadata_live", "status": "validated_partial_slice", "schedulable": true, "requires_data_record": true }
```

## Contract And Registry Row Drafts

Target file when shared edits are allowed:
`extractor/documents/MASTER_EXTRACTION_CONTRACT.md`.

Draft domain row:

```markdown
| `session_multiplayer_metadata_live` / 57 | `VALIDATED_PARTIAL_SLICE_R4` | Shared integration schedules domain 57 for the validated partial slice only: player-country rows from `CGameState=u64[module+0x3316E90]` via `+0x394` count / `+0x388` human byte array / `+0x310` handles (`handle+0x08` slot index) / `+0x358` TAG records stride `0x20`, plus pause state from the clock singleton slots `module+0x33172E0` and `module+0x33172E8`, vtable `module+0x2952510`, bool `+0x6C1`. Live proof: 2026-07-02 read-only strict probe, SP oracle exactly one human row IRE/Ireland index 29, pause byte `[1,1,1]`. Fail-closed: game speed root remains unpinned; player_name/player_key/session_slot/host/connected and session-event hooks remain MP-fixture blocked and are not emitted. |
```

Target file when shared edits are allowed:
`extractor/documents/DOMAIN_REGISTRY.md`.

Draft registry row:

```markdown
| 57 | `session_multiplayer_metadata_live` | validated partial slice (R4): schedulable player-country human TAG rows plus pause state; game speed and MP-only player/session-event fields remain fail-closed | session/multiplayer metadata |
```

Update focused capability rows:

```markdown
| Player-country mapping | Which human controlled which country at each time. | `VALIDATED_PARTIAL_SLICE_R4` | Domain 57 reads the validated CGameState human-status array and TAG table. SP strict oracle proved exactly one human row, IRE/Ireland index 29; MP uses the same N-row shape. | Shared speed-five proof and MP fixture expansion for lobby-only fields remain open. |
| Game speed/pause commands | Speed changes and pauses for playback timing. | `PARTIAL:PAUSE_VALIDATED_SPEED_OPEN` | Domain 57 reads pause as per-hour world state from the validated clock singleton. Game speed remains unpinned and no command hook is claimed. | Pin a real speed read root or command hook before emitting speed. |
```

## Pre-Proof Gate

Strict passive-probe rerun before shared promotion:

```powershell
Set-Location 'extractor/native/lanes/session_multiplayer_metadata_live'
python .\live_probe_session.py --expect-tag IRE --expect-name Ireland --expect-index 29 --expect-paused 1 --json .\live_probe_session_strict.json
```

Acceptance:

- Process exit code is 0.
- JSON has `"passed": true`.
- Both build-profile sentinels pass.
- `CGameState` resolves from `module+0x3316E90`.
- Player-country rows are stable across two reads.
- Strict oracle observes IRE/Ireland at index 29.
- Clock slots agree, vtable equals `module+0x2952510`, and pause reads match
  `--expect-paused`.

## Riskiest Assumptions

- The SP proof establishes the player-country array shape and one-row oracle,
  but not multiple simultaneous human players. The MP assumption is limited to
  "same array, more nonzero bytes"; lobby-only metadata stays blocked.
- Pause flip from 1 to 0 was not observed because the probe was read-only. The
  confidence comes from the engine getter and paused oracle, not a live toggle.
- `tag_db_id = slot_index` relies on the country index bridge remaining aligned
  with the TAG table; the collector should keep bridge diagnostics and reject
  invalid TAGs.
- The R2 sidecar schema is a staging home. A future schema update should replace
  it with a first-class session/player block.
