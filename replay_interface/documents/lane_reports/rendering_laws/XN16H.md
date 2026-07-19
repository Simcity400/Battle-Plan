# XN16H - atomic execution pack for the two proven-absent XN16 captures

## Disposition

**HISTORICAL TARGET-SPECIFIC GATE RETAINED; PRODUCTION PROMOTION ADMITTED BY XN16Q.** XN16W2/XN16R2 remain authoritative only for reproducing their exact historical scenes. They are no longer production-frame prerequisites.

Production refreeze receipt SHA-256 is `ae94496fc987e8090ff4e75a59fdda17a28f08842053ee67723f5c24b9880ab6`; shared verification is `68409cbd92453c520929720d8ec6095154ac33af1fba4d1f42c8a5e5f1b67662` (88/88), manifest `aa7b15297e672d2fa651eb22e6d0ddbca6d226090d81e5b6a92e77c140b5b480`. Function address, job ID, artifact path, and Ghidra manifest hash are `null`.

## Minimal execution set

1. Capture one clean `trees_measure` terrain -> water -> rivers -> trees round. Terrain/water must authenticate scene CB0 `619059c647782be4`; rivers/trees must authenticate `9c2d9a7336c291e4`. All four source-local passes, payload bytes, hashes, manifest rows and ordered events must satisfy XN16W2.
2. Capture the exact `FINAL_full` rivers pass through pass close. Its river scene CB0 must be `5866dccf4c7cb126`, its frozen terrain/water anchors and first river draw signature must match, and its engine post-pass RTV must satisfy XN16R2.

These are necessarily two shots. The `FINAL_full` river CB identity differs from both retained `trees_measure` identities, so one reusable frame cannot authenticate both gates. Capture-local RenderDoc `ResourceId`, pointer and view identities may not cross between them.

Run the single command in `XN16H/CAPTURE_EXECUTION_PACK.md` only after both disjoint source directories exist. Promotion evidence exists only when the process exits 0 and `xn16h_combined_admission.json` reports `ADMIT_XN16_TWO_CAPTURE_SET`. Each component must independently return its exact schema and success status. One success plus one missing or rejected component is `REJECT`.

## Mechanical verification

`verify_xn16h.py` passes 35/35 checks with status `PASS_TWO_CAPTURE_EXECUTION_PACK_FROZEN`. It pins the frozen XN16 integration and every XN16W2/XN16R2 report, contract, validator, manifest and verification hash. It then runs the combined gate against the exact-a729 wrong-camera replacement and the incomplete retained `live_FINAL_full` directory. Both component validators reject, the combined process exits 2, and no atomic admission or capture claim is emitted.

- Frozen XN16 integration: report `50f85f80c0f903e975874d286af6f6919a529347b08c912a21c187cb5610cada`; contract `db545b697db79af2dd407d70b44c7a3ffd7c24257c9910f050deb6ced25d529c`; manifest `8f536ec3125d86e9107f8382bea2f151d3579474d8d780fb3cc91bd9a5382984`; verification `eb610d42d32216215e9052734d69ab0fc3b841bcc88a2b09239bfc44f86b5670`.
- XN16W2: report `aacde53386fb79b3806b8d9995c6232a89cad5384aab10eed1cf3797b7b12649`; contract `95530f5c05a26c7ed78303b11af79ed693eb0ca6f0a16cd1c90ac8238392088e`; validator `1eed07cfeb52cd47b39f833649d3cb73ad1efcdcf3205dc45cebafb45b2f4a5e`; manifest `0f00ec615c8c50f35e631a0a723b73e717b854f426e74c4929585d7833b9a0ce`; verification `b6d475140550bd5aed0d5fe02af07fb39bee4fff8fef69464ee7e7e5421ac641`.
- XN16R2: report `48e85145bc05d5bc4a6f90b1dec8226863894bc0c22cac4965addb1b0a08a8dc`; contract `66d3babb6002e694195ca75babca4c6ee8602ef8b519dd152697c6f2723ab6e7`; validator `919f2e876f332511c360073f1f062f2315fde68c63d41669f275ff5b7906d8e2`; manifest `1ac67fea397243739fdb1e28bd742a936cdb8a039664651f3204ea238aceadd6`; verification `41da6a86458024bee1ec3fc2ea1af6ac286809b7b764e6d82e1b5f9ac56ded36`.
- XN16H execution contract: `d3c37f38ebb8971f4528947407c89fb28a454c529c51a462f9f0b3b32c8fcb3e`.
- Combined validator: `669fd4c5f5a31a3ea9d31b9fb394871d4831341864635f0f116c1870d65a775c`.
- Execution card: `2f5bb127712fb10ec7002c929e5bd83de1b5a99cccc36c8d749dad9530e1f3c7`.
- Verification: `ef971a87b21ccc43136bd8b700e6d8311d91bd0725e39d530e1e6b65d96d069a`.

## Provenance and frozen manifest

This is an offline capture-execution/admission packet. Ghidra function address: **N/A**. Ghidra job ID: **N/A**. Ghidra artifact path: **N/A**. The frozen artifact-manifest path is `replay_interface/re_evidence/rendering_laws/XN16H/artifact_manifest.json`; its SHA-256 is `af9783152ade1f23a40a8a9825d4ddbdeab3e883417ce6930119032e226e3c66`.

No `PROGRAM_STATUS.md`, collector, renderer, extractor, XN16 integration artifact, or upstream packet was edited by XN16H. No adversarial review was performed.
