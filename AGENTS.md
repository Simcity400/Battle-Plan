# Repository agent guidance

## Current Ghidra broker authority

- Always call `ghidra_status` before Ghidra work. The active exact-a729 broker
  is expected to report `ready=true`, `execution.shard_count=20`, shards
  `shard-01` through `shard-20` ready, read-only/no-analysis mode, image base
  `0x140000000`, and SHA-256
  `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.
- Do not reconfigure or reset the broker to ten shards. Historical
  `exact-10`, `ten-shard`, and `invalid-20-shard` receipts describe a closed
  one-time evidence replay; they are not the current broker topology contract.
- The configured shard count is operational capacity, not semantic evidence
  provenance. Do not reject, quarantine, or downgrade a job solely because it
  ran while 20 shards were configured. Validate the executing shard's binary
  identity, mode, job ID, request/manifest hashes, and artifact hashes.
- Agent/subagent concurrency limits are unrelated to Ghidra shard count.

## Ghidra evidence: query the index before exporting

A SQLite index over the export store answers "what do we already know?" in
milliseconds without launching Ghidra. Re-exporting indexed evidence is the
biggest source of wasted time and storage here, and exports are far larger than
they look: `depth=0` is not seed-only, since each seed's direct callers come too.

- Find — `ghidra_coverage` (is this already exported?), `ghidra_search_decompiled`,
  `ghidra_functions_for`, `ghidra_subsystems_of`
- Read — `ghidra_get_function`
- Traverse — `ghidra_callers`, `ghidra_callees`, `ghidra_path_between`
- Export — `ghidra_export_functions`, only once a query shows the address is
  genuinely unindexed
- **Record — `ghidra_annotate_function`, before you call the investigation done**

Completed exports fold themselves into the index before the call returns, so new
evidence is queryable immediately; the response's `index_refresh` reports how it
went, and a lagging cache never means lost evidence.

**Recording is not optional.** The index saves Ghidra's *output* automatically —
decompiled C, disassembly, p-code, call graphs — but it cannot save what you
worked out. As of 2026-08-05 the registry holds **15 claims against 110,803
indexed functions**, so the next agent to open a function you already understood
sees the same raw decompile and pays your analysis cost again. Exports no longer
get repeated; conclusions still do. An investigation that ends without a claim has
not finished, it has only been done privately.

**The index is advisory.** Results report what was indexed at build time and never
assert that anything is still current. `ghidra_verify` is the only tool that
compares cache against disk: call it and cite the identity it actually reports
before publishing a claim. `ghidra_annotate_function` is the only writing tool.

**Never rewrite artifacts inside a completed job directory.** They are hardlink-
deduplicated, so many paths share one physical file and an in-place edit would
corrupt every job citing it. Derive new evidence into a new job.

Operating guide: `replay_interface/tools/ghidra/README.md`.

## Unified agent-native RE tooling

- Use the `hoi4_re` MCP facade for shared build/symbol lookup, patch migration,
  registered live-memory observations, bounded Frida call traces, constrained
  Unicorn emulation, and save/memory correlation. Start with `re_status`.
- The facade does not replace the current Ghidra authority. Continue to call
  `ghidra_status` before exact-a729 Ghidra exports and follow every rule above.
- New-build analysis runs only in isolated writable projects under the facade's
  machine-local state root. Never repoint or open a current authority shard
  read-write for diffing.
- `observe_runtime` is read-only. `trace_calls` is hook-based instrumentation;
  use it only when live work is in scope, and never treat a trace or diff match
  as automatic promotion evidence.
- Operating guide: `replay_interface/tools/hoi4_re/README.md`.

## Rust builds and tests

- This policy covers `extractor/native`, `replay_interface/native`, and their `portable_replay` dependencies.
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
- Read-only test-quality audit: `python extractor/native/scripts/Audit-RustTests.py`.
