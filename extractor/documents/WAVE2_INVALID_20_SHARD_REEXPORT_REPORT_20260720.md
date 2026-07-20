# Wave 2 invalid-pool Ghidra re-export report

Date: 2026-07-20

Machine-readable authority: `WAVE2_INVALID_20_SHARD_REEXPORT_MANIFEST_20260720.json`

Machine-checkable consumer and batching reconciliation: `WAVE2_10_SHARD_REEXPORT_RECONCILIATION_20260720.json`

## Result

The retained Wave 2 pool produced 123 Ghidra job records after the first explicit Wave 2 job and before the pool became idle. All 123 records carry the expected `hoi4.exe 1.19.2.0.a729` identity, image base `0x140000000`, and SHA-256 `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`, but they were dispatched while the broker exposed a 20-shard pool. They are discovery evidence only and cannot be promotion authority.

The companion manifest inventories every attempt and gives one deterministic exact-10-shard replay queue. It contains 122 canonical actions because two independent lanes submitted the same semantic request. No other request was deduplicated.

## Census boundary

- Canonical root: `<local-path>`
- `C:\Users\Example\ghidra_agent_exports` is a junction to that root and is not a second evidence set.
- Inclusive first job: `20260719T131701Z-x2-wave2-terminal-loss-width-string-cens-edb8c4fe`
- Inclusive last job: `20260719T140325Z-weather-close-zoom-particle-packet-final-b3b95df5`
- Inclusion rule: every timestamped job in that interval, including jobs whose labels omitted `wave2`. This prevents silent loss of concurrent lane work.

## Inventory totals

| Operation/status | Attempts |
|---|---:|
| `dump_data` / complete | 30 |
| `export_functions` / complete | 67 |
| `export_functions` / failed | 3 |
| `search_strings` / complete | 23 |
| **Total** | **123** |

| Originating lane | Attempts |
|---|---:|
| A2/A12 focus, research, decisions | 5 |
| A28 dynamic collections | 2 |
| A6/A7 supply and convoy | 18 |
| A8/A9/B6 naval combat | 14 |
| C02/D77 camera envelope | 1 |
| C03 direction/action | 2 |
| CAP6 unit-counter rendering | 13 |
| D54 army battleplans | 4 |
| D81 exact rail replay state | 19 |
| G3 snow/mud pre-upload | 1 |
| Occupation/resistance/compliance | 8 |
| Project lifecycle/history | 13 |
| Weather close zoom | 2 |
| X2 land-combat residuals | 10 |
| X4 production priority | 9 |
| XUI21 owner classifier | 2 |

Sixty-four attempts are already cited by a repository promotion/evidence consumer. Fifty-nine are not yet cited outside the quarantine authority files. The latter remain in the replay queue: absence of a checked-in lane citation is not evidence that an agent did not use or intend to publish the retained result.

The consumer audit also corrected one stale convoy citation. The closeout had named nonexistent job `20260719T140201Z-cfexmember-client-handle-constructor-source-831f9d08` and a nonmatching hash. Retained job metadata identifies the actual job as `20260719T140201Z-convoy-client-handle-constructor-source-ba39dcc9`, with manifest SHA-256 `6C2D248E937D7E09314C05E093E91147DF740D6DBDBBC82B98C6C7470D98D5F6`.

## Exact 10-shard gate

Before action 1, the central reset must prove all of the following in one status observation:

