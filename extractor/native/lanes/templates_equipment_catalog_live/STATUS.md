# templates_equipment_catalog_live status

Completion status: `NOT COMPLETE`

Backlog reduced: yes.

Exact master checklist row narrowed:

- `Unit templates and equipment definitions`: narrowed to a production-shaped deterministic static artifact contract for scripted unit/equipment definition files.
- `Division template catalog`: narrowed to scripted starting-template catalog extraction and replay-container verification. Live-created or edited templates and division-template-id joins remain open.

Files changed:

- `extractor/native/lanes/templates_equipment_catalog_live/templates_equipment_catalog_gate.py`
- `extractor/native/lanes/templates_equipment_catalog_live/test_templates_equipment_catalog_gate.py`
- `extractor/native/lanes/templates_equipment_catalog_live/REGISTRATION_PATCH_PLAN.md`
- `extractor/native/lanes/templates_equipment_catalog_live/STATUS.md`
- `extractor/native/lanes/templates_equipment_catalog_live/proofs/strict/domain59_static_gate_report.json`
- `extractor/native/lanes/templates_equipment_catalog_live/proofs/strict/templates_equipment_catalog_live_domain59.battleplan`
- `extractor/native/lanes/templates_equipment_catalog_live/proofs/strict/templates_equipment_catalog_live_domain59/index.json`
- `extractor/native/lanes/templates_equipment_catalog_live/proofs/strict/templates_equipment_catalog_live_domain59/snapshot_000000.json`
- `extractor/native/lanes/templates_equipment_catalog_live/proofs/strict/templates_equipment_catalog_live_domain59/domain59_static_catalog.json`
- `extractor/native/lanes/templates_equipment_catalog_live/proofs/explicit/domain59_static_gate_report.json`
- `extractor/native/lanes/templates_equipment_catalog_live/proofs/explicit/templates_equipment_catalog_live_domain59.battleplan`
- `extractor/native/lanes/templates_equipment_catalog_live/proofs/explicit/templates_equipment_catalog_live_domain59/index.json`
- `extractor/native/lanes/templates_equipment_catalog_live/proofs/explicit/templates_equipment_catalog_live_domain59/snapshot_000000.json`
- `extractor/native/lanes/templates_equipment_catalog_live/proofs/explicit/templates_equipment_catalog_live_domain59/domain59_static_catalog.json`

Roots/offsets/functions validated or rejected:

- Validated vanilla root discovery from the running `hoi4.exe` process path: `<local-path> of Iron IV\hoi4.exe`.
- Corroborated the same root from Steam metadata: `C:\Program Files (x86)\Steam\steamapps\libraryfolders.vdf` contains app `394360` under `F:\SteamLibrary`, and `<local-path>` has `installdir = Hearts of Iron IV`.
- Validated explicit configurable root contract: `--vanilla-root <local-path> of Iron IV`.
- Validated static file resolution path: `game_assets.extract.build_file_map_layers` plus `game_assets.paradox_parser.parse`.
- Validated default mod descriptor path: sibling `test.mod` can provide `replace_path` semantics when present.
- No live `hoi4.exe` roots, offsets, or functions were validated.
- No live candidate was rejected.

Compact schema or static artifact changes:

- Added lane-local static artifact schema `templates_equipment_catalog_live.static_artifact.v1`.
- Added deterministic source-file SHA-256 inventory for `common/units`, `common/units/equipment`, and `history/units`.
- Added replay snapshot schemas `templates_equipment_catalog_live.static_index.v1` and `templates_equipment_catalog_live.static_snapshot.v1`.
- Added bounded vanilla-root discovery to the lane gate. Discovery evidence is kept in the proof report, not the replay payload, so explicit and auto-discovered roots produce the same static artifact SHA-256.

Rust decoder/gate changes:

- No shared Rust files changed.
- Lane gate uses the existing `snapshot` crate `pack_replay` binary for `.battleplan` pack/verify.

