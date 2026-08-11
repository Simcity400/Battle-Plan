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

The authoritative code is `replay_interface/native/replay_app/src/install_paths.rs` and
`replay_interface/native/replay_app/src/content_compatibility.rs`.

## Package and install acceptance

The replayer ships inside the offline portable ZIP. `extractor/documents/PORTABLE_PRODUCT_PACKAGE.md`
is the authority on that package's shape, on the recorder-side payload and data-root rules, and on
how a build becomes promotable; the distribution policy it is built against is:

- one stable channel by manual download — no automatic updater, downgrade service, online
  downloader, or retained update staging;
- no publisher signature, detached signature, or Authenticode gate. Content hashes and
  `SHA256SUMS.txt` detect accidental corruption and are never publisher authentication, so there is
  no public trust material, key rotation, or revocation lifecycle to maintain;
- per-user operation with no administrator requirement: writable state under the current user's
  application-data directory, protected by an inheritable owner/SYSTEM ACL;
- a normal desktop window — no tray-only UI, background service, or installer-managed surface;
- standard Windows DLL-search and process mitigations, chosen so they preserve required recorder and
  replay behaviour;
- vanilla plus exact content-addressed modsets that have passed fail-closed validation; unvalidated
  modsets are rejected;
- stale locks and incomplete incoming residue are quarantined and recovered only after mechanical
  validation.

The signed version-store launcher, retained-update, active-pointer, rollback, and installer mechanics
under `replay_interface/native/package_runtime` are reusable reference code. They are not production
prerequisites, and nothing here claims signed-installer acceptance for them.

Reference playback hardware is an i5-8400 or Ryzen 5 2600, 16 GiB RAM, and a GTX 1060 6 GiB or
RX 580 8 GiB at 1920x1080, required to hold an average of 60 fps with p95 frame time no worse than
33.3 ms. That contract is enforced, not merely stated:
`replay_interface/native/replay_app/src/performance_bench.rs` and
`replay_interface/native/replay_app/Invoke-HeadlessPerformanceAcceptance.ps1`.

Package acceptance cases. These specify required behaviour; only the current canonical receipt
(`.cargo-target/release/hoi4-portable-package-receipt.json`) and a successful
`extractor/native/scripts/Test-PortableProductPackageReceipt.ps1` run establish that a particular ZIP
met them.

| Case | Operation | Required outcome |
|---|---|---|
| Full release | the unfiltered centralized release across both native workspaces | the unsigned portable ZIP and its canonical receipt are published only after exact verification |
| Non-authoritative build | extractor-only, replay-only, package-filtered, expression-filtered, or direct builder work | no promotable canonical package receipt |
| Invalid input artifact | a missing, empty, reparse-point, wrong-PE-kind, or non-GUI binary | package publication refused |
| Existing output | publishing over an existing destination | refused; never replaced |
| Archive shape | inspect the closed candidate ZIP | exactly the specified entries, in stable order |
| Archive integrity | reopen and decompress every candidate entry before publication | every entry's length, SHA-256, and bytes match the retained inputs and the checksum list |
| Receipt drift | mutate the builder, source inputs, attestation, or build environment after receipt creation | receipt verification refused |
| Staged receipt | verify a candidate receipt with staged-receipt authorization | the staged pass is explicitly non-canonical and non-promotable |

Out of scope for this product: publisher signing keys or trust material, detached package
signatures, Authenticode as a release gate, an online downloader or automatic updater, an
installer-managed version store or rollback service, a tray-only UI or background service,
administrator installation, and guessing Steam, the game, or profile paths.
