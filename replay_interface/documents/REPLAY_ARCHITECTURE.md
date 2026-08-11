# Replay architecture

`documents/DECISIONS.md` defines product intent. This document describes the
shipping Replay architecture.

```text
.battleplan
    |
    v
replay_core -- authenticated ingest, typed WorldTimeline, state_at(hour)
    |
    +--> ProductMapCompositionSession -- shared XMAP22 non-UI map graph
    |
    +--> ProductObserverUiRuntime -- XUI27 authored observer UI graph
    v
replay_app -- window, headless dump, benchmark, transport, Present authority
```

## Recording boundary

The portable `.battleplan` file is the only installed-runtime boundary between
Extractor and Replay. `replay_core::source::ReplaySource` admits recording bytes
after container identity and integrity checks. Snapshot recordings use the
schema in `REPLAY_SCHEMA.md`; production compact streams use the certified
portable pre-fold contract and the single production `WorldFold` implementation.

The production application retains and authenticates the recorder sibling,
compact source, request, result, and private spool identities. It does not use
PATH, registry, install discovery, an alternate decoder, or an ordinary/raw
retry. `replay_core` lazily rehashes and parses one certified hour at a time.
The portable contracts live in `portable_replay/native`; no installed Replay
component links Extractor-private types.

Absent, explicit null, measured empty, and measured values remain distinct.
Unknown fields remain reachable. Invalid identity, completeness, content, hour,
or sequence makes the owning typed feature unavailable.

## Native components

- `replay_core`: container/stream ingest, timeline, typed views, provenance,
  coverage, and certified spool lifetime.
- `replay_app`: product lifecycle, window/swapchain, headless dump, benchmark,
  viewer state, map/UI composition, and presented-generation input authority.
- `map_viewer`: packet-native map providers, shared XMAP22 composition, bundle
  tooling, reference rendering, and map authority checks.
- `game_ui`: authenticated GUI/GFX/font/localization admission, authored manager
  transactions, observer models, composition, and hit geometry.
- `terrain_capture`: diagnostic capture tooling; never a playback prerequisite
  or product authority.
- `replay_fixture_gen`, `replay_qa`, and `package_runtime`: deterministic
  fixtures, parity QA, and immutable package activation.

## Shipping map route

Windowed playback, `--dump-frame`, and the recording benchmark all construct a
`ProductMapCompositionSession` and execute the same XMAP22 graph on the caller's
target. Foundation/content admission completes before product device or UI setup.
Each family uses its packet-native provider and receipt; composition order is
`XMAP22/COMPOSITION_SEQUENCE_CONTRACT.md`.

Missing or invalid required state fails the applicable family or composition.
Static effective-VFS data, typed recording carriers, engine-derived laws, and
packet receipts are not interchangeable. Diagnostic captured-Present or
selected-Present data is never a substitute for a required semantic carrier.
GPU completion/readback authenticates the final Replay-owned target. Window
Present separately decides whether that generation may publish input state.

The XMAP00 TSV and audit are the exhaustive non-UI input/disposition authority;
`MAP_GRAPHICS_PRODUCT_STATUS.md` is the concise current status view. Individual
laws own their production mechanisms and packet evidence only.

## Shipping UI route

`ProductObserverUiRuntime::execute` consumes the current typed `WorldFrame`, the
authenticated effective VFS, and viewer-owned transport/country/mapmode/panel
state. It builds a private authored manager transaction, composes the framebuffer,
and publishes hit authority only after `Presented`. Seek, rollback,
discontinuity, occlusion, and failed Present cannot advance undisplayed input
state. The complete external boundary is generated from the XUI27 dependency
ledger; no handwritten status table duplicates it.

## Runtime exclusions

Replay rendering does not read live game memory, recorder UI ownership, recorder
POV, an unverified installed game tree, or generated inspection caches.
Machine-local evidence, capture corpora, Python asset tools, Ghidra, and
`hoi4_re` support implementation and verification but are not runtime inputs.
