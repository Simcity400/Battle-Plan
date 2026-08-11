# Scientists Live Lane

This lane keeps only active build/test files here. Historical integration notes
were moved to `../../documents/archive/native_lanes/scientists_live_lane/`.

Current extraction completion status lives in
`../../documents/MASTER_EXTRACTION_CONTRACT.md`.

## Certification authority

`scientists_stream_proof` certifies scientists from the canonical physical H4CR
collector stream. It does not accept a detached scientist payload or manifest
row counts as proof. The production profile requires:

- the exact PERF and DATA rosters derived from `compact_contract`'s production
  descriptors;
- contiguous sequence numbers and game hours ending on physical `FRAME_END`;
- canonical record flags and payload framing;
- exact domain-41 DATA/PERF row, changed-row, and status equality;
- progress-v2 mixed-family domain-41 payloads from the DLL scan source (never a
  control bridge), with zero hidden root-failure counters;
- a full scientist baseline before deltas, valid tombstones, and equality
  between the folded logical roster and the source-observed roster; and
- a nonempty scientist roster in at least one closed frame.
- an explicit positive requested game-hour advancement count, with exactly
  `N + 1` consecutive closed frames spanning exactly `N` game hours.
- an independently emitted `scientists_live_dll_scan_source_v1` source report,
  plus the exact collector DLL and HOI4 executable named by that report. The
  verifier hashes the report bytes, both binaries, and raw/decoded stream and
  requires exact cross-artifact equality for the loaded/staged/source DLL,
  game build/module/process identity, stream duration, and scientist route.

Candidate build/test runs through the worktree-local wrapper. The stream proof
is a product/diagnostic invocation after those gates:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\extractor\native\scripts\Invoke-CentralRustValidation.ps1 -Mode Test -Workspace Extractor -Package scientists_live_lane -CacheMode Iterative
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\extractor\native\scripts\Invoke-CentralRustValidation.ps1 -Mode Build -Workspace Extractor -Package scientists_live_lane -CacheMode Reusable
cargo run --manifest-path extractor/native/scientists_live_lane/Cargo.toml --bin scientists_stream_proof --release --offline -- --stream <collector_stream.bin-or-zst> --proof <scientists_stream_proof.json> --profile production --requested-game-hours <N> --source-report <scientists_live_dll_scan_source.json> --collector-dll <exact-staged-collector.dll> --game-exe <exact-hoi4.exe>
```

The source report is an input from the retained production capture authority;
this standalone verifier never synthesizes it. Its schema is exact-key and
requires:

- `verdict="passed"`, `source_kind="dll_scan"`, domain `41`, a canonical
  session UUID, and a 256-bit evidence nonce;
- stored/decoded stream hashes and exact requested/closed/first/last hours;
- identical source, staged, and loaded collector-DLL hashes;
- exact game executable/backing-file hash, build `870ebf`, PID, process creation
  FILETIME, page-aligned module base, and PE `SizeOfImage` module size; the
  executable must also contain the supported build's daily-update and
  long-string sentinels at their file-backed RVAs;
- every scientist route offset/RVA/header constant used by this package; and
- `special_projects_scan=true`, DLL-scan source, and zero bridge rows.

Missing, malformed, forged, extra-key, or hash/identity-mismatched evidence
forces `production_certifying=false`, `profile_is_production=false`,
`source_capture_verified=false`, and a nonzero exit. A synthetically correct
H4CR stream alone can therefore never become a production source proof.

The `scientists-fixture` profile exists only for production-wire-compatible
unit fixtures with a reduced roster. The CLI deliberately returns nonzero after
writing a passing fixture report, so it cannot be mistaken for production
certification.
