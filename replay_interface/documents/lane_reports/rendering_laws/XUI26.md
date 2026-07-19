# XUI26 — frontend/meta/dev and DLC music

Disposition: **PROVEN / PRODUCTION-PROMOTION-READY / FAIL-CLOSED**.

XUI26 admits all 25 formerly descoped base frontend/meta/dev panel files and all
three DLC music panel families. The frozen corpus contains 28 source families,
232 window definitions, and zero definition rejections. Static chrome is owned
by the effective-VFS/game-file laws already frozen by XUI01, XUI07, UI14, and
UI15; XUI26 adds the runtime ownership seam that those packets intentionally
left gated.

## Production law

- The resolved `.gui` source must match the XUI26 source SHA-256 before its
  frontend family is admitted.
- Dynamic content is admitted only from a slot-specific
  `FrontendBindingPayload` variant whose authority and non-empty source-record
  provenance match the frozen binding slot. Opaque snapshots reject.
- Missing replay, backend, live-session, host-local, or developer data does not
  invent content and does not contact a service. The exact unavailable reason
  is returned while the frozen chrome remains renderable.
- Only nine slots are genuine external-service inputs: account summary,
  multiplayer directory, friends roster, achievement state, backend account,
  browser document, POPS login, subscription state, and matchmaking state.
- The other 17 slots have mandatory recorder/replay/runtime routes: scenario,
  tutorial, rules, player-AI, AI trace, A15 players/hotjoin/chat, settings and
  message snapshots, and replay-UI loader/nudge/music state.
- Music audio playback is outside the rendering input. The three music rows
  promote the exact visible chrome and station-entry definitions.

The implementation is
`replay_interface/native/game_ui/src/frontend_meta.rs`. It covers exact source
identity, all 28 family identifiers, typed authority validation, duplicate and
unknown-slot rejection, provenance/completeness rejection, explicit unavailable
composition, and installed-game parsing/admission of all 232 definitions.

## Evidence and external-input disposition

- `promotion_contract.json` — exact 28-family file/hash/window/widget/sprite
  census and all 26 dynamic binding slots.
- `external_input_contract.json` — corrected v2 source disposition with nine
  genuine service inputs and 17 replay-owned routes; no new capture or human
  input is required.

Ten producer exports were frozen; the exact function/job/artifact/manifest
tuples are recorded below.

## Ghidra provenance

