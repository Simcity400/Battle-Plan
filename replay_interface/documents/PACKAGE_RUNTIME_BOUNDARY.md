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
      replay_prefold_worker.exe # extractor-owned WorldFold worker; required sibling
      ... every other manifest-listed immutable file
```

The stable launcher reads the canonical active pointer, verifies the selected
version directory and every listed file, applies explicit signing and release
admission policies, and only then starts the manifest entrypoint. It never
guesses Steam, the game, or user-profile paths. the game content/profile selection
remains owned by `replay_app::install_paths`.

The standalone replay release must list both `replay_app.exe` and
`replay_prefold_worker.exe` in the immutable package manifest. The app resolves
the worker only as a regular non-symlink sibling of its own executable; it does
not search `PATH`, the registry, an extractor installation, Steam, or a profile
directory. This keeps the replay Cargo graph free of extractor crates while
shipping the one extractor-owned production `WorldFold` implementation needed
to activate compact-only recordings.

Updates are staged outside the active pointer. Promotion verifies the complete
candidate again, acquires a fail-closed local update lock, retains the previous
digest, and atomically replaces only `active-release.json`. Rollback verifies
the retained package again and atomically swaps current/previous digests.
Version directories are content-addressed and never rewritten by these hooks.
Launch, promotion, and rollback each require a product-supplied
`ReleaseAdmissionPolicy`; signature validity alone cannot silently authorize an
obsolete release, downgrade, channel hop, or emergency rollback.

`retain_staged_release` is the installer transaction before promotion. It
verifies the signed source, exclusively creates an inert `.incoming-*` tree in
the version store, copies and hashes each payload through a new file, flushes
the payloads, writes the canonical signed manifest last, verifies the copied
tree, and atomically renames it to `versions/<digest>`. It never changes the
active pointer. A pre-existing digest tree is independently verified and never
rewritten. Symlinks, Windows reparse points, hard-link aliases, unlisted files,
and mutation during copy are refused.

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

## Launcher invocation boundary

The static invocation mechanics are now centralized in
`invoke_active_release_from_current_cli` and
`invoke_active_release_from_current_launcher`:

1. The running stable executable is resolved without following a symlink or
   reparse alias and must have the deterministic `replay_launcher.exe` name.
   Its parent is the install root; no Steam, the game, profile, PATH, registry, or
   current-working-directory discovery is performed.
2. `discover_active_release` parses the canonical active pointer, verifies the
   selected content-addressed directory, applies the injected signature and
   active-release policies, and rereads the active pointer after hashing. A
   concurrent promotion/rollback therefore refuses the stale launch and the
   next invocation starts verification again from the new pointer.
3. Only the manifest-declared, fully verified entrypoint and its verified
   version directory reach `AdmittedEntrypointLauncher`. User arguments remain
   opaque OS argument values and are forwarded separately; this layer never
   invokes a shell or accepts a caller-selected executable.
4. Any layout, pointer, manifest, identity, hash, signature, release-policy, or
   pointer-race failure returns before the process-launch callback. There is no
   recovery-to-unsigned, fallback-version, or guessed-entrypoint path.

The API exposes these outcomes as `LauncherFailure::Preflight` versus
`LauncherFailure::ProcessCreation`. This lets a future CLI report whether trust
refused the launch or whether direct creation of an already admitted process
failed, without parsing error text. User-facing wording, telemetry handling,
and numeric exit codes remain product policy and are not selected here.

The CLI adapter treats every native argument after `argv[0]` as a replay-app
argument and performs no lossy UTF-8 conversion. It defines no launcher-owned
update, channel, downgrade, or debug switches; adding any of those requires an
explicit product policy rather than an ambiguous parser fallback.

`AdmittedEntrypointLauncher` intentionally has no default implementation. The
product-owned stable executable must create the supplied path directly, set the
supplied verified directory as its working directory, and apply the selected
Windows DLL-search and process-mitigation policy. This separation prevents the
library from silently choosing deployment policy while making the fail-closed
call order mechanically unavoidable.

The active-pointer reread closes pointer selection drift during a long hash
pass. It does not make mutable package bytes safe between their final hash and
`CreateProcess`; the production ACL and process-creation implementation remain
part of the trust boundary.

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

These choices cannot be derived from the game, replay files, build metadata, or
retained captures. They are the complete human product-policy boundary for
this runtime; layout, hashing, retention, launch admission, pointer switching,
and rollback mechanics do not require further reverse engineering.

## Installer acceptance matrix

The final pipeline and installer must run this matrix against the actual signed
Windows artifacts. Expected runtime behavior is fixed; only signer, channel,
install-scope, ACL, mitigation, and transport policy is external.

| Case | Required operation | Fixed expected result |
|---|---|---|
| clean first install | retain signed staging tree, then promote its digest | immutable digest directory retained; generation 1; no previous digest |
| identical reinstall | retain the same digest again | existing tree reverified; no rewrite; active pointer unchanged |
| ordinary upgrade | retain candidate, then promote | admitted transition increments generation; old current becomes previous |
| interrupted copy | terminate before incoming-directory rename | no active-pointer change; only inert `.incoming-*` residue may remain |
| interrupted promotion | terminate during active-pointer replacement | pointer is either complete old canonical bytes or complete new canonical bytes |
| payload/manifest tamper | alter, remove, add, hard-link, symlink, or reparse a package path | verification and launch fail before spawn |
| corrupt occupied digest | pre-create `versions/<digest>` with different bytes | retention refuses it and never replaces it |
| wrong target/product | present a validly signed package for another identity | retention, promotion, and launch fail closed |
| unsigned/unknown signer | omit signature or use a signer outside the production allowlist | production admission fails closed |
| downgrade/channel hop | present a validly signed but policy-forbidden candidate | release policy refuses pointer mutation |
| pointer race | promote or roll back while a launch verification is in progress | active-pointer reread refuses the stale launch; no process callback |
| rollback | verify current and previous, then request rollback | both releases and rollback policy pass before atomic swap |
| stale update lock | leave `.package-update.lock` behind | all mutations refuse until explicit operator recovery |
| unprivileged mutation | attempt to alter launcher, pointer, or retained tree | OS ACL test denies the write; verification independently fails on any successful tamper |
| signer rotation/revocation | test old, new, and revoked signer fixtures | selected trust policy produces the declared verdict |

## Remaining product wiring

- Connect the release pipeline to `build_unsigned_manifest`, sign its declared
  raw digest with the selected external signing service, and write its canonical
  bytes as `replay-package.json`.
- Implement the stable launcher executable that supplies the chosen production
  policy/verifier and an `AdmittedEntrypointLauncher` with the chosen Windows
  DLL-search/process-mitigation policy. The runtime-owned install-root,
  verification, argument-forwarding, pointer-race, and fail-before-spawn
  mechanics are already wired.
- Connect the selected downloader/enterprise installer to
  `retain_staged_release`, then invoke `promote_release` only after retention
  succeeds. Transport and stale-residue cleanup stay outside this crate.
- Run the matrix above with the final platform-signed launcher/installer,
  production trust material, ACLs, process policy, and release policy.
