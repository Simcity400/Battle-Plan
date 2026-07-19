# XN16M — post-capture merge and candidate refreeze

## Disposition

**Current production disposition: PROVEN SYNCHRONIZED PACKAGE ADMITTED BY XN16Q.** The two-result merger remains historical-target tooling. Refreeze receipt `0557e9f87ca93ce5a04726294bc1717acf72afd19b7fdc70bd455fe82fd5332b`; verification `68409cbd92453c520929720d8ec6095154ac33af1fba4d1f42c8a5e5f1b67662` (88/88); manifest `aa7b15297e672d2fa651eb22e6d0ddbca6d226090d81e5b6a92e77c140b5b480`. Ghidra function/job/artifact/manifest: `null`.

**FROZEN FAIL-CLOSED — WAITING FOR TWO EXACT ADMISSION RESULTS.** XN16M is ready to consume the independent W2 synchronized `trees_measure` result and independent R2 exact `FINAL_full` rivers-pass-close result. Neither currently exists in this packet, so the verified current outcome is rejection with frozen XN16 preserved.

## Promotion boundary

`merge_xn16_captures.py` does not run XN16H or either per-shot validator. It accepts the two already-produced JSON results, verifies exact schemas, success statuses, frozen validator-contract identities, complete passing check sets, and source-local capture roots, then re-hashes the admitted capture bytes.

For W2, every `verified_local_files` entry is rebound to the source round, and the evidence-manifest/production-verification relationship and executable identity are rechecked. For R2, terrain, water, river, and post-pass RTV hashes are rebound. R2 v1 does not emit session/event hashes, so XN16M additionally hashes those files and rechecks the rivers captured state, incomplete/complete/advance order, draw count, and absence of a pre-close rivers budget failure. This closes the stale-result gap without changing or invoking R2.

Admission also requires distinct source roots, exact frozen XN16/W2/R2/P dependency hashes, and the existing PAN2 clear-provenance and PAN_chunks readable-water gates to remain closed. Success atomically emits a candidate disposition `PROMOTION_READY_EXACT_CAPTURE_GATES_CLOSED`; it still does not modify XN16. Missing, inferred, tuned, partial, cross-camera, cross-frame, capture-local ResourceId, pointer, or view-handle substitutions are rejected.

## Current mechanical result

The verifier uses genuinely absent paths under `XN16M/current_input`; it creates no mock admission or capture evidence. The merge exits 2, atomically emits `REJECT_MISSING_OR_INVALID_EXACT_CAPTURE_ADMISSIONS`, records both missing inputs, makes no capture claim, and preserves `HUMAN_CAPTURE_REQUIRED_EXACT_TWO_SHOTS_PROVEN_ABSENT`. Frozen XN16 hashes are checked before and after execution.

## Incidental findings retained

- XN16H's combined result is orchestration only and is not XN16M admission authority.
- W2 emits a full verified-local-file map suitable for byte rebinding.
- R2 v1 omits session/event hashes; XN16M binds and rechecks those bytes before compose promotion.
- No W2 or R2 success output exists; this is a capture-input condition, not a tool block or partial finding.

No `PROGRAM_STATUS`, collector source, frozen XN16 artifact, or adversarial-review surface was edited.

## Ghidra provenance

No new binary claim was made. The manifest records null function address, job ID, artifact path, and manifest hash with the offline post-capture merge reason.