1. Binary `hoi4.exe 1.19.2.0.a729`, image base `0x140000000`, SHA-256 `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.
2. Exactly 10 configured shards and exactly 10 ready shards.
3. Read-only operation, analysis disabled, and per-shard serialization enabled.
4. No persistent broker error.

The queue is ordered by `sequence` and carries a round-robin `target_shard_id` plan across `shard-01` through `shard-10` (13 hints each for shards 01 and 02, 12 each for shards 03 through 10). These values are advisory scheduling hints only. The broker owns actual scheduling and may return any ready shard. Each request preserves the original addresses, depth, p-code flag, dump parameters, or string terms. Labels are replaced only with deterministic `wave2-r10-NNN-*` labels; labels are not part of semantic request deduplication.

## Consolidated root-call plan

The reconciliation represents the 122 canonical semantic actions plus 24 supplemental exact-entry actions in 45 broker-valid root calls without merging incompatible semantics:

- Twelve successful canonical `export_functions` calls, split within the five exact `(depth, pcode)` groups so no call exceeds the broker maximum of 32 addresses.
- Two isolated expected-failure roots for `0x140082cb0`. Each required exact-ten retry was attempted once and failed deterministically; neither root is retried or allowed to poison a successful mixed batch.
- Three `dump_data` calls, split so no call exceeds the broker maximum of 128 addresses. Every source claim retains its exact membership list.
- One dedicated `SP-01` `export_functions` call for five lifecycle/reward-history entries from `SPECIAL_PROJECT_LIFECYCLE_REWARD_RE_20260719`, using depth 1 with p-code enabled.
- Four supplemental compatibility roots for 19 exact function entries omitted by the original request census: D81 rail owner/live-object/frame (7), X2 terminal-owner/width (4), convoy terminal route (2), weather named-entity registry/factory (4), GH04 per-state topology (1), and the naval activity discriminator (1).
- Twenty-three unchanged `search_strings` calls. String requests are not coalesced because this pass was authorized to consolidate compatible address requests only.

The address calls use deterministic chunk IDs under `ADDR-01` through `ADDR-06`; the unchanged four-address depth-2/p-code call remains `ADDR-05`. Supplemental calls are `SP-01` and `SUP-01-D0-P1` through `SUP-04-D2-P1`; deterministic failures are `EF-01-D0-P0` and `EF-02-D1-P1`; string calls remain `STR-001` through `STR-023`. Their round-robin `target_shard_id` values are planning hints, not an execution command or provenance claim. Every one of the 123 discovery claims maps to one canonical action, one root call, one advisory shard hint, and one replacement-result JSON pointer. The 24 supplemental actions remain outside the 122-action/123-claim discovery census. There are zero unmapped claims and zero unmapped canonical actions.

The original 60-address `ADDR-01` submission was rejected by input validation before the broker created a job. It is not a failed receipt and has no authority. `ADDR-01-01` and `ADDR-01-02` supersede that plan entry. The first exact-ten attempts for `ADDR-01-02` and `ADDR-03-01` then failed on their common `0x140082cb0` member. That member is now isolated in the two `EF-*` roots, while the 27- and 31-address clean remainders have new exact request hashes.

Claims with an external lane consumer must have each consumer rewritten to the new exact-10 result before promotion. Claims without an external lane consumer are consumed by the quarantine ledger itself and remain non-promotable until a lane adopts them. This distinction prevents inventory coverage from being mistaken for production authority.

## Broker receipt authority

The authoritative replacement identity is the broker-returned receipt, not `target_shard_id`. Each root-call receipt must bind all of these fields:

- Actual job ID and `job.json` path.
- Actual shard ID reported by the job/result record. It may differ from the advisory target.
- SHA-256 of the actual normalized semantic request.
- Exact binary identity and identity-manifest SHA-256.
- Artifact or function-manifest path and SHA-256.
- Exact-10 pool-status receipt and its SHA-256.

All 43 successful roots now carry those fields with `receipt_authoritative=true`. The two isolated `0x140082cb0` roots carry exact-ten pool, identity, job, request, shard, and deterministic error receipts with authority limited to the expected-failure outcome.

## Request/hash and consumer audit

Request identity uses SHA-256 over UTF-8 compact JSON with fixed key order. Labels are excluded. Search requests hash `terms`; data requests hash `addresses`; function exports hash `addresses`, `depth`, and `pcode`. Array order is preserved.

Each unbatched source-action hash identifies the exact original claim. Each consolidated root-call hash identifies the actual batched request submitted to the broker. For multi-action batches these hashes are intentionally different: promotion requires exact source membership in the batch plus an exact match between the broker receipt's request hash and the root-call hash. Treating source and batch hashes as interchangeable is forbidden.

The reconciliation records 64 external-consumer discovery-claim mappings plus 24 supplemental mappings across 33 distinct repository paths and 32 consolidated root calls. The central consumer rewrite ledger binds every path to the receipt's actual job ID, actual shard, request hash, job hash, artifact path/hash, identity-manifest hash, and pool-status hash. Planned shard IDs are never valid rewrite evidence. All 43 successful receipt slots and all central consumer bindings are closed. The two `EF-*` roots are terminal expected-failure receipts, not missing successful artifacts.

Exact-ten evidence-provenance replacement is admitted. This does not flip any lane activation latch or satisfy an independent runtime/human capture gate. The old artifacts remain discovery provenance inputs, not promotable outputs.

## Consumer rewrite result

`WAVE2_EXACT10_CONSUMER_REWRITE_LEDGER_20260720.json` is the machine-readable central replacement authority; its companion Markdown report provides the disposition table and exact root receipt index. The static pass covers all 33 mapped consumer files and 107 consumer/root edges:

- 7 consumers are enabled and now use the exact-ten receipts as replacement provenance.
- 5 consumers are pinned authority contracts that do not themselves activate a lane.
- 21 consumers remain deliberately false because their separate reader integration, runtime observation, controlled fixture, or human/live capture gate is still open.

None of the deliberately false dispositions is attributed to a missing exact-ten Ghidra receipt. All 107 mapped consumer/root edges contain their required local replacement binding: successful edges carry the actual replacement job ID and artifact SHA-256, while deterministic negative edges retain their complete failure receipt without fabricating an artifact. The central ledger hash-pins each complete consumer file and indexes the fuller job/shard/request/job/artifact/identity/pool binding for every edge.

## Failed requests retained and isolated

Three 20-shard attempts failed while decompiling `0x140082cb0`:

- `20260719T132149Z-convoy-save-schema-serializer-326c710f`: depth 1, p-code enabled.
- `20260719T132213Z-convoy-save-schema-serializer-no-pcode-9de6e1a6`: depth 0, p-code disabled.
- `20260719T133604Z-occupation-garrison-serializer-root-2026-7584d456`: depth 1, p-code enabled.

The first and third are the sole exact duplicate and map to one canonical replay action while retaining both lane aliases. The depth-0/no-p-code request remains separate. Both canonical retries were made under the exact-ten gate in mixed batches and failed again on the same function entry: job `20260719T220127Z-wave2-r10-addr-03-01-631965a4` for depth 1/p-code and job `20260719T220039Z-wave2-r10-addr-01-02-562658b0` for depth 0/no-p-code. Those failures are terminal for the required one-retry contract. Do not retry them, fabricate a body, or use them to suppress the successful members now isolated in the clean remainder roots.

## Supplemental seed census

The static dossier-to-request comparison is complete for the assigned production lanes:

- D81: all 15 required functions and 13 data addresses are now literal members; seven function entries required supplemental roots.
- X2 terminal/width: all named writer, allocator, owner-chain, producer, and mutable-leaf functions are now literal members; four required supplemental roots. The later width runtime fixture remains a separate human activation gate.
- Convoy: all 12 required functions and both vtable/data addresses are now literal members; two functions required supplementation.
- Weather entity registry: the named lookup, named factory, definition-backed factory, and definition-retention constructor are all direct depth-1/p-code members.
- GH04: `0x1411B2420` is now a direct depth-1/p-code member.
- Project lifecycle/reward: all 13 functions and both data addresses were already covered after `SP-01`; no further seed was added.
- CAP6: all six Ghidra jobs cited by the current portable-carrier dossier are represented; no additional seed was justified.
- S4: no Ghidra seed exists or is required; it is the offline publication/repack integrity gate.
- Naval: all four minimum discriminator functions are now literal members; `0x140D5A690` required supplementation.

## Human input

No human input is required to perform this re-export. The only prerequisite is the centrally controlled 10-shard reset and status proof. Any human-input requirements discovered by the lane work remain lane-specific and are not expanded by this inventory task.

## Scope note

This report and manifest were produced only from repository text, retained `job.json` metadata, and retained result metadata. No Ghidra operation, build, test, capture, or product-code edit was performed.
