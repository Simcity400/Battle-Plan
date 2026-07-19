# Portable content manifest: extractor production contract

Status: **in-process extractor/VFS interpreter observations and container seal
promotion-ready**. Production does not accept a user-authored content-trace
sidecar. Renderer routes without an authenticated runtime observer are omitted,
not backfilled from vanilla catalogs.

This document is extractor-only. It does not amend or duplicate the rendering
law reports under `replay_interface/documents/lane_reports/rendering_laws/`.

## Boundary

The extractor and replayer remain independently distributable. Their only
binding artifact is the portable `.battleplan` file. The extractor records
identity and hashes, never install/profile roots or copied vanilla catalogs.
The replayer resolves the recorded paths through the selected playback VFS and
admits them only when the portable manifest validates.

The neutral schema is owned by
`portable_replay/native/content_manifest`. The extractor adapter is
`extractor/native/recorder_host/src/portable_content_manifest.rs`.

## Recorder flow

1. The launcher-equivalent resolver supplies the effective VFS digest, ordered
   base/DLC/mod layers, archive/directory identity, and `replace_path` list.
2. The finalized static bundle's authenticated
   `proofs/catalog_source_join.json` identifies every effective VFS file
   actually consumed by each admitted extractor interpreter. It records exact
   winning namespace ordinal, relative path, byte count, and payload hash.
3. Production finalization constructs observations directly from that join and
   its double-read `source_inventory.json`. No external content trace is read.
4. `RecorderContentTrace::record_observation` and `write_atomic` remain a
   bounded debug/test handoff for typed runtime observers. Production/profile
   CLI rejects `--portable-content-use-trace`.
   Production fold-and-repack also rejects legacy captures without an already
   embedded in-process content manifest; it cannot retrofit provenance from a
   sidecar after capture.
5. The normal production finalizer scans the authenticated compact stream in
   process. Domain 79 supplies the latest clean pointer-free HUD activation;
   the recorder rebuilds the exact recorded-layer VFS from launch-corroborated
   roots, walks each observed runtime GUI root, and appends only dependency
   closures whose GUI/GFX/texture/font/localisation winners exactly match the
   finalized source inventory. Unknown or incomplete surfaces are omitted as a
   unit. No HUD sidecar, vanilla filename list, or authored-only root pairing is
   accepted.
6. Domain 77 also carries the exact M02 combat-indicator frame. The finalizer
   scans every authenticated production frame, validates each M02
   `UsedContentKey`, rejects winner drift across the recording, and admits only
   the matching source-inventory winners for
   `replay_app.production_ui.m02_combat_indicator`.
7. Domain 80 is the dedicated canonical map-content snapshot. It is separate
   from domain 77's production render frame and enters the manifest only when
   an actual authenticated lookup payload is present. Domain 80 remains
   unscheduled until that genuine engine lookup producer exists; the recorder
   never fabricates an all-vanilla snapshot.
8. The adapter cross-checks executable SHA-256, runtime version/build, profile
   and playset provenance, exact ordered mod identities, enabled DLC, language,
   and display/settings identity against the attached launch.
9. Repeated identical effective paths are merged with sorted unique consumers
   and an exact observation count. Any key, kind, winning layer, source hash,
   payload hash, or payload-length drift fails closed.
10. Required layers and DLC are computed from actual winning observations.
   Selected-but-unused DLC remains recorded but is not silently required.
11. The shared selection digest and manifest digest are computed, and the
   content manifest enters `PackIdentity`. The container declares
   `portable_content_manifest_v1` and validates the digest when reopened.

Production capture cannot finish with an empty, mutating, malformed, or
launch-mismatched finalized catalog/source join. Debug/recovery captures may
use an explicit trace for fixture work and are not portable production
artifacts.

## Stable identities

- Base layer: `base`.
- Mod layer: normalized descriptor-relative ID such as `mod/example.mod` plus
  the path-redacted semantic descriptor hash. Physical `path`/`archive`,
  filename, whitespace, and comments do not affect that hash; semantic fields,
  ordered `replace_path`, and dependencies do.
- DLC layer: descriptor-relative ID such as
  `dlc/dlc003_rocket_launcher_unit_pack/dlc003.dlc`.
- Integrated DLC descriptors bind the semantic descriptor identity to their
  physical containing directory. An authored `archive` or `path` field cannot
  redirect that engine-integrated mount.
