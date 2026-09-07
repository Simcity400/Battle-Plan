# Battle Plan / Replay

Battle Plan is being built to record supported Hearts of Iron IV campaigns into portable `.battleplan` files and reconstruct the game map and observer UI interactively. The intended workflow is automatic, passive recording followed by playback using authenticated game, DLC, mod, language, and settings content. Reload branches and unreadable periods remain explicit; unsupported builds and incompatible content receive named refusals.

[PROJECT.md](PROJECT.md) defines requirements and completion criteria; this overview is not a release certification. [AGENTS.md](AGENTS.md) governs agent work.

## Architecture

```text
hoi4.exe -> passive injected collector -> external recorder host
         -> validation, fold, content binding -> .battleplan
         -> authenticated ingest -> WorldTimeline -> map and observer UI
         -> windowed playback, headless export, benchmarks
```

- [extractor/](extractor/): process discovery, build admission, attach/teardown, capture scheduling, coverage, campaign composition, folding, and publication.
- [portable_replay/](portable_replay/): neutral format and schema crates linked by both products; neither product depends on the other's private runtime types or process.
- [replay_interface/](replay_interface/): replay admission, timeline, playback, rendering, interaction, and package verification.

Within the viewer, `replay_core` owns typed state and provenance; `map_viewer` owns map providers and non-UI composition; `game_ui` owns authenticated GUI/GFX, fonts, localisation, and hit geometry; `replay_app` owns application lifecycle, playback controls, country/panel state, and presentation. Windowed, headless, and benchmark output share composition routes.

## Sources of truth

- [Domain registry](extractor/native/domain_registry.json): producer identity, status, and scheduling.
- [UI dependencies](replay_interface/re_evidence/rendering_laws/XUI27/external_producer_dependencies.json): producer/consumer contracts, unavailable behavior, and evidence bindings.
- [Map input ledger](replay_interface/re_evidence/rendering_laws/XMAP00/non_ui_visual_input_ledger_20260730.tsv): non-UI visual inputs and dispositions.
- Portable schemas and tests: grammar, limits, and identity rules.
- Evidence manifests in `extractor/**/re_evidence/`, `extractor/native/runs/`, and `replay_interface/re_evidence/`: build identities and validation receipts.

Fixture or structural evidence proves only the boundary exercised, not real capture completeness or final visual fidelity. Consult current receipts before claiming a capability is complete.

## Distribution

The required distribution is an offline portable Windows ZIP containing the recorder, viewer, collector DLL, instructions, and checksums. See [release requirements](PROJECT.md#release) for packaging restrictions and promotion criteria.
