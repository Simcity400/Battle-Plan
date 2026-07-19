# XN16D - operational preflight for the XN16 two-shot capture

## Disposition

**HISTORICAL TWO-SHOT PREFLIGHT RETAINED; LIVE PRODUCTION PACKAGE ADMITTED BY XN16Q.** The completed synchronized capture supersedes this dry-run as production admission evidence; the preflight remains useful provenance.

Production refreeze receipt SHA-256 is `0169093fd060bebc2cd29941f081225b943be216d7555961e84e39594f6dbd8e`; shared verification is `68409cbd92453c520929720d8ec6095154ac33af1fba4d1f42c8a5e5f1b67662` (88/88), manifest `aa7b15297e672d2fa651eb22e6d0ddbca6d226090d81e5b6a92e77c140b5b480`. Function address, job ID, artifact path, and Ghidra manifest hash are `null`.

## Verified operational boundary

- Authority is exact `hoi4.exe 1.19.2.0.a729`, image base `0x140000000`, executable SHA-256 `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.
- XN16H is pinned through its report, atomic contract, validator, verification and final manifest. XN16S is pinned through its report, recipe, verifier, verification and manifest. The only recipe references consumed here are `XN16S-R1` and `XN16S-R2`; their scene content remains owned by XN16S.
- Both shots use suspended early injection with no active `hoi4.exe`, no live-PID fallback, and XN16S's immediate root sequence. Map-frame rounds, arm delay, camera/console injection and chunk capture are forbidden. The exact four moments are terrain, water, rivers and trees; `trees_close_mode` is `frame-repeat` as frozen by XN16S.
- Native caps are exact: 8,192 draws per pass, 8,589,934,592 payload bytes per session, 30,000 ms capture work per frame, 1,073,741,824 queued writer bytes, 8,000 ms settle, 30,000 ms pre-resume timeout and 240,000 ms dwell under the 600,000 ms hard cap. Any budget event, missing sequence-complete event, non-`pass_close` finalization or incomplete ordered stream rejects the attempt.
- RenderDoc is version 1.45 (`2fc0bc04cb95499635f63986a55bc6f67849dd9f`); `renderdoc.dll` SHA-256 is `313f450bfa1f7f7d5f50cc7ee80d9f99b68d3fe873e25eb865ff0433e359f7b3`. External `renderdoccmd capture`, qrenderdoc attach and `capture_vanilla.ps1` hook stacking are forbidden. If retained, one RDC is armed through authenticated in-process API 1.7 from the accepted update to the next Present. RDC identity is supplementary; XN16H admission remains the exact source-local native bundles.
- Output is restricted to new direct children of `C:\tmp\hoi4_rendering_law_capture` on NTFS, outside OneDrive and disjoint from retained inputs. Production names are `<UTC>-xn16w2-trees-measure` and `<UTC>-xn16r2-final-full`. Each attempt has isolated `stage`, `gpu`, `rdc`, `context` and `admission` directories plus both controls, append-only attempt ledger, preflight/context receipts and a content manifest.
- Required free space is evidence-derived, not tuned: `2 * (8,589,934,592 native bytes + 1,238,120,569 maximum retained RDC bytes) = 19,656,110,322`. Both repeated dry runs proved live free space met or exceeded that threshold. Volatile free-byte counts and process IDs are intentionally not persisted in the frozen result.
- Existing capture directories are never reused or overwritten. Recursive cleanup is forbidden. A failed attempt is retained and manifested; a successful source becomes immutable after its manifest. Combined admission runs only after both independent source manifests and component admissions exist.

## Mechanical result

`validate_preflight.py` passes 132/132 checks twice with `PASS_OPERATIONAL_DRY_RUN_READY`. The two persisted dry-run results are byte-identical. `verify_xn16d.py` passes 27/27 with `PASS_OPERATIONAL_PREFLIGHT_FROZEN`. Before and after both runs, the two reserved `XN16D_DRYRUN_DO_NOT_CAPTURE_*` directories did not exist. Both runs observed zero the game processes and zero external RenderDoc processes and performed no launch, attach, injection, RenderDoc arm, capture-directory creation, deletion or overwrite.

- Plan: `ad555b9432ceeec8f8b60b1102c00db880f2626c7930a20a75f9f4743ed9f882`.
- Contract: `a4a1082f28a4215466937a4ad3686c0d8603748c71578c2cb85bc9fb592ce3b0`.
- Validator: `837516b30ae7a9d643ff6ff06dd7552c3db9d2763f57cf7fd6088917184cf0d2`.
- Verification: `60eeb18d507e0ac658f835fb7554514112fefa2d20c8f7ea3ac9463562133a28`.
- Isolated current-source build: helper `5a3f9bf139e64d9fe76241862192b73f0015e8e7c9ce72388537dbe4dae40b8a`; DLL `b20d1cd58841ff52fd2d548ca23227fa21315e56a64b7ebe4c6faae0d3128857`.

## Incidental findings retained

1. The pre-existing default target DLL was older than the current RenderDoc integration source. XN16D therefore built the unchanged crate in isolated `target/xn16d`, pinned every Rust/Cargo build input, and verified required CLI/RenderDoc/budget strings in the resulting binaries.
2. The attach helper clears `terrain_capture_events.jsonl` before injection. Reusing an attempt directory could destroy evidence before the game starts, so new, absent attempt roots are a terminal preflight requirement rather than an advisory.
3. Mechanical pinning corrected a prior handoff typo: the unchanged XN16H final manifest is `5fbb7590c722b1d1a1a25bfbcd2ff4f4992a4e25c54ee413426a72490c2d107d` (`...bfbcd2...`).

## Provenance

This is an offline host/tool preflight. Ghidra function address: **N/A**. Ghidra job ID: **N/A**. Ghidra artifact path: **N/A**. Frozen artifact manifest: `replay_interface/re_evidence/rendering_laws/XN16D/artifact_manifest.json`, SHA-256 `3adde63d23c3f4c783e7162a9007dae7664b683dfb04a4eca33d72f26c810cc0`.

No harness production source, `PROGRAM_STATUS.md`, collector, renderer, extractor or upstream XN16 packet was edited. No adversarial review was performed.
