# A20 owner-joined project progress production closeout

Production promotion additionally requires the terminal D41 `PPA1` v1 outer
seal. It binds the pre-seal payload hash, record sequence/hour, status/counts,
exact a729 executable, reviewed A20/X9 receipts, and the same-hour D18 roster
digest. Legacy inner-only `PPGF` stays readable but is never exact.

Date: 2026-07-20

## Result

A20 project-progress counters now have an unambiguous production carrier and a
fail-closed replay/UI admission path. The stable identity is
`(tag_db_id, local_id)`, not the bare `CProject` local ID.

The collector appends `PPGF` version 1 after `PLCF` and before `PRHF`. Its
20-byte header carries row count, successfully walked country count, and
country-atomic abort count. Each 52-byte row carries the owner/local key plus
the exact a729 project state-machine fields:

- static definition handle;
- current state offset and the four state enums;
- prototype iteration count;
- raw phase-progress numerator and raw phase target;
- completed bit.

Delta tombstones retain only the owner/local key and bit 31. The recorder
authority decoder, world fold, typed replay view, game-UI admission, and replay
app selected-country/global seam all reject malformed, duplicate,
out-of-bounds, or incomplete observations.

## Ambiguous legacy rows removed from production emission

The old base D41 project and project-state rows were keyed only by `local_id`.
That ID is allocated inside a country project pool and can collide between
countries. Production collection no longer emits those two ambiguous base-row
sets. Lifecycle, owner-joined progress, and reward-history carriers remain
keyed by owner and local ID. Older recordings can still be structurally
decoded, but the exact typed project-progress view admits only `PPGF` authority.

## Live read boundary

The owner join is performed during the country walk:

`CCountry + 0xFA8 -> CProgramStatus + 0x18 -> CProjectPool + 0x10`

Every project is pinned and its state machine is read at that same country
boundary. If any project/state row is invalid or torn, the entire country is
aborted for this observation. A nonzero `aborted_countries` value makes the
typed replay and UI views unavailable; no partial country set is presented as
exact.

The UI receives the raw phase numerator and raw target. No percentage, scale,
normalization, interpolation, or display rounding is inferred because no
retained engine evidence proves such a transform.

## Evidence disposition

The required preflight was confirmed against the active exact-a729 broker:
20 ready read-only/no-analysis shards, image base `0x140000000`, and executable
SHA-256 `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.
Existing retained a729 field/writer evidence was sufficient; no new
decompilation was required.

The retained 423-hour Evidence closure is a d24 weather capture. It does not
contain D41 project-progress proof and is deliberately not cited as A20 live
evidence.

## Residuals and validation

No human input is required for the exact raw project-progress counter path.
Project display-name resolution and X9 specialization semantics are separate
lanes and are not inferred here.

Per repository policy, this subagent performed no build, test, formatter, or
verifier run. Central Rust validation remains for the primary agent after the
wave is merged.
