# Indexed the game Ghidra evidence

## Grounded constraints

The live exact-a729 broker is a 20-shard, read-only/no-analysis authority rooted at
`<local-path>`; the export store is
`<local-path>`. The legacy paths in `.mcp.json` are
junctions, not independent stores. Existing tool limits and evidence layouts are
unchanged.

`FunctionEvidence.java` does more than a callee closure: for every requested seed
it exports the seed's direct callers as well. This is why real depth-zero jobs can
contain hundreds or thousands of functions. Graph records have three exact
sections (`CALLERS`, `NON_CALL_XREFS_TO_ENTRY`, `CALLEES`); callable endpoints are
either an internal hexadecimal address or an address-space location such as
`EXTERNAL:00000107` after the final `@`, while caller rows additionally carry
`from=` and `type=` fields. `NO_FUNCTION`, decorated namespaces, thunks, and
self-recursion are observed. External and still-unknown forms are recorded with
counters and never make the containing job fail.

## Cache schema

`function_index.sqlite3` uses WAL mode and contains:

- `indexed_jobs` and `indexed_string_jobs`: content-hash receipts that make scans
  incremental and resumable;
- `artifacts`: one canonical `(address, kind)` row with content SHA-256, size,
  evidence-relative path, and builder receipt timestamps;
- `artifact_provenance`: every job/path occurrence, retaining legacy citations;
- `artifact_conflicts`: alternate content hashes for the same address/kind rather
  than silently replacing the first indexed artifact;
- `functions`: names and body bounds parsed from graph headers;
- `call_edges` and `call_edge_sources`: deduplicated caller/callee relationships
  plus the graph artifacts that prove them;
- `external_call_endpoints` and `skipped_graph_endpoints`: non-internal endpoint
  observations and total-parser diagnostics;
- `decompile_fts`: FTS5 content for only the canonical decompile artifact per
  function;
- `string_xrefs`: terms, occurrences, reference sites, and referenced functions
  parsed from real `string-xrefs.txt` jobs;
- `function_tags`: curated-address, decompile-pattern, and string-xref-derived
  subsystem tags;
- `meta`: schema/build timestamps, store snapshot, and indexed counts.

The builder hashes every admitted artifact in a private generation database,
validates the manifest sidecar plus the durable `job.json` result/identity, and
publishes the completed snapshot atomically using SQLite backup in WAL mode. An
already-running reader retains the prior complete snapshot. A rerun rehashes
small manifests/job metadata and skips unchanged receipts. It never uses
link counts, compression state, or artifact mtimes as identity. Read failures or
files replaced during the concurrent dedup pass leave the job unreceipted so a
later run retries it. `--rebuild`, `--verify`, and bounded `--job` fixture/repair
selection are operational options, not MCP limits.

## Additive MCP tools

- `ghidra_get_function(address, kinds, max_chars)` returns advisory indexed
  metadata, citations, provenance jobs, claims, tags, and a bounded direct read
  of canonical artifact content. It does not hash or re-derive the content.
- `ghidra_search_decompiled(query, limit, address_min, address_max, subsystem)`
  returns ranked FTS candidates with SQL `LIMIT` and indexed citations.
- `ghidra_verify(addresses, kinds)` is the explicit opt-in disk check. For every
  requested artifact it reports indexed/actual hash and size, `agrees`, the full
  citation, and an error instead of raising when unreadable. A disagreeing
  canonical copy triggers checks of other indexed provenance copies.
- `ghidra_callers(address, depth)`, `ghidra_callees(address, depth)`, and
  `ghidra_path_between(from, to, max_depth)` traverse indexed edges and return
  their indexed graph citations without reparsing artifacts.
- `ghidra_coverage(addresses)` reports indexed kinds/jobs, claims, tags, and
  citations per address.
- `ghidra_index_status()` reports counts, build metadata, and conflicts without
  launching Ghidra or comparing the cache with the export store.
- `ghidra_functions_for(subsystem)` and `ghidra_subsystems_of(address)` expose the
  curated/derived tags.
- `ghidra_annotate_function(address, note, evidence_refs)` appends a `claims` row
  in the existing `hoi4_re` WAL registry. It does not create a second findings
  store or mutate prior claims. Every reference must identify an existing
  completed-job artifact by job-relative path and matching SHA-256.

New schemas deliberately avoid arbitrary result caps. Existing limits remain
exactly 1..32 addresses/depth 0..3 for exports, 1..128 for data dumps, and the
existing read/list limits.

## Advisory query contract

The index is never evidence authority and ordinary queries never claim
verification. They report what the last completed generation indexed, without
FTS expression re-evaluation, snippet confirmation, graph-edge/header reparsing,
tag confirmation, curation staleness checks, or cache-versus-disk adjudication.
Every artifact-bearing result carries `job_id`, `job_dir`,
`artifact_relative_path`, `canonical_path`, `path`, `indexed_sha256`, and
`indexed_bytes`; the prefix explicitly identifies build-time recorded identity.

`ghidra_verify` is the only cache-versus-disk comparison path. Agreement confirms
artifact identity, not the caller's semantic interpretation. Findings packets
must cite the artifact path and indexed hash, invoke `ghidra_verify`, and record
the actual identity before publishing a claim. Missing/corrupt/locked databases
degrade query tools to an explicit unavailable response and never affect
existing Ghidra export tools.

## Citation-safe reuse

Reuse is off unless `BATTLE_PLAN_GHIDRA_REUSE=1`, and an export request can additionally
set `no_reuse=true`. The broker writes a tab-separated reuse plan outside the
evidence directory from currently indexed artifacts. During the normal Ghidra
closure walk, `FunctionEvidence.java` considers each exact artifact path, rehashes
the proposed source, and hardlinks it into the new evidence directory only when
path, size, and SHA-256 all match. Any missing index, stale row, hash mismatch,
cross-volume hardlink failure, or race falls back to normal generation for that
artifact.

The Java exporter still owns seed resolution, closure membership, ordering, and
manifest generation. It rehashes every linked/generated destination and emits the
same sorted v1 manifest and sidecar bytes a full export would emit. Reuse stats
are written outside `evidence` and copied into `job.json` only when reuse is
enabled or explicitly controlled; no old job or artifact is modified. With the
gate off and `no_reuse` absent, request/response, `job.json`, and evidence layout
retain the exact master shape with no reuse fields.

## Rollout order

1. Merge/deploy code with reuse disabled.
2. Build the index against the real export root, rerun to prove incremental no-op
   behavior, exercise advisory queries, and confirm selected artifacts with
   `ghidra_verify`.
3. Point the MCP environment directly at the real store/index/registry paths and
   restart the server; the README contains the proposed JSON block.
4. Validate semantic tags and append a disposable test annotation only if the
   owner authorizes a registry write.
5. Wait for the separate dedup/LZX pass to finish and receive orchestrator
   confirmation.
6. Run gated synthetic/canary exports with `BATTLE_PLAN_GHIDRA_REUSE=1`, compare complete
   manifests against reuse-disabled exports, then enable the gate in production.
