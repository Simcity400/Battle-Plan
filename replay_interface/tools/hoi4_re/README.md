# Agent-native HOI4 reverse-engineering facade

`hoi4_re` is the single agent-facing entry point for cross-build migration,
shared symbol/evidence lookup, bounded runtime observation, leaf-function
emulation, and save/memory correlation. It is a local stdio MCP server and has
five visible tools:

- `re_status`
- `re_query`
- `re_run`
- `re_list_jobs`
- `re_read_artifact`

All work is represented as durable jobs below the machine-local state root.
The default is `<local-path>`; override it with
`BATTLE_PLAN_RE_STATE_ROOT`. The SQLite registry uses WAL mode, permits concurrent
readers, and serializes writes. Agents never edit the database directly.

## Lean whole-game toolchain

Agents use this MCP service rather than coordinating separate GUI projects.
The installed stack is deliberately limited to tools with distinct jobs:

- Ghidra 12.1.2: shared exact-build static authority (20 serialized shards).
- angr 9.3.2: an independent decompiler and symbolic-analysis second opinion.
- Rizin 0.8.2: fast independent disassembly and basic-block recovery.
- Frida 17.15.3: bounded call observation through `trace_calls`.
- Unicorn 2.1.4: leaf-function emulation through `emulate_function`.
- DynamoRIO 11.91 with drcov: coverage capture for a dedicated live lane.
- WinDbg with TTD: last-resort time-travel investigation in that live lane.

There is no Cutter, Triton, capa, second registry, or second MCP service. Four
offline `cross_analyze_function` jobs may run at once; further jobs wait for a
slot in one fixed machine-wide lock namespace; state-root overrides cannot add
capacity. Live work remains one-at-a-time because multiple instrumenters on one game
process corrupt the observation rather than increasing throughput. `re_status`
reports every installed path and readiness check, so agents do not need to know
machine-specific install directories.

`candidate_census` accounts for compiler-described x64 functions against the
shared Ghidra index and emits a compact missing-RVA queue. It refuses an index
without a published generation and matching executable SHA-256/image base. It is a work queue,
not a claim that every entry is a unique gameplay routine or understood.
`cross_analyze_function` accepts either a stable symbol ID or a raw RVA and
writes isolated angr and Rizin artifacts from a content-addressed executable
snapshot. It never replaces Ghidra evidence or
automatically promotes a reconstruction claim.

```json
{"operation":"candidate_census","inputs":{"build_id":"hoi4-exe-1-19-2-0-a729-870ebf1f6152"}}
```

```json
{
  "operation":"cross_analyze_function",
  "inputs":{
    "build_id":"hoi4-exe-1-19-2-0-a729-870ebf1f6152",
    "rva":"0x24de600",
    "engines":["angr","rizin"],
    "timeout_s":180
  }
}
```

The current machine uses Python 3.12 at
`<local-path>`. To recreate only that isolated component:

```powershell
py -3.12 -m venv <local-path>
<local-path> -m pip install --requirement replay_interface\tools\hoi4_re\requirements-angr.txt
```

Tool and evidence paths have `BATTLE_PLAN_RE_*` overrides in `Config.from_env`; the
four-slot machine lock intentionally does not. The default service configuration
needs no per-agent setup.

The exact-a729 Ghidra broker also reads function-relevant claims from this same
registry. `ghidra_get_function` and `ghidra_coverage` surface claims whose subject
or value names the exact VA, RVA, native logical symbol, or function name.
`ghidra_annotate_function` appends a `status=proposed`, `predicate=annotation`
claim through `Registry.record_function_annotation`; it does not create a second
notes database and never updates or deletes an older claim. Evidence references
must name an existing completed Ghidra job, a job-relative artifact path, and the
artifact's matching SHA-256. Validated references are retained in the claim value
and job IDs are also populated in the existing claim column. Claim lookup uses
exact address/token boundaries and isolates build-specific claims.

## Relationship to the exact-a729 broker

The existing `hoi4_ghidra` server remains the sole supported entry point for
exact-a729 static exports. This facade does not import scripts into, analyze, or
open any current authority shard read-write. `ingest_ghidra_jobs` verifies and
indexes immutable function-export artifacts already emitted by that broker.

`inventory_build` is for patch migration. It creates a separate writable Ghidra
project under the facade state root, validates the registered executable hash,
and emits a function fingerprint inventory using `BuildInventory.java`.

## Patch-day workflow

The normal path is one durable operation after the old build has been indexed:

```json
{
  "operation": "migrate_build",
  "inputs": {
    "source_build": "...",
    "target_exe_path": "D:\\archived-builds\\new\\hoi4.exe"
  }
}
```

It registers and hashes the target, builds its isolated Ghidra inventory, diffs
it against the source, and emits both `migration_diff.json` and
`compatibility_pack.draft.json`. The pack stays a draft until changed and
critical symbols have fresh evidence.

1. `re_status`.
2. `re_run(operation="register_build", inputs={"exe_path": ...})`.
3. `re_run(operation="inventory_build", inputs={"build_id": ...})` for the
   old and new registered binaries. The existing exact-a729 job corpus may also
   be bootstrapped with `ingest_ghidra_jobs`, but it is a partial inventory.
