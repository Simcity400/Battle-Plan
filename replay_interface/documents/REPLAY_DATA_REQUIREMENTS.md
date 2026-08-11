# Replay recording data requirements

This document defines the recording-side rules Replay enforces. It does not
duplicate the live packet ledgers.

## Canonical requirement authorities

- Every externally supplied Replay requirement, exact carrier identity,
  consumer, packet authority, and unavailable behavior:
  `../re_evidence/rendering_laws/XUI27/external_producer_dependencies.json`.
- Mandatory generated human view:
  `EXTRACTOR_INPUT_RESIDUALS.md`.
- Exhaustive non-UI map inputs and current XMAP00 dispositions:
  `../re_evidence/rendering_laws/XMAP00/non_ui_visual_input_ledger_20260730.tsv`.
- Non-UI consumer/effective-VFS audits:
  `../re_evidence/rendering_laws/XMAP00/`.
- Canonical snapshot fields and typed carrier keys: `REPLAY_SCHEMA.md`.
- Exact field schemas and completeness rules: the owning XUI/XMAP packet and
  the producer contract linked by the ledgers above.

These machine-readable authorities are the complete lists. New requirements
must be added there first; prose summaries must not copy their rows or counts.

## Admission law

A usable recording preserves exact campaign identity and canonical time, stable
country/state/province/unit/combat identities, authenticated effective content,
and the typed per-feature carriers required by the selected product route.
Carrier admission binds the executable/evidence/content identities, hour and
sequence, schema version, completeness census, and packet-specific hashes named
by its owner.

For every field, Replay preserves four distinct states:

- domain absent;
- captured but unobtained/null;
- measured empty;
- measured value.

Only the owning contract may establish measured empty. Missing, legacy, partial,
wrong-hour, wrong-content, unresolved, or unauthenticated inputs remain typed
unavailable. Replay never substitutes:

- owner for controller, history/game-start state for current state, or another
  country/perspective;
- zero, empty seas/skies, open ocean, a neutral tint, grey, or an omitted badge;
- a representative province/base, centroid, nearest object, first row, or
  recorder selection;
- selected-Present pixels, recorder UI/POV, hard-coded game data, heap scans,
  visual tuning, or a captured diagnostic pass.

Static content is admitted only from the recording-authenticated effective VFS,
preserving base-game/mod/DLC/language winner order. Static definitions cannot
replace current campaign state. Derived presentation is permitted only where the
owning packet proves the exact engine/game-file function and all of its inputs.

## Map carriers

Map production consumes the typed `WorldFrame` paths in the XMAP00 TSV. This
includes current ownership and control, country presentation identity, selected
POV visibility, diplomacy and map-mode inputs, land/naval/air objects, unit and
order state, rail/supply state, weather and snow/mud state, map furniture,
effects, and packet-authenticated rendering dependencies. XMAP14 combat is a
top-level carrier family; XMAP15 battle plans require their authenticated
semantic carrier. `map.control_by_state` and `map.control_by_province` are
separate authorities and neither may be inferred from the other.

Each applicable XMAP00 row must carry its owner-proven production disposition.
An unavailable row suppresses or rejects only as specified by its packet; it is
never silently rendered as empty. XMAP22 composes admitted providers but does
not weaken their carrier contracts.

## UI carriers

UI production consumes authenticated authored GUI/GFX/font/localization content,
the current typed `WorldFrame`, and viewer-owned interaction state. The XUI27
ledger is the complete external boundary. Country-scoped carriers require exact
selected-country joins and complete rosters where specified. Collection order,
unavailable-versus-empty, localization, catalog identity, and country-switch
invalidation remain packet-local laws; generic rectangles, recorded cursor
state, or recorder widgets are not semantic authority.

## Container and forward compatibility

The container must authenticate the replay schema, source/content identities,
and all referenced carrier artifacts. Unknown domains and fields survive ingest
for forward compatibility, but no consumer may treat an unknown field as an
admitted known carrier. Production compact recordings and snapshot fixtures use
the single portable pre-fold boundary described by `REPLAY_ARCHITECTURE.md` and
`HOI4REPLAY_CONTAINER_SPEC.md`.
