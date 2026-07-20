# Wave 2 exact-10 Ghidra re-export receipt

> **Historical receipt:** the ten-shard count below is an immutable observation
> of this completed replay, not a current topology requirement. New work uses
> the authorized 20-shard broker, and pool size alone does not validate or
> invalidate a job. See
> `replay_interface/tools/ghidra/CURRENT_AUTHORITY.md`.

The exact-10 authority replay is terminal: **43 root calls completed successfully** and **2 isolated roots reproduced the expected `0x140082cb0` decompiler failure**. There were no unexpected failures and the failed seed must not be retried.

- Binary: `hoi4.exe 1.19.2.0.a729`
- Image base: `0x140000000`
- SHA-256: `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`
- Pool: exactly 10 ready shards, read-only, analysis disabled, per-shard serialized
- Canonical action mappings: 122/122
- Supplemental action mappings: 24
- Actual executing shard for this lane: `shard-03` (planned shards were advisory only)

## Expected failures

- `EF-01-D0-P0`: depth 0, p-code disabled, `0x140082cb0`; deterministic decompile failure; no retry.
- `EF-02-D1-P1`: depth 1, p-code enabled, `0x140082cb0`; deterministic decompile failure; no retry.

Both surrounding clean remainders completed, isolating the failure to that seed. Promotion remains closed only for pending external consumer rewrites and any claim that requires the unavailable decompile, not for missing exact-10 replay work.

Machine authority: `WAVE2_EXACT10_REEXPORT_RECEIPT_20260720.json`.
