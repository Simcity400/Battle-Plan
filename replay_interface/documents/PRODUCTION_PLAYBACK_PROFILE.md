# Production playback profile and compatibility gate

The standalone replayer stores only two machine-local values: the selected the game
content root and the selected the game user-data/profile root. They live in the
replayer's versioned local configuration and never enter a `.battleplan` file.
The recording remains portable and may be renamed or moved independently of the
extractor and the source machine.

On first launch, or with `replay_app --configure`, the user selects both roots.
`--config <file>` provides a deployment/headless configuration location and
`--non-interactive` converts every missing or invalid selection into an
actionable failure. Explicit `--game` and `--userdata` values reselect moved
roots. The legacy `product_paths.json` shape is migrated after validation.

Install validation is content-based (`common`, `gfx`, and `map` roots), not tied
to Steam, a default drive, or `hoi4.exe`. Profile validation requires the
launcher's `dlc_load.json`. The production VFS then reconstructs the effective
base/DLC/mod/archive layers, order, overrides, and `replace_path` outcome before
any replay content is admitted.

The playback resolver mounts only layers marked required by that recording and
retains their original (possibly sparse) engine ordinals. Locally enabled but
unused mods or DLC are invisible to the replay, so they cannot change a winner.
DLC comparison uses the shared stable launcher identity, while descriptor
identity is a semantic, path-redacted digest; relocated installations and
absolute descriptor `path` changes are not treated as content changes.

Playback is fail-closed through the portable content manifest. Errors name the
specific mismatch: game version/build, required DLC, required mod/content layer,
layer identity/order, `replace_path`, VFS winner, source/payload hash or length,
language, or material settings. Missing content is never silently substituted.
Selected but unused DLC is provenance only and does not block playback.

Production containers require `hoi4.portable-content-manifest.v2` and the
`portable_content_manifest_v2` feature. The signed manifest carries the
extractor static-bundle-v3 `product_modset_sha256`; container open, playback
admission, production map/UI preparation, D3D execution receipts, and headless
pixel receipts retain that key. It is full capture-product provenance, not a
requirement to install every unused source-layer byte: local compatibility
continues to validate the exact `required_content` capability set.

Profile/playset labels are provenance only. Compatibility uses a reproducible,
path-redacted selection digest; machine paths and launch timestamps are not
compatibility identities.

## Mechanical gates

- first launch and saved-profile reuse
- invalid/stale path reselection and non-interactive failure
- moved install and profile roots
- legacy configuration migration
- content-root validation without a platform/store executable assumption
- relocated/renamed replay file open
- every compatibility issue class has a distinct fail-closed diagnostic
- unused/optional DLC is distinguished from required DLC
- sparse required ordinals survive omission of unused local layers
- unrelated enabled DLC/mods cannot override or substitute required content
- descriptor identity is stable when only the machine-local root changes
- a non-UI-only replay binds without constructing or requiring any UI catalog

The authoritative code is `native/replay_app/src/install_paths.rs` and
`native/replay_app/src/content_compatibility.rs`. Test receipts and source hashes
are frozen separately under `re_evidence/production_playback_profile/`.
