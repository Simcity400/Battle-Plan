# Production the game content VFS

Status: production implementation and focused fixtures pass. This report is
separate from the rendering-law packet reports and from extractor RE records.

## Product boundary

The replayer receives an install root and user-data/profile root selected by
the user. No vanilla catalog, install path, user path, asset list, or DLC
ownership assumption is compiled into the resolver. The extractor and replayer
remain independently distributable; the portable replay content manifest is
the only compatibility contract between them.

`GameFileResolver::from_dlc_load` reproduces the selected local profile for
inspection/recording. `GameFileResolver::from_recorded_content_layers` builds
an exact playback VFS from only the descriptor-relative layers marked required
by the replay and preserves their original, potentially sparse, recorded
ordinals. Extra locally active DLC/mods are omitted so they cannot change a
winner; missing required layers fail with the exact stable source ID.
Recorded-subset construction identifies requested descriptors directly from
their stable IDs and launcher enabled/disabled state. It never opens, parses,
or materializes an unrequested descriptor/archive, so a corrupt or missing
locally active but replay-unused layer cannot block playback.

## Effective namespace

The production namespace is shared by UI, map, shader, entity/model/particle,
localisation, define, and scripted-content callers. Paths are slash-normalized,
case-folded for collision selection, and rejected when absolute,
drive-qualified, NUL-bearing, or parent-traversing. Every public file,
directory, and recursive-enumeration route also canonicalizes candidates
against the mounted layer root; symlink/junction escapes fail closed.

Mount order is base, integrated DLC, enabled non-disabled DLC, then enabled
mods. The launcher `enabled_mods` sequence is retained for local-profile
inspection. Exact playback never infers an engine order: it consumes the
recorded ordinal and stable source ID of each required layer. Descriptor
`dependencies` are retained in declaration order as provenance and are not
used to invent a secondary sort. DLC selection uses the stable, lower-cased
full DLC folder/content ID (for example `dlc003_rocket_launcher_unit_pack`) on
both recording and playback sides; it does not collapse distinct products to
a short numeric prefix.

Loose content directories and ZIP-backed DLC/mods use the same layer law.
Archives are expanded to a SHA-256-addressed cache under the selected profile,
using a completion marker and staging directory; unsafe or case-colliding
members fail closed. The archive SHA identifies the source container and the
resolved/decompressed file SHA identifies the payload.

Later layers win a case-insensitive relative-path collision. `replace_path`
removes lower-layer files whose immediate parent is the exact declared
directory; it is intentionally non-recursive, and the replacing layer may
re-add files.

Every winning file can be returned with path-free provenance: normalized
logical path, source kind, descriptor-relative source ID, mount ordinal,
descriptor SHA-256, archive SHA-256 when applicable, declared `replace_path`
and dependencies, payload length, and payload SHA-256. Local materialization
paths are runtime-only and are skipped during serialization.

Descriptor identity is the SHA-256 of compact UTF-8 semantic JSON under schema
`hoi4.semantic-descriptor.v1`: trimmed name/version/supported-version/remote ID,
slash-normalized lower-cased `replace_path` in declaration order, and trimmed
first-occurrence-deduplicated dependencies in declaration order. Physical
`path`, `archive`, descriptor filename, comments, and formatting are omitted,
so relocating identical content does not change its descriptor identity.
Archive bytes remain independently authenticated by the source-container hash.

## Portable replay admission

`GameFileResolver::playback_inventory` resolves the manifest's required paths
through the selected playback roots and returns the neutral
`PlaybackContentInventory`. After the portable manifest validates that
inventory, `GameFileResolver::with_content_admission` binds the resulting
capability to the ordinary concrete resolver.

An admission-bound resolver preserves existing renderer APIs while making all
single-file and directory-listing routes fail closed for unrecorded paths. It
checks winning layer kind, stable ID, ordinal, archive/file source SHA-256,
payload SHA-256, and payload length. Installed but unrecorded content is never
silently admitted.

## Engine-law and content separation

Hard-coded production behavior is limited to the evidenced resolver laws:
ordered overlays, case-folded path collisions, exact-immediate-parent
`replace_path`, descriptor-relative source identity, and archive payload
materialization. DLC names, mod names, asset paths, content bytes, UI layouts,
definitions, localisation, shaders, and catalogs all come from the selected
installation/profile through the effective VFS.

The retained UI14 evidence does not prove a729 engine MOD parser call order
from launcher state alone. Therefore launcher JSON order is not labelled an
engine law. Exact playback uses the extractor-recorded order from the portable
file, and absence of that order is a compatibility failure rather than a
fallback sort.

## Verification

Focused Rust gate (primary agent only):

`extractor/native/scripts/Invoke-CentralRustValidation.ps1 -Mode Test -Workspace Replay -Package map_viewer -FilterExpression 'test(mod_resolver)'`

Result: 19 passed, 0 failed. The fixtures cover loose overrides, exact and
non-recursive `replace_path`, recursive listings, case/path normalization,
absolute/drive/parent/NUL rejection, canonical symlink/junction containment,
launcher order versus explicit sparse recorded ordinals, direct ZIP mounting
without extractor staging, missing archive/descriptor failure, enabled versus
disabled DLC, stable full DLC IDs, semantic descriptor identity across two
machine-specific roots, changed UI/definition content, path-free provenance,
portable inventory, relocated roots, and denial of unrecorded installed files
after admission. A dedicated regression also places a corrupt active unused DLC
descriptor and a missing enabled unused mod beside a valid required layer and
proves required-only playback never opens either unused layer.

Build gate (primary agent only):

`extractor/native/scripts/Invoke-CentralRustValidation.ps1 -Mode Build -Workspace Replay -Package map_viewer`

Result: success; all map-viewer library, binary, and integration-test targets
compiled. Existing unrelated warnings remain outside this VFS lane.

Those results predate the 2026-07-20 archive-mod/offline-preparation delta.
Current certification belongs to the parent wave's single centralized Final
battery; UI15's Python verifier never invokes Cargo itself.

Machine-readable evidence is in
`replay_interface/re_evidence/production_vfs/manifest.json`.