4. `re_run(operation="diff_builds", inputs={"source_build": ..., "target_build": ...})`.
5. Read `result.json` from the diff job and work only the `changed_candidate`,
   `ambiguous`, and `unmatched` queue.
6. Record fresh evidence and claims. A diff never promotes a build by itself.

Function matching uses, in order, unique byte identity, unique normalized
instruction shape, then a bounded fuzzy instruction-shape comparison. Stable
logical symbol IDs are carried to exact and high-confidence target instances.

The initial historical-corpus ingest verifies every manifest artifact. If that
job reports zero invalid jobs, setup seals its incremental cache with the same
operation and `{"seed_receipts_from_job":"<job id>"}`; later ingests then skip
unchanged manifests instead of hashing the entire corpus again.

## Live observation

Raw arbitrary memory reads are deliberately not exposed. Register a typed,
build-specific path, then observe it:

```json
{
  "operation": "register_memory_path",
  "inputs": {
    "path_id": "game-date",
    "build_id": "...",
    "base_rva": "0x3316e90",
    "offsets": ["0x468"],
    "deref_base": true,
    "value_type": "i32",
    "description": "CGameState current date"
  }
}
```

`observe_runtime` uses only `PROCESS_VM_READ | PROCESS_QUERY_INFORMATION` and
refuses an executable SHA mismatch. `trace_calls` is different: Frida
`Interceptor` is hook-based observational instrumentation. It is capped at 32
symbols, 30 seconds, and 64 hits per target, holds an exclusive machine lease,
has no force override, and treats cleanup failure as a tainted session.
Each call-entry row also records a guarded `this`-receiver vtable pointer and
module-relative vtable RVA when `RCX` is a readable C++ object. This lets agents
separate shared thunks by exact runtime type without exposing arbitrary memory
reads or treating the trace as promotion authority.
`trace_calls` accepts an optional `receiver_vtable_rvas` object keyed by a
requested symbol ID; each value is a bounded list of exact module-relative
vtable RVAs. Nonmatching calls are ignored before the per-target hit cap, which
keeps a frequent shared thunk from starving the exact owner type under study.
An optional `receiver_qword_offsets` object can capture at most 32 aligned
receiver fields per symbol (bounded to the first 64 KiB), including a guarded
pointee-vtable RVA. This is intended for statically proved object layouts such
as owner-to-runtime-root joins; the configured offsets are retained in the job
request and the trace remains discovery-only.
`receiver_pointee_qword_offsets` follows at most eight explicitly named owner
fields and reads at most 64 aligned qwords from each pointee. The request is
globally capped at 4,096 qwords, allowing bounded owner-to-runtime-object graph
joins without enabling arbitrary unretained process-memory scans.
`receiver_pointer_chains` follows at most eight explicitly named pointer chains,
with no more than four aligned dereferences per chain, then reads at most 64
selected qwords from the final object. The requested steps and guarded results
are retained, and chain qword reads have their own 4,096-qword trace budget.
`skip_hits` can retain a later bounded call window for a requested symbol. It
is applied after receiver-type filtering and before the existing hit cap, so a
stable traversal order does not limit observation to the first objects in a
large exact-type pool.
If Frida cannot instrument one registered target, the trace now retains a
`target_error` row for that symbol and continues attaching the remaining
bounded targets instead of losing the rest of a multi-symbol census.
For statically proved container ABIs, `argument_pointer_arrays` can capture a
bounded pointer array rooted at `RCX`, `RDX`, `R8`, or `R9`, together with exact
item vtables, selected aligned item qwords, and selected vtable-slot targets.
Item qword reads are separately capped at 8,192 qwords per trace request. Array reads are capped at 4,096
items each and 8,192 items for the whole trace request, and every register,
offset, count bound, and requested slot is retained in the durable job request.
`start_index` permits a later bounded window when a proved container exceeds
the per-array cap; emitted item indexes remain indexes in the original array.

Do not launch or attach to HOI4 merely to smoke-test this server. Live jobs need
explicit task authorization and an already-running matching process.

## Emulation and correlation

`emulate_function` admits indexed leaf functions only. It maps the registered
PE at its preferred image base, applies a fixture, and records registers and
watched memory. Imports, TLS, engine lifecycle, and functions containing calls
remain rejected in this first contract.

`correlate_save_memory` accepts inline JSON, workspace/state-root JSON paths,
`{job_id, path}` artifact references, or `.hoi4` files below the configured save
root. Binary saves are decoded to JSON with Rakaly inside a temporary
directory; the large intermediate is deleted, while source hashes and the
Rakaly version are retained in the job result. It ranks identical before/after
pairs and numeric deltas. Results are discovery only; static writer and live
timing evidence are still required.

## Offline validation

```powershell
python -m unittest replay_interface\tools\hoi4_re\test_server.py -v
```

The test suite uses synthetic Ghidra artifacts, build inventories, memory/save
snapshots, and x86-64 code. It does not launch or attach to HOI4.
