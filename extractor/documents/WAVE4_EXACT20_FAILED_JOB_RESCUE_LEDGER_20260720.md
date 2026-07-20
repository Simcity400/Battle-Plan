# Wave 4 exact-20 failed-job rescue ledger

Status: **all six Wave 3 failures classified; replacement coverage hash-validated; no failed artifact promoted**.

The preflight for this rescue reported all 20 shards ready in read-only/no-analysis mode for `hoi4.exe 1.19.2.0.a729`, image base `0x140000000`, SHA-256 `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.

## Result

The six failures were not six missing semantic discoveries:

- Three A28 requests reached `EXTERNAL:00000124` while expanding callees. The exact roots are present in the successful depth-zero manifest; 22 of 23 broad-request roots also have successful split depth-one manifests. The remaining constructor at `0x141f04ea0` has a complete direct function receipt, and Wave 4 job `20260720T044447Z-w4-rescue-a28-mio1-external-edges-39275c55` binds its two external call sites and IAT entries as exact bytes. It does not pretend that imported Win32 routines are project-image functions.
- Two A40 requests mixed defined function entries with internal code labels at `0x141490ea0` and `0x1417506d0`. The corrected request exported all ten defined roots, while the two labels have exact byte receipts.
- The A29 callback supplement similarly supplied three internal labels as function seeds. Its exact byte dump is the correct evidence type and makes no standalone-function claim.

Every replacement artifact hash was recomputed from disk and matched the broker-reported hash. The machine-readable ledger contains the job, shard, job JSON, artifact, identity-manifest, and failed-to-replacement relationships.

One attempted all-internal-callees diagnostic was deliberately stopped after the depth-zero exporter expanded common allocator/TLS helpers into more than 20,000 direct-caller files (115.04 MiB). Job `20260720T044623Z-w4-rescue-a28-mio1-internal-callees-13f7051e` has no complete manifest and no authority. This is retained as a negative tool-scaling result; none of its partial files are used.

## Promotion boundary

Failed jobs remain negative diagnostics and contribute no artifact authority. The replacement receipts close only static custody for the requested code or data locations. This rescue enables no live source, runtime schedule, collection completeness claim, or activation latch, and it creates no new human-input requirement.

Machine-readable receipt: `extractor/documents/WAVE4_EXACT20_FAILED_JOB_RESCUE_LEDGER_20260720.json`.
