# XN16F — final XN16 capture-readiness compose gate

## Disposition

**PRODUCTION ADMITTED - SYNCHRONIZED FOUR-PASS PACKAGE VERIFIED.** XN16F retains H/S/D/C/M as historical-target infrastructure and admits production through the explicit XN16Q refreeze.

Refreeze receipt `6e4e229e223c285a2400d3a9999523c1d12602ad2384cd6deb4b79a0c8afe2d0`; verification `68409cbd92453c520929720d8ec6095154ac33af1fba4d1f42c8a5e5f1b67662` (88/88); manifest `aa7b15297e672d2fa651eb22e6d0ddbca6d226090d81e5b6a92e77c140b5b480`. Production package manifest is `09c07387a50df6a8756319c4951f03c4a0b1125cd2e0a1f64c579a1e3e8efceb`; final target FNV-1a64 is `3b3e6af22757b767`. Ghidra function/job/artifact/manifest: `null`.

## Frozen compose

- XN16H supplies atomic two-capture execution and independent W2/R2 admission orchestration.
- XN16S supplies the exact scene/camera preparation recipe.
- XN16D supplies the corrected, operational dry-run harness preflight.
- XN16C supplies canonical content-addressed preservation of admitted capture evidence.
- XN16M supplies post-capture byte rebinding, merge, and candidate refreeze.

`compose_xn16f.py` hashes every file in the five root-approved tuples and checks each packet's frozen semantic status. It invokes none of the component tools and duplicates none of their validation logic. The compose result is written atomically.

All component tuples are ready, but `current_input/trees_measure_admission.json` and `current_input/final_full_admission.json` are genuinely absent. The result therefore separates `READY_EXACT_TWO_CAPTURE_PIPELINE` from the final fail-closed disposition `HUMAN_CAPTURE_REQUIRED_EXACT_TWO_SHOTS`; `production_admitted` remains false.

## Rejected history and incidental findings

- The first posted XN16D tuple is explicitly non-authoritative because root reruns exposed non-idempotent dry-run/verification bytes and stale manifest coverage.
- Only the corrected XN16D tuple, verified by two full byte-identical root cycles, is admitted.
- XN16H readiness is not capture evidence, XN16C's frozen importer is not a current import, and XN16M's frozen merger is not a current refreeze.
- Component readiness and production promotion are distinct gates; completing the former cannot silently clear the latter.

No frozen XN16 artifact, `PROGRAM_STATUS`, collector source, or adversarial-review surface was edited.

## Ghidra provenance

No new binary claim was made. The manifest records null function address, job ID, artifact path, and manifest hash with the offline readiness-compose reason.
