# Extractor Documents

**Purpose:** Route readers to canonical extractor status, code-authoritative domain metadata,
active design/reference documents, and archived evidence.

| File | Purpose |
| --- | --- |
| `MASTER_EXTRACTION_CONTRACT.md` | The only active extraction completion checklist and backlog (also holds the recorder static-artifacts table). |
| `EXTRACTION_ATLAS.md` | Exhaustive descriptive census of every extractable datum (save paths, per-country schema, memory-only/UI state, gaps). |
| `PRODUCT_SESSION_SEMANTICS.md` | Product design for attach, campaign identity, reload, and stitching behavior. |
| `DOMAIN_REGISTRY.md` | RETIRED 2026-07-05 — stub redirect only. Domain IDs/status now single-sourced in `extractor/native/domain_registry.json` (+ `compact_contract`). |
| `REVERSE_ENGINEERING_TOOLING.md` | Tooling setup and reverse-engineering workflow notes. |
| `RE_REMAINING.md` | RE promotion ledger, incl. the A24–A40 seam contracts. |
| `LIVE_ACCEPTANCE_RUNBOOK.md` | The owner-run path to production acceptance. |

**Other active docs** (design/spec + evidence — define intended behavior or retain measurements,
never completion status): design specs `CAMPAIGN_IDENTITY_DESIGN`, `DUPLEX_CONTROL_TRANSPORT`,
`CONFIRMED_COMPLETE_DESIGN`, `DERIVED_RECOMPUTE_SPECS`, `AUTO_ATTACH_DESIGN`, `STITCHING_DESIGN`,
`C1_STAGEB_REGISTRY_DESIGN`, `PORTABLE_CONTENT_MANIFEST`, `XUI21_RE_REPORT`; evidence/reference
`dossiers/`; runbooks/audits `R4_PATCH_DAY_RUNBOOK`, `R2_VALIDATION_BOUNDS_AUDIT`,
`MOD_AND_UPDATE_ROBUSTNESS_PLAN`. Repo-wide status: `../../documents/STATUS.md` and
`../../documents/REMAINING_WORK.md`.

Design/spec documents in this directory define intended behavior but do not carry completion
status. Historical plans, proof reports, lane packets, and superseded status files live in
`archive/`; retained root-level historical evidence files label themselves explicitly.

Do not create another extraction completion plan. Update
`MASTER_EXTRACTION_CONTRACT.md` instead.