All exports use executable SHA-256
`870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

| Function addresses | Job ID | Artifact path | Manifest SHA-256 |
|---|---|---|---|
| `0x141cce1c0`, `0x141cba2c0`, `0x141cbb130` | `20260717T102740Z-xui26-account-friends-subscription-produ-7d364202` | `C:\Users\Example\ghidra_agent_exports\20260717T102740Z-xui26-account-friends-subscription-produ-7d364202\evidence\_FUNCTION_EVIDENCE_MANIFEST.json` | `b749f9e6bcbc0fff1d38a7da5a7ccc7f5440ac7c18b721dba4804e8ea09a3c10` |
| `0x1416755e0`, `0x1406119f0`, `0x1418845c0` | `20260717T102823Z-xui26-achievement-progress-producers-v1-3ce2ef35` | `C:\Users\Example\ghidra_agent_exports\20260717T102823Z-xui26-achievement-progress-producers-v1-3ce2ef35\evidence\_FUNCTION_EVIDENCE_MANIFEST.json` | `e21b8cc8c024b32e0f7054c86aaa196d59e3398e1cc93d3f591d4df079924131` |
| `0x141861420`, `0x14185abb0`, `0x141860820`, `0x14185cd10` | `20260717T102953Z-xui26-player-lobby-producers-v1-8fdf586b` | `C:\Users\Example\ghidra_agent_exports\20260717T102953Z-xui26-player-lobby-producers-v1-8fdf586b\evidence\_FUNCTION_EVIDENCE_MANIFEST.json` | `09b2a244bfb8dc06138093bac078b5297ae356aa57867d5888274efc8e020afe` |
| `0x14179a5d0`, `0x14179a7f0`, `0x141cd2b90`, `0x142308ad0` | `20260717T103002Z-xui26-hotjoin-matchmaking-producers-v1-11606934` | `C:\Users\Example\ghidra_agent_exports\20260717T103002Z-xui26-hotjoin-matchmaking-producers-v1-11606934\evidence\_FUNCTION_EVIDENCE_MANIFEST.json` | `ce32d5fbe65cbe0007696928f560ff52e6a5c801c78cb1ddace7852a5015f0d4` |
| `0x1402178c0`, `0x1419a7a20`, `0x141faf8e0`, `0x141fb0f60`, `0x141fb1510` | `20260717T103017Z-xui26-settings-saved-rules-producers-v1-e7ef0dec` | `C:\Users\Example\ghidra_agent_exports\20260717T103017Z-xui26-settings-saved-rules-producers-v1-e7ef0dec\evidence\_FUNCTION_EVIDENCE_MANIFEST.json` | `5d6d5c5851c4f0379325b4f6c2e0bdd0af3c739c0374290184332ab2f8daa0e3` |
| `0x1401826e0`, `0x140180c00`, `0x140b60710` | `20260717T103032Z-xui26-loading-progress-producers-v1-9f1240e5` | `C:\Users\Example\ghidra_agent_exports\20260717T103032Z-xui26-loading-progress-producers-v1-9f1240e5\evidence\_FUNCTION_EVIDENCE_MANIFEST.json` | `49c0d4ccab8561b6cdc8b0def21857a1231754a748d1eafc33e9d85a9a1e93b7` |
| `0x140dad320`, `0x140dc3510`, `0x1419b3ab0`, `0x1419b44b0` | `20260717T103104Z-xui26-tutorial-producers-v1-806bd9b6` | `C:\Users\Example\ghidra_agent_exports\20260717T103104Z-xui26-tutorial-producers-v1-806bd9b6\evidence\_FUNCTION_EVIDENCE_MANIFEST.json` | `5f585b6cff9c5373f561ddd86af9999d51c3a5773191783bf71e9ba2fa6b62c7` |
| `0x1417bf3d0`, `0x141f79420`, `0x141f79740`, `0x141de3160` | `20260717T103340Z-xui26-music-player-producers-v1-f3446868` | `C:\Users\Example\ghidra_agent_exports\20260717T103340Z-xui26-music-player-producers-v1-f3446868\evidence\_FUNCTION_EVIDENCE_MANIFEST.json` | `f28aadb3521114908d65f8b346d39cbec4a3332bf5adfe7c2f5bec7977c26396` |
| `0x141b27d60`, `0x141b2f6a0`, `0x140122410` | `20260717T103348Z-xui26-nudge-ai-trace-producers-v1-861c8a42` | `C:\Users\Example\ghidra_agent_exports\20260717T103348Z-xui26-nudge-ai-trace-producers-v1-861c8a42\evidence\_FUNCTION_EVIDENCE_MANIFEST.json` | `65d257143c1db8fd8b10882da721653fbcf95ed78dc14cd609587e762de33c32` |
| `0x141cbdbe0`, `0x141cd1960` | `20260717T103409Z-xui26-frontend-setup-producers-v1-ea05c8c0` | `C:\Users\Example\ghidra_agent_exports\20260717T103409Z-xui26-frontend-setup-producers-v1-ea05c8c0\evidence\_FUNCTION_EVIDENCE_MANIFEST.json` | `ad5df83a98824201a0c5644e96d661b7a23d45fa4beb1d08280985bcbf3e29ce` |

## Verification

`verify_xui26.py` rebuilds the contract in memory, re-hashes every installed
source, cross-checks the 26 active-playset rows against XUI01, checks the two
standalone Sabaton DLC families separately, validates every source route and
slot-exact payload kind,
and runs the focused runtime unit gate, the ignored installed-game 232-window
parser/admission gate, and the full `game_ui` library regression suite. It
writes deterministic verification and artifact-manifest files only on a full
pass. Two consecutive freezes passed 338/338 checks with identical manifest
SHA-256 `9e86df4c4deba619f61cac47b68910081f861126af1d75df472298443e0f2e40`.

## Production effective-VFS re-audit (2026-07-18)

The launcher-equivalent replay VFS now closes the static-file side of this law:
winning `.gui`, `.gfx`, localisation, font and texture payloads are authenticated by
logical path, layer identity/order, source hash, payload hash and byte length. The
production route therefore no longer needs or permits XUI26's frozen vanilla source
allowlist.

That does not supply the runtime selection law. The portable `WorldFrame` currently
carries neither the selected frontend family/window nor any
`FrontendBindingPayload`/slot provenance. Consequently the replay application cannot
choose which effective frontend/meta/dev family is open, or bind its dynamic slots,
without inventing state. `frontend_meta.rs` remains a proven fail-closed packet law
and verifier, but XUI26 is omitted from the production image until those typed replay
carriers exist. This is an exact runtime-data gap, not a VFS, installation-path,
frozen-hash, or capture-tool blocker.
