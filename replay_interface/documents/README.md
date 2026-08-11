# Replay Interface documentation

Read `../../documents/DECISIONS.md` for product intent and
`../../documents/STATUS.md` for current project status.

## Active specifications

- `REPLAY_ARCHITECTURE.md`: shipping crates, authority flow, lifecycle, and
  fail-closed boundaries.
- `REPLAY_SCHEMA.md`: recorded schema and typed carrier identities.
- `HOI4REPLAY_CONTAINER_SPEC.md`: container format and authentication.
- `REPLAY_DATA_REQUIREMENTS.md`: inputs Replay requires from a recording.
- `PRODUCTION_PLAYBACK_PROFILE.md`: authenticated content and launch profile.
- `MOD_COMPATIBILITY.md`: base-game/mod/DLC/language winner-order contract.
- `REPLAY_UI_DATA_SOURCES.md`: UI authority boundary.
- `MAP_GRAPHICS_PRODUCT_STATUS.md`: current non-UI map product status.

Rendering laws in this directory describe current production mechanisms only.
Packet-local evidence and contracts live beneath
`../re_evidence/rendering_laws/`. Enumerate that directory to discover packets;
this page intentionally does not maintain a second handwritten packet index.
The XMAP00 and XUI27 machine-readable ledgers are the current coverage and
external-dependency authorities.

Machine-local evidence is indexed by `<local-path>`. Git history,
not dated handoff or recovery documents, preserves development chronology.
