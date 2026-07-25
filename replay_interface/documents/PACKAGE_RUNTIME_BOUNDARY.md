# Package runtime boundary

## Selected production distribution

The selected production distribution is one unsigned, offline, portable ZIP.
It contains exactly:

- `Battle Plan Recorder.exe`;
- `Battle Plan.exe`;
- `collector.dll`;
- `README.txt`; and
- `SHA256SUMS.txt`.

There is no installer, stable launcher, service, tray-only process, automatic
updater, downloader, version store, administrator requirement, publisher
signature gate, or Authenticode prerequisite. `SHA256SUMS.txt` detects accidental
corruption; it is not publisher authentication.

All twelve product-policy decisions are resolved in
`extractor/documents/PRODUCTION_PROMOTION_HUMAN_INPUT_REGISTER_20260719.json`
and mirrored by `PACKAGE_INSTALL_ACCEPTANCE_MATRIX.json`. There are zero
unresolved human deployment choices. CID1 M3's second physical machine remains
the only external dependency, and it is unrelated to packaging.

## Build and publication authority

`New-PortableProductPackage.ps1` is the only selected package builder. It runs
after an unfiltered centralized `Release -Workspace All`; it never calls Cargo.
It consumes only the centrally attested recorder, replay viewer, and collector
paths, rejects reparse points and wrong PE kinds, no-follow opens and
single-reads every input, writes a stable ZIP with fixed timestamps, then
reopens the closed ZIP and verifies every decompressed entry's bytes, length,
SHA-256, and checksum-list row.

The terminal central-release receipt binds the ZIP hash to the artifact
attestation, package builder, and source-input hashes. Distribution must run
`Test-PortableProductPackageReceipt.ps1` without `-AllowStagedReceipt` and
require `PASS_CENTRAL_RELEASE_PORTABLE_PACKAGE` with
`canonical_receipt=true`. A staged ZIP, an artifact receipt without the
terminal package receipt, or a filtered release run is not a promotable
product.

## Runtime layout and process boundaries

The user extracts the ZIP into a user-writable, non-reparse directory and runs
either executable directly. The recorder resolves `collector.dll` only beside
its own canonical executable. It never searches `PATH`, the registry, Steam, a
profile directory, or another extractor installation for that payload.

Recorder writable state lives under the current user's LocalAppData
`Battle Plan Recorder` directory. Startup no-follow opens and freezes every
path component, rejects reparse points and foreign-owned descendants, and
applies the protected owner/SYSTEM ACL by handle before writes. Closing the
window, pressing **Stop and finish recordings**, or Windows session shutdown
requests coordinator-owned shutdown and waits for active workers to drain and
verify.

`Battle Plan.exe` with no arguments opens the replay-file picker. A single
positional `.battleplan` path supports double-click/file association. Both
routes surface startup failures in a normal dialog; option-bearing invocations
remain diagnostic CLI use. First product open uses native folder selectors for
the game installation and user-data roots, validates both through the existing
fail-closed profile/content rules, and atomically stores only a valid pair.

The standalone replay release does not ship `replay_prefold_worker.exe`.
Compact playback invokes the admitted recorder sibling in its hidden internal
worker mode and binds the request/receipt to a retained-handle SHA-256
and file identity. The worker has no `PATH`, registry, extractor, Steam, or
profile fallback.

Shipped compact playback has a 2 GiB uncompressed source ceiling. The manifest
declaration is rejected before spooling and the decompressor enforces the same
limit while writing. The internal child is limited to 15 minutes wall time,
15 minutes CPU time, 2 GiB process memory, one active process, 32 MiB stdout,
and 1 MiB stderr. Every limit is fail-closed.

## Legacy package runtime disposition

`replay_package_runtime` retains deterministic manifest, signature-policy,
content-addressed version-store, promotion, rollback, and stable-launcher
mechanics as unselected legacy/reference code. It is not called by the selected
portable ZIP route and is not a production-completion dependency.

In that legacy model, signed claims are still verified fail-closed: a missing
or unknown signer, malformed signature, digest mismatch, stale pointer,
unlisted file, hard-link alias, symlink/reparse path, or policy-forbidden
transition refuses before launch or pointer mutation. Those guarantees must not
be weakened merely because the model is unselected. Conversely, no document or
test may describe its signer, installer, downloader, channel, ACL, or rollback
choices as unresolved human production inputs for the portable product.

The legacy installer/signature acceptance matrix is reference coverage only.
The selected portable acceptance matrix is the mechanically specified matrix
in `PACKAGE_INSTALL_ACCEPTANCE_MATRIX.json`; passing it is asserted only by a
current authenticated receipt, never by this document.

## Remaining work

There is no signing, installer, downloader, stable-launcher, channel, or
version-store wiring remaining for the selected product. Remaining release
work is mechanical: central validation, package construction, terminal receipt
verification, and publication of those exact authenticated bytes.
