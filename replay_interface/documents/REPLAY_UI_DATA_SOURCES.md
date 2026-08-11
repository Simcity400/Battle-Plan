# Replay UI data sources

The Replay UI consumes typed replay state plus playback-admitted game content.
It never reads live game memory, recorder UI state, recorder POV, or an
unverified installed game tree while rendering playback.

## Owned by Replay

- Carrier schemas and exact identity checks in `replay_core` and `game_ui`.
- Missing-versus-measured-empty admission and country/context joins.
- Effective-VFS, GUI, GFX, localization, font, shader and texture admission.
- Authored layout, composition, clipping, paint order and hit order.
- Viewer-owned transport, country, mapmode and panel state.
- `Presented`-only input authority, plus seek/discontinuity/rollback invalidation.
- Windowed, headless and benchmark product routes.

## Supplied externally

The sole authoritative list is the machine-readable ledger:

`replay_interface/re_evidence/rendering_laws/XUI27/external_producer_dependencies.json`

Its generated human view is:

`replay_interface/documents/EXTRACTOR_INPUT_RESIDUALS.md`

Each row names exact carrier identities, requirements, current source bindings,
behavior tests and packet authorities. A non-shipping row is never represented
as a compositor, runtime, or current product-pixel route. This file deliberately
does not copy rows or counts because a second hand-maintained view would drift.

External producer population is not certified by Replay. Absence of a required
carrier makes the owning feature unavailable; it cannot be treated as a zero
measurement or substituted from another country, selected UI state, display
text, hard-coded game data or a visual approximation.

## Content source

Static UI/content bytes come only from the recording's authenticated effective
content manifest and effective VFS, preserving base-game/mod/DLC/language
winner order. The production renderer uses authored assets and game-file values;
it does not visually tune replacement constants.

## Certification

XUI27 mechanically validates the ledger against current shipping source tokens,
the renderer's external-dependency table, current packet manifests, curated
cross-runtime behavior tests and zero Replay omission tables. It certifies
Replay behavior and source identity, not Extractor implementation and not
measured end-to-end framebuffer equality. Tracked source identity is the
content-addressed `HEAD` Git blob (object id, byte count, and SHA-256), so the
same source remains identical across `core.autocrlf` checkout policies. Both
locked passes still require a clean attached worktree.
