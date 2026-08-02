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

## Indexed Ghidra queries — try these BEFORE exporting

A SQLite index over the export store answers most "what do we already know?"
questions in milliseconds, without launching Ghidra. Exporting to rediscover
something already on disk is the single biggest source of wasted time and
storage in this repo: the store held 2,080,565 artifact files covering only
248,629 distinct contents, because agents had no way to ask first.

**Start every function-level investigation with a query, not an export:**

- `ghidra_coverage` — for a list of addresses: what is already exported, in which
  jobs, and what claims exist. The "has anyone looked at this?" call.
- `ghidra_get_function` — decompiled C, disassembly, p-code and call graph for an
  address, straight from the index.
- `ghidra_search_decompiled` — FTS5 search across the decompiled corpus when you
  have a term but no address.
- `ghidra_callers` / `ghidra_callees` / `ghidra_path_between` — traverse the
  global call graph instead of exporting closures to discover relationships.
- `ghidra_functions_for` / `ghidra_subsystems_of` — reach functions by subsystem
  name rather than guessing addresses.
- `ghidra_index_status` — index counts and build metadata.

Call `ghidra_export_functions` only when a query shows the function is genuinely
not indexed, or a fresh export is required. Export limits are unchanged
(`depth` 0..3, 1..32 addresses) and nothing here caps what you may ask for. Note
that `depth=0` is not seed-only — the exporter always adds each seed's direct
callers, so a depth-0 job routinely yields hundreds to thousands of functions.

### The index is ADVISORY — it never claims to be current

Query results report **what was indexed at build time**. They carry
`indexed_sha256` and `indexed_bytes` (build-time values, not fresh measurements)
plus a full citation: `job_id`, `job_dir`, `artifact_relative_path`,
`canonical_path`, `path`. No query result asserts that anything is verified, and
`indexed: true` means "the index knows about it", never "it is present and
unchanged on disk".

`ghidra_verify` is the only tool that compares cache against disk. It rereads the
cited file, reports `agrees` with actual hash and size, and — when the canonical
copy disagrees — points to a provenance copy that still matches.

**Before publishing a claim in a findings packet: cite the artifact path plus
`indexed_sha256`, call `ghidra_verify`, and record the actual identity it
reports.** Hash agreement proves the bytes are unchanged; it does not validate
the semantic conclusion drawn from them, which remains yours to defend.

`ghidra_annotate_function` appends a finding to the shared `hoi4_re` claim
registry and is the only indexed tool that writes. A claim's
`registry_claim_status` is claim-lifecycle metadata from that registry; a value
of `verified` there is an editorial state, not artifact confirmation.

### The index maintains itself

You do not have to remember to rebuild it. A completed `ghidra_export_functions`
or `ghidra_search_strings` job folds itself into the index before the call
returns, so **the moment an export finishes, its functions are queryable** and
the next agent's `ghidra_coverage` finds them.

The response carries an `index_refresh` object: `refreshed`/`queryable`,
`jobs_indexed`, `jobs_failed`, `seconds`. If `queryable` is false the evidence
still landed on disk — only the cache lagged — and the next export's refresh
picks it up, because each refresh is a full incremental pass rather than a
single-job one. Nothing is lost; a query may simply answer "not indexed" for a
short window.

The durable `job.json` receipt is unaffected by this: `index_refresh` appears in
the MCP response only, never in the evidence receipt that citations depend on.

### Storage note

Export artifacts are deduplicated (hardlinked) and LZX-compressed in place.
Compression is transparent — reads return identical bytes and hashes are
unchanged — so no tool needs special handling. Never rewrite artifacts inside a
completed job directory: many paths share one physical file, so an in-place edit
would corrupt every job that cites it. Derive new evidence into a new job.

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
