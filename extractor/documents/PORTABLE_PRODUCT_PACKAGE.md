> **Read `documents/DECISIONS.md` first.** It is the target authority and outranks this file.
> This file describes the system **as built**.
>
> Superseded here: the production ZIP does not ship `collector.dll` as the recorder's sibling injection payload; that collector remains an unshipped comparison oracle.

# Portable product package

The production distribution is one offline ZIP containing exactly two executables:

- `Battle Plan Recorder.exe` — the normal desktop recorder and auto-attach supervisor;
- `Battle Plan.exe` — the replay viewer;
- `collector.dll` — the recorder's sibling injection payload;
- `README.txt` and `SHA256SUMS.txt`.

There is no installer, service, tray process, automatic updater, network fetch, administrator
requirement, publisher-signature gate, or Authenticode requirement. The checksum list detects
accidental corruption only and is not an authenticity claim.

`New-PortableProductPackage.ps1` assembles the ZIP from centrally validated release artifacts. It
does not call Cargo or build anything. It no-follow opens and single-reads every input, rejects
missing, empty, reparse-point, wrong-PE-kind, or non-GUI-subsystem binaries, refuses to overwrite an
existing output, and writes entries in a stable order with a fixed ZIP timestamp. Before atomic
publication it reopens the closed ZIP and compares every decompressed entry's length, SHA-256, and
bytes with the retained inputs, including exact `SHA256SUMS.txt` agreement.

The sole promotable package route is an unfiltered centralized
`Release -Workspace All`. After both native workspaces pass, the runner writes
and revalidates its six-artifact production attestation, invokes the package
builder using only the attested recorder, replay viewer, and collector paths,
and writes a same-directory candidate receipt that binds the exact ZIP hash to
the artifact-attestation, package-builder, and source-input hashes. The central
runner runs `Test-PortableProductPackageReceipt.ps1` against those exact staged
bytes and only then atomically publishes
`.cargo-target/release/hoi4-portable-package-receipt.json` as its terminal
transaction step; distribution must run the same
verifier without `-AllowStagedReceipt` and require both its
`PASS_CENTRAL_RELEASE_PORTABLE_PACKAGE` result and `canonical_receipt=true`.
The staged-only result is deliberately non-promotable. The
verified final receipt is the sole promotion marker, so an artifact receipt or
ZIP left by termination before that marker is not a product. A normally caught
packaging or receipt failure removes the ZIP and both promotion receipts.
Extractor-only, replay-only, package-filtered, and expression-filtered release
runs do not produce or claim this package.

The builder remains directly callable for development inspection, but that
output is not promotable without the centralized receipt and successful receipt
verification. Example diagnostic invocation after a root-owned release build:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File extractor/native/scripts/New-PortableProductPackage.ps1 `
  -RecorderExe <release-recorder-exe> `
  -ReplayExe <release-replay-exe> `
  -CollectorDll <release-collector-dll> `
  -OutputZip <destination-zip> `
  -Version <release-version>
```

The recorder resolves `collector.dll` only beside its own canonical executable. Its writable state
is created beneath the current user's LocalAppData `Battle Plan Recorder` directory with a
protected inheritable owner/SYSTEM ACL. Startup no-follow opens and freezes every path component,
rejects all reparse points and foreign-owned descendants, and reapplies the protected ACL by handle
before any recorder write. Closing the recorder window, pressing **Stop and finish recordings**, or
ending the Windows session requests coordinator-owned shutdown and waits for active workers to
drain and verify before normal process exit. The window reports recording start, verified success,
or failure with the exact attempt/replay path and provides an **Open recordings folder** button.

Running `Battle Plan.exe` with no arguments opens the Windows replay-file picker. Double-click/file
association launches pass one positional `.battleplan` path. Both routes surface startup failures in
a normal dialog; explicit option-bearing invocations remain the diagnostic CLI. On a clean machine,
the first product open then asks for the game installation folder and the game user-data folder with
native folder selectors. Each choice is validated through the existing fail-closed content/profile
validators, then the pair is revalidated and atomically stored before playback starts. Invalid
choices reopen the relevant selector; cancellation produces a visible error and no partial profile.
The picker starts in the recorder's LocalAppData output root when that directory exists.
