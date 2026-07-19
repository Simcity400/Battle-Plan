# Package runtime boundary

`replay_package_runtime` is the non-UI authority shared by a future stable
launcher and installer. It does not download packages, hold signing keys, or
create tray/window UI.

The same authority deterministically constructs unsigned manifest payloads
from a complete staging directory and verifies them after installation. The
builder sorts portable paths, streams every file hash, marks the declared
entrypoint, rejects stale manifests, and leaves detached signing to the release
pipeline.

## Deterministic installed layout

```text
<install-root>/
  replay_launcher.exe       # stable, platform-signed authority
  active-release.json
  versions/
    <package-digest-sha256>/
      replay-package.json
      replay_app.exe
      ... every other manifest-listed immutable file
```

The stable launcher reads the canonical active pointer, verifies the selected
version directory and every listed file, applies explicit signing and release
admission policies, and only then starts the manifest entrypoint. It never
guesses Steam, the game, or user-profile paths. the game content/profile selection
remains owned by `replay_app::install_paths`.

Updates are staged outside the active pointer. Promotion verifies the complete
candidate again, acquires a fail-closed local update lock, retains the previous
digest, and atomically replaces only `active-release.json`. Rollback verifies
the retained package again and atomically swaps current/previous digests.
Version directories are content-addressed and never rewritten by these hooks.
Launch, promotion, and rollback each require a product-supplied
`ReleaseAdmissionPolicy`; signature validity alone cannot silently authorize an
obsolete release, downgrade, channel hop, or emergency rollback.

## Signing boundary

There is no default signing policy. Production construction requires a nonempty
algorithm-to-signer allowlist and a real `PackageSignatureVerifier`. A missing
signature, unknown algorithm/signer, absent verifier, digest mismatch, malformed
hex signature, or verifier failure refuses launch/promotion. Unsigned packages
are admitted only through the explicitly named development policy; a package
that claims to be signed is never silently treated as unsigned.

The detached signature input is defined byte-for-byte: the verifier receives
the 32 raw bytes represented by `package_digest_sha256`, never the 64-byte hex
text. This keeps release-pipeline signing and runtime verification interoperable.

The detached manifest signature does not authenticate the stable launcher or
installer binaries themselves. Platform code signing and OS ACLs must protect
those authorities. The runtime returns an admitted entrypoint path; production
must not allow that tree to be modified between verification and process
creation.

## Product decisions and human inputs

These are deployment decisions, not code-discovery or reverse-engineering gaps:

1. Choose the production detached-signature algorithm and public-key/certificate
   identity format.
2. Supply the public trust material and its rotation/revocation policy. Private
   signing credentials must remain outside the repository and runtime package.
3. Choose the platform code-signing identity and release process for the stable
   launcher and installer themselves (separate from the package manifest).
4. Choose install scope and provision OS ACLs so unprivileged users cannot edit
   the stable launcher, `active-release.json`, or version directories.
5. Choose the downloader/enterprise-installer transport and staging retention
   policy. Downloading is intentionally outside this crate.
6. Choose stale update-lock recovery policy. The runtime refuses a stale lock
   rather than guessing that its owning updater is dead.
7. Choose active-version/channel and emergency-downgrade policy, then implement
   all three decisions through `ReleaseAdmissionPolicy` in the stable
   launcher/installer.

## Remaining product wiring

- Connect the release pipeline to `build_unsigned_manifest`, sign its declared
  raw digest with the selected external signing service, and write its canonical
  bytes as `replay-package.json`.
- Implement the stable launcher executable that supplies the chosen production
  policy/verifier, calls `discover_active_release`, and creates the process with
  a safe DLL-search policy from the ACL-protected admitted directory.
- Implement the downloader/installer that writes a complete version directory
  and invokes `promote_release` only after staging is durable.
- Exercise update interruption, downgrade, ACL, signing-key rotation, and
  rollback in the final installer acceptance matrix.
