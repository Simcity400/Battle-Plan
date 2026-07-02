# Session Multiplayer Metadata Live Lane

This lane keeps active extractor, decoder, gate, and fixture files here. Historical
registration plans and proof artifacts were moved to
`../../../documents/archive/native_lanes/session_multiplayer_metadata_live/`.

Current extraction completion status lives in
`../../../documents/MASTER_EXTRACTION_CONTRACT.md`.

## Live-root validation (2026-07-02, read-only)

`live_probe_session.py` is a lane-local, READ-ONLY passive probe
(ReadProcessMemory only; no writes/injection/debugger). It validates the
domain-57 live roots against a running, paused `hoi4.exe`.

Base resolution ground truth is the shared per-build profile
(`extractor/native/compact_contract/src/build_profile.rs`,
`COLLECTOR_PROFILE_ID = vanilla_latest_2026_06_30`). Both AOB sentinels
(daily-update prologue @ `module+0x1D4390`, long-string accessor @
`module+0x526A40`) were confirmed live. NOTE: the running exe sha256 was
`870ebf1f...`, which differs from the `memory_reader/layout.16022851` gate; the
build_profile RVAs are stable across those two builds and are authoritative.

`CGameState` singleton: `u64[module + 0x3316E90]`.

Strict rerun gate: pass explicit oracles such as `--expect-tag IRE`,
`--expect-name Ireland`, `--expect-index 29`, and `--expect-paused 1`. Without
`--expect-tag`, the player-country oracle is INCONCLUSIVE and exits nonzero
unless `--allow-unverified-oracle` is used for exploratory runs. Exit code 0
means every required executed check passed; any
FAIL, target exception, skipped dependent read, or unwaived inconclusive writes
`"passed": false` to JSON and exits nonzero.

### TARGET 1 -- player-country mapping: VALIDATED

Chain (all offsets off the `CGameState` object):
- `+0x394` i32 `country_count` (observed 440).
- `+0x388` u64 -> `byte[count]` human-player status array. `A[i] != 0` marks a
  human-controlled country slot `i`. (Derived from the `print_controllers`
  console handler -> `CGameState::get_human_controlled_countries_string`
  @ `0x1401e44e0`, which loops `i in 0..[+0x394]` reading `[+0x388][i]`.)
- `+0x310` u64 -> `qword[count]` country handles. `handle+0x08` = i32 slot index;
  `handle+0x10` = localized country name (MSVC SSO std::string);
  `handle+0x38` = adjective.
- `+0x358` u64 -> country-slot records, stride `0x20`; `rec+0x00` = 3-char TAG
  (SSO). Slot 0 = "---" sentinel, slot 1=GER, 2=ENG, ... Resolve a human's TAG
  via `slot_base + (handle+0x08 index) * 0x20`. (Derived from the handle->record
  resolver @ `0x140ba5c20`: `rec = [CGameState+0x358] + i32[handle]*0x20`.)

Oracle + observation: single-player session -> exactly ONE nonzero entry in the
`+0x388` array, at `gs_index=29`, `slot_index=29`, TAG `IRE`, name `Ireland`.
Stable across 2 reads. This matches the SP expectation of one human player and
cross-checks against the independent `country_index_bridge` (tag_db_id 29 -> IRE
-> "Ireland").

SP vs MP: the `+0x388` status byte per country is the human/AI discriminator.
In MP each additional human country would appear as another nonzero entry; the
byte value itself is the printed status code. The player NAME / player_key /
session_slot / host / connected flags in the domain-57 schema are NOT sourced
here (see TARGET 3).

### TARGET 2 -- pause state: VALIDATED (flip unverified); game speed: OPEN

Clock/pause singleton: two adjacent static slots `module+0x33172E0` and
`module+0x33172E8` both point at the SAME object (vtable rva `0x2952510`).

- `is_paused` bool at `clock + 0x6C1`. The engine's own `gamespeed 0` / pause
  banner path reads exactly this byte via `vtable+0x2E8`
  (getter `0x140dc3db0`: `movzx eax, byte [rcx+0x6C1]; ret`).

Oracle: the game IS currently paused -> `clock+0x6C1` reads `1`, stable across 3
reads. The flip to `0` (unpause) is NOT verified because the running game must
not be disturbed (read-only, no unpause). Confidence: high (static getter + live
paused oracle).

Game SPEED (0..4) is OPEN: the `gamespeed` console command sets speed as
`arg-1` (0..4) via `set_speed` (`0x140dcb950`) -> a sub-object obtained from a
getter call-chain (`0x1401ff6f0`), stored by `0x140202250`. It is NOT a fixed
offset off a static pointer, so no clean live speed root is pinned yet. Needs a
speed-getter/chain resolution pass.

### TARGET 3 -- host/lobby/player metadata: SP-LIMITED (fail closed)

The MP lobby machinery exists in the binary (`CGameLobby`, `CPlayerLobby`,
`player_name=`, `playerlobby_playerlist`) but is NOT populated in a single-player
session. From SP only the local human country is observable (TARGET 1). The
domain-57 MP-only fields -- `player_name`, `player_key_hash`, `session_slot`,
`PLAYER_FLAG_HOST`, `PLAYER_FLAG_CONNECTED`, and the session-event hooks
(hotjoin/chat/tag-switch) -- cannot be validated from an SP fixture and fail
closed. Validating them requires a live MP session capture.
