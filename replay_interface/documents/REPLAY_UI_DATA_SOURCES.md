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

The central battery runs `XUI27/external_dependency_ledger.py --check-doc`. It
validates the ledger against current shipping source tokens, requires the
production renderer's `PROMOTED_EXTERNAL_DEPENDENCY_PACKETS` table to be this
ledger row for row and in order, requires each shipping row to bind exactly one
behavior test, and requires the generated human view above to match what the
ledger renders. Its self-tests run in the same gate. Tracked source identity is
the content-addressed `HEAD` Git blob (object id, byte count, and SHA-256), so
the same source stays identical across `core.autocrlf` checkout policies.

It does not certify Extractor implementation and does not claim measured
end-to-end framebuffer equality. Nor does it freeze the Replay source closure or
pin packet manifests by blob: lane KK retired the two locked passes and the
terminal receipt that made those claims on 2026-08-15, because no gate ever ran
their verifier and it had been red on master for 337 commits. What replaced them
is the continuously-run battery, which tests the current tree instead of
attesting to a frozen one.
