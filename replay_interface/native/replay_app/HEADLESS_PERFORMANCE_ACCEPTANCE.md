# Headless performance acceptance

The F1 reference-hardware check is optional post-build performance evidence,
not a product-wide production gate or external completion dependency. It is
executable without the game, a replay recording, or a future capture, and measures
a checked-in deterministic 1920x1080 stress
workload on the production map D3D executor and selected-Present image renderer.
Its operation counts are hash-pinned to retained a729 authorities: 1,266 map
draws from XN15L and 1,167 visible UI-pass draws from the M02 closure.

This workload is **performance-only**. Its geometry, shader, content identity,
and UI pixels are synthetic. It is not replay-semantic evidence, pixel-parity
evidence, reverse-engineering evidence, or rendering-law/promotion evidence.
The contract and both receipt layers carry those prohibited claims explicitly.
The separate recording-backed benchmark remains available as a stronger profile
when an authenticated production recording is supplied.

The reference class is Intel Core i5-8400 or AMD Ryzen 5 2600, nominal
16 GiB RAM, and NVIDIA GTX 1060 PCI `10DE:1C03` with exactly 6 GiB dedicated
VRAM or AMD RX 580 PCI `1002:67DF` with exactly 8 GiB. The selected DXGI adapter
identity is authoritative; WMI display names are not. The renderer warms 120
frames and measures 600. Every timed frame ends with a D3D11 EVENT query, so
measurement includes GPU completion. Acceptance requires at least 60 average
FPS, average frame time at most 16.666667 ms, and p95 at most 33.3 ms.

Run the centrally built Release executable from PowerShell:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File `
  .\Invoke-HeadlessPerformanceAcceptance.ps1 `
  -ReplayApp C:\path\to\replay_app.exe `
  -ReleaseArtifactReceipt C:\path\to\hoi4-production-prebuilt-artifacts.json `
  -Output C:\path\to\f1-performance.json
```

The wrapper accepts only the `replay_app.exe` path and hash authenticated by a
successful central `Release -Workspace All` receipt. It holds a deny-write and
deny-delete handle over the executable, verifies equal pre/post hashes, checks
the exact CPU/RAM/GPU contract, independently revalidates all checked-in hashes
and counts, recomputes every statistic, and emits an immutable aggregate JSON
receipt. The immutable raw renderer receipt is retained beside it as
`<output>.renderer.json` and bound by byte length and SHA-256. Missing inputs,
hash drift, count drift, operation instability, omissions, hardware mismatch,
or threshold failure fail closed for this optional receipt. Absence of matching
reference hardware or of an F1 receipt does not disable recording, packaging,
replay admission, or any exact-authority product lane.

The lower-level stress command is diagnostic because it does not validate the
central Release attestation or CPU/RAM inventory:

```powershell
replay_app.exe `
  --benchmark-contract .\headless_performance_workload.json `
  --benchmark-stress-output C:\path\to\renderer-stress.json `
  --benchmark-warmup-frames 120 --benchmark-sample-frames 600 `
  --non-interactive
```

The stronger recording-backed profile remains separate and fail-closed in
`headless_performance_recording_workload.json`:

```powershell
replay_app.exe --recording C:\path\to\session.battleplan `
  --benchmark-contract .\headless_performance_recording_workload.json `
  --benchmark-output C:\path\to\renderer-recording.json `
  --benchmark-warmup-frames 120 --benchmark-sample-frames 600 `
  --non-interactive --config C:\path\to\profile.json
```

That recording contract deliberately remains disabled until its exact
authenticated production workload fields are populated. Its absence cannot
block the optional F1 reference check or the product and does not authorize
fabricated recording evidence.