- Mod and DLC descriptors use the same lossless Windows/Latin-1 byte mapping as
  playback before semantic parsing, so valid non-UTF-8 names preserve identical
  cross-product descriptor hashes.
- DLC selection: the full product directory ID, such as
  `dlc003_rocket_launcher_unit_pack`.
- Content path: normalized game-relative VFS path only.
- Source identity: hash of archive bytes for archive entries, otherwise source
  file bytes; payload identity is the resolved/decompressed bytes consumed.

Profile/playset labels are provenance. Playback compatibility uses the shared,
reproducible selection digest derived from exact layers, required DLC,
language, and rendering-relevant settings.

## Portability and fail-closed rules

- Absolute install, user-data, Workshop, OneDrive, and temporary paths are
  forbidden from the packed recorder manifest.
- The local run manifest may retain those roots for recovery/audit. Its packed
  copy is separately redacted and then scanned; an embedded absolute-path
  fragment rejects packaging.
- No vanilla asset, definition, catalog, layout, or path list is a production
  default. Test fixtures are evidence only.
- A changed mod definition/asset changes the content-manifest digest.
- Directory and archive mods, `replace_path`, changed DLC selections, and a
  relocated replay are represented without machine paths.
- Source-inventory namespaces marked `binding=asset_only` are content
  observations, not selected playback layers. They are excluded from playback
  precedence and the remaining namespace ordinals are reindexed exactly.
- Exact playback must reject missing/wrong winning layers or hashes. It must
  not substitute absent DLC/mod content.

## Mechanical verification

Run from the repository root:

```powershell
cargo test --manifest-path extractor/native/recorder_host/Cargo.toml --lib portable_content_manifest::tests -- --nocapture
cargo test --manifest-path extractor/native/recorder_host/Cargo.toml --bin collector_record portable_recorder_manifest_redacts_every_local_root_but_keeps_stable_ids -- --nocapture
cargo test --manifest-path extractor/native/recorder_host/Cargo.toml --bin collector_record production_rejects_debug_content_trace_sidecar_and_uses_in_process_join -- --nocapture
cargo test --manifest-path extractor/native/recorder_host/Cargo.toml --bin collector_record parse_args -- --nocapture
```

Verified 2026-07-18:

- portable content-manifest adapter: 22 passed, 0 failed;
- d79 fold/runtime-stream subset: 3 passed, 0 failed;
- d80 canonical fold: 1 passed, 0 failed;
- production render-submit carrier: 10 passed, 0 failed;
- replay-core typed carrier suite: 50 passed, 0 failed;
- XUI13 design-catalog contract: 10 passed, 0 failed;
- A27 recorder seal/relocate/admit gate: 1 passed, 0 failed;
- full frozen A27 projection: 1 passed, 0 failed, exactly 6,407 pointer-free
  records from the 20,734,645-byte authenticated source;
- packed recorder-manifest path redaction: 1 passed, 0 failed;
- CLI parse regression subset: 14 passed, 0 failed.

The adapter suite covers deterministic repeat aggregation, atomic trace
publication, no-sidecar production construction, early foreign-VFS rejection,
path-redacted cross-machine descriptor identity, non-UTF-8 Latin-1 mod/DLC
semantic identity and DLC archive hashing, winner/hash drift rejection,
mod/DLC/language/settings cross-checks, directory/archive and `replace_path`
identity, integrated-DLC physical-directory binding, `asset_only` namespace
exclusion/reindexing, M02 exact-winner admission, changed definition/asset
identity, runtime build binding, portable container roundtrip, and replay
relocation.

## Integration ownership

`production_render_submit_live` owns typed resolver-backed submit promotion and
must not depend on `recorder_host`. Domain 77 remains exclusively the production
render frame; domain 80 owns map-content lookup observations. HUD semantic-role
snapshots are produced from domain 79 by `hud_role_observer_live_lane` and the
effective-VFS emitter in `game_ui::hud_role_index`, then merged only by the
explicit recorder adapter. Unobserved runtime routes are absent from production
rather than silently assigned vanilla content.

During this integration, `world_fold::FOLDED_DOMAIN_IDS` was found stale: it
omitted domains 70 through 78 even though their decoders were already active.
The registry now names every decoded domain through 80, so generic exact-length
and domain-accounting gates cover those existing carriers as intended.
Static definition, localisation, GUI/GFX catalog, and scripted-content
interpreter sources are already observed through the finalized catalog/source
join.