DLL-only: no. Static-artifact path explanation: this slice is deterministic replay enrichment from the resolved vanilla/mod file set. It is valid only for static definitions and scripted starting templates, not live template edits.

Compact stream: no.

Rust decoded: no shared domain-59 decoder yet; `.battleplan` container verification uses the existing Rust snapshot verifier.

`.battleplan` verified: yes for the strict production static artifact path and yes for the explicit-root contract.

Proof commands/results:

- `python -m pytest extractor/native/lanes/templates_equipment_catalog_live/test_templates_equipment_catalog_gate.py -q`
  - Result: pass, `4 passed in 0.06s`.
- `python extractor/native/lanes/templates_equipment_catalog_live/templates_equipment_catalog_gate.py --out-dir extractor/native/lanes/templates_equipment_catalog_live/proofs/strict`
  - Result: pass.
  - Vanilla root selected from running process evidence: `<local-path> of Iron IV`.
  - Counts: `division_template_files=315`, `division_templates=306`, `equipment_definition_files=51`, `equipment_definitions=59`, `unit_definition_files=247`, `unit_definitions=1180`.
  - Artifact SHA-256: `6427e4b62686447be5dedde7e27c28dd84f8f1d597f366703356d28ace4a4ae4`.
  - Replay verifier result: `verify: OK`, `byte-identity OK for all 2 snapshots`.
- `python extractor/native/lanes/templates_equipment_catalog_live/templates_equipment_catalog_gate.py --vanilla-root "<local-path> of Iron IV" --out-dir extractor/native/lanes/templates_equipment_catalog_live/proofs/explicit`
  - Result: pass.
  - Counts match the auto-discovered strict run.
  - Artifact SHA-256 matches the auto-discovered strict run: `6427e4b62686447be5dedde7e27c28dd84f8f1d597f366703356d28ace4a4ae4`.
  - Replay verifier result: `verify: OK`, `byte-identity OK for all 2 snapshots`.

Speed-five/current-equivalent hourly-or-lower: not applicable to this static-artifact slice.

Zero skipped hours: not applicable to this static-artifact slice.

Any partial/source/worker/bridge failures: no live worker/bridge involved. Static artifact proof is no longer fixture-blocked.

Proposed `MASTER_EXTRACTION_CONTRACT.md` row changes for orchestrator review:

- `Unit templates and equipment definitions` proposed status: change from `ROOT_DISCOVERY_OPEN` to `STATIC_ARTIFACT_PROVED_LIVE_JOINS_OPEN` if the orchestrator accepts a static-artifact intermediate status; otherwise keep `ROOT_DISCOVERY_OPEN`. Proposed evidence/domain text: `domain 59 lane-local deterministic static artifact gate; strict vanilla+mod proof selected <local-path> of Iron IV from running process evidence, emitted 1180 unit definitions / 59 equipment definitions / 306 scripted templates, packed .battleplan, and passed byte-identity verification; explicit --vanilla-root run produced the same artifact SHA-256 6427e4b62686447be5dedde7e27c28dd84f8f1d597f366703356d28ace4a4ae4`. Proposed remaining closure condition: `Register static artifact in replay manifest, then validate live template mutation/id-join compact records from DLL path.`
- `Division template catalog` proposed status: change from `ROOT_DISCOVERY_OPEN` to `STATIC_ARTIFACT_PROVED_LIVE_JOINS_OPEN` if the orchestrator accepts a static-artifact intermediate status; otherwise keep `ROOT_DISCOVERY_OPEN`. Proposed evidence/domain text: `domain 59 lane-local scripted starting-template catalog artifact; strict proof emitted 306 division templates from 315 effective history/units files and verified replay byte identity`. Proposed remaining closure condition: `Register the strict static artifact, then close live-created/edited template roots and division-template-id joins through the game process -> injected DLL -> compact records -> Rust recorder -> .battleplan.`
