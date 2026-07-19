# Repository agent guidance

## Rust builds and tests

- This policy covers `extractor/native`, `replay_interface/native`, and their `portable_replay` dependencies.
- Subagents must not compile, build, or run tests. They may perform static analysis and edits only.
- The primary agent runs Rust validation once, centrally and serially, through `extractor/native/scripts/Invoke-CentralRustValidation.ps1`. Do not invoke Cargo directly or start competing runners.
- Complete extractor and replay validation:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File extractor\native\scripts\Invoke-CentralRustValidation.ps1 -Mode Final -Workspace All
```

- Use `-Workspace Extractor` or `-Workspace Replay` and `-Mode Check`, `Build`, `Fast`, `Integration`, or `Test` for narrower primary-agent validation. These modes accept package/target selectors; do not broaden a targeted check unnecessarily.
- `Fast`, `Integration`, and `Test` fail fast. `Final` is the complete deterministic offline battery. Run `Soak` and `Evidence` only when that evidence is required and its prerequisites are available.
- New default-tier tests must be deterministic, exercise a distinct production invariant, and require no game installation, capture corpus, live process, long sleep, stress loop, or soak budget. Put those tests in the explicit soak/evidence tiers; never silently return success when evidence is absent.
- Before deleting a test, prove that it has no distinct contract or regression value. Exact body similarity alone is not deletion proof across separate producer/consumer or domain boundaries.
- Use `-CacheMode Iterative` for repeated local edits or `-CacheMode Reusable` for clean/worktree builds. Add `-Timings` to produce an HTML Cargo timing report without a second compile.
- The shared target directory is `.cargo-target`; do not override it. Its first build is intentionally cold.
- Detailed usage: `extractor/native/scripts/CENTRAL_RUST_VALIDATION.md`.
- Read-only test-quality audit: `python extractor/native/scripts/Audit-RustTests.py`.
- Reviewed test-tier disposition: `extractor/native/scripts/TEST_SUITE_AUDIT.md`.
