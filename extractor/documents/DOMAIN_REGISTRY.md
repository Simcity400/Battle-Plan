# DOMAIN_REGISTRY.md — RETIRED 2026-07-05

**Purpose:** Preserve the retired filename as a compatibility stub and route every reader to the
code-authoritative registry and the single completion ledger.

The former hand-maintained table existed to coordinate domain-ID assignment across independent
lane subagents, a workflow no longer in use.

Where its information lives now:

| You want… | Read |
| --- | --- |
| Domain IDs, status, schedulability (the source of truth) | `extractor/native/domain_registry.json` — and the code-authoritative `compact_contract::ASSIGNED_COMPACT_DOMAINS` in `extractor/native/compact_contract/src/lib.rs` |
| What each domain/slice actually covers + what remains open (completion contract) | `extractor/documents/MASTER_EXTRACTION_CONTRACT.md` |
| The recorder static-artifacts table (migrated here) | `extractor/documents/MASTER_EXTRACTION_CONTRACT.md` → "Static Artifacts Packed By Recorder" |
| The exhaustive census of every extractable datum | `extractor/documents/EXTRACTION_ATLAS.md` |

Do not re-add a hand-maintained domain status table here — it duplicated the
JSON and drifted stale. If a future workflow genuinely needs an ID registry,
design a new form rather than reviving this file.

(This stub is kept at the original path so existing doc/comment references still
resolve; it carries no maintained data.)
