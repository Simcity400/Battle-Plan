> **Read `documents/DECISIONS.md` first.** It is the target authority and outranks this file.
> This file describes the system **as built**.
>
> Superseded here: the Domain 79/80 in-process VFS observation and container-v2 sealing flow is not the target content-acquisition architecture; the VFS and portable manifest remain reusable proven work.

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

### Symbol-assignment companion

The same neutral crate owns `symbol_manifest::SymbolManifest`
(`hoi4.portable-symbol-manifest.v1`), the deterministic symbol-id artifact required by
`WIRE_FORMAT.md` §5. It is a companion to content/VFS identity, not an optional reinterpretation
of it:

- the base band is the exact UTF-8-bytewise sorted, deduplicated vocabulary of the admitted
  content set and receives zero-based ids;
- genuinely new campaign strings enter one append-only overflow band, while a complete serialized
  batch ledger records availability events for every container append ordinal and `(u32 branch,
  tick)`, including existing-symbol-only and no-new-event batches;
- the container's contiguous append ordinal is the global order. Branch-local ticks increase, but
  there is no false global `(tick, branch)` order, so a fork from the past remains representable;
- each batch is bytewise canonical. A divergent branch can add availability for an existing
  file-global id, while repeat observations already visible on the same or an ancestor branch add
  no redundant event;
- recorder-local numeric references are bound as namespaced `(namespace, source_id)` keys to
  authenticated observed symbols, and ambiguity or an unresolved reference fails closed;
- d18 `country_identity` supplies country and cosmetic-tag bindings. It is a container
  symbol-table feed, not a reason to invent a string scalar or repeat the static roster as a
  domain block;
- a mapper resolves both `ContentId` values and `ContentId` key components before record/key
  construction. Runtime database words are never cast directly into portable manifest ids.

Cross-branch lookup is container-authoritative. The symbol artifact resolves same-branch
availability from the event tick and append ordinal, but gives the authenticated container branch
table both full append points when asking whether an event is visible from another branch
(including the fork-point rule). If that answer is unavailable, lookup fails closed.

The compact JSON byte representation is canonical and reader-capped. Its top-level field order is
`schema`, `base_symbols`, `overflow_symbols`, `batches`; each batch is `ordinal`, `branch`, `tick`,
`available_symbols`. Decode requires byte-identical canonical re-encoding and rejects reordered or
whitespace-varied JSON, invalid ids, and malformed assignment or availability history. The
conservative safety caps are 256 MiB of encoded JSON, 1,000,000 symbols, 65,536 bytes per symbol,
128 MiB of total symbol bytes, 10,000,000 batches, and 10,000,000 availability events. Before
serde, decode rejects any raw JSON string token longer than 393,216 bytes (six times the decoded
symbol-byte cap, preserving every legal JSON escape spelling). During serde, it checks each decoded
string before retaining it, bounds base plus overflow symbols as one aggregate, bounds their decoded
bytes as one aggregate, and passes the remaining availability-event capacity into each next batch
before that batch's id vector is allocated. Once any vector reaches its limit, the reader probes an
excess element as ignored JSON rather than deserializing the element type. Semantic validation still
performs the corresponding aggregate symbol/count checks after decode and before canonical
acceptance.

Runtime numeric bindings are intentionally not serialized: after reload, an authenticated feed
rehydrates bindings only to symbols already in the manifest, in a separate transactional operation
that cannot mint ids or append a batch.

The companion is a separate serializable type because the deployed
`hoi4.portable-content-manifest.v2` JSON body and digest are already a compatibility boundary.
Adding a required field to that struct would rewrite every existing manifest and every
independent replay consumer. Container storage and the older dNN-mapper resolver retrofit remain
separate integration gates; they do not reopen the assignment rule.

## Recorder flow

1. The launcher-equivalent resolver supplies the effective VFS digest, ordered
   base/DLC/mod layers, archive/directory identity, and `replace_path` list.
   Its collision join enumerates decompressed archive members as logical VFS
   paths, so archive/loose and archive/archive winners use the same exact UI14
   receipt law as directory layers.
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
   `portable_content_manifest_v2` and validates the digest when reopened. The
   v2 body requires the static-bundle-v3 `product_modset_sha256`; v1 containers
   are migration-only inputs and are not production-admissible.
12. Once a live resolver has emitted the authenticated six-file binding corpus,
   `prepare_product_modset_capture.py --bindings` can copy, re-hash, and rebuild
   the capture-ready product offline. It never queries a stale PID or rereads
   launcher/log state, and it must reproduce the same path-independent product
   key before static generation proceeds.

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
- Private catalog staging rewrites only the physical descriptor content field,
  preserves all other narrow-string bytes losslessly, and admits archive mods
  only when the normalized staged namespace equals the authenticated archive
  namespace exactly.
- Source-inventory namespaces marked `binding=asset_only` are content
  observations, not selected playback layers. They are excluded from playback
  precedence and the remaining namespace ordinals are reindexed exactly.
- Exact playback must reject missing/wrong winning layers or hashes. It must
  not substitute absent DLC/mod content.

## Mechanical verification

Run from the repository root:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File extractor\native\scripts\Invoke-CentralRustValidation.ps1 -Mode Final -Workspace All
```

The v2/product-modset boundary added on 2026-07-20 is pending the parent
wave's single centralized validation run. No subagent build result is claimed.

Historical pre-v2 verification on 2026-07-18:

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
