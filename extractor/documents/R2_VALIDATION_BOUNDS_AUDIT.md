# R2 — Validation-bounds audit (mod-robustness)

**Purpose (plan §4 R2, mod-support mandate):** every hardcoded plausibility bound tuned on
*vanilla* observation is a place a **modded** save can legitimately exceed the limit. This
audit tables every such bound and classifies its exceed-behavior. The standing law (the d69
pattern) is: a bound MUST **fail closed WITH an attribution sample**, so a modded save that
trips one **names itself in a single capture** instead of silently losing data.

**Evidence scope:** 239 named ceiling constants swept across `extractor/native/collector/src/lib.rs` and
`extractor/native/compact_contract/src/build_profile.rs`, filtered to the 149 that gate *content* (IDs, counts,
lengths, caps) — the mod-exceedable set. Pure
value/magnitude sanity limits (100M-scale raw-word guards) and internal-safety limits
(byte budgets, worker durations, cache-row caps) are catalogued but are **not** mod-support
gates (a mod cannot make a country lose 200M divisions or a worker run 25h).

**Historical snapshot (superseded as status 2026-07-14):** the audit table was complete for the
mod-critical subset at the audited revision; its exceed-behavior column is derived from the
then-current fail-site code pattern. **The dominant finding was:
most bounds use `.filter(|v| *v <= MAX)`, which SILENTLY DROPS the field (→ `None`/absent) when
exceeded — NOT fail-closed-with-attribution.** On vanilla this never fires; on a modded save
that exceeds the bound it is a *silent* per-field data loss with no sample naming the trip. The
d69/d47 attribution-sample pattern is the target end-state; the table marks which bounds already
have it vs. which need it before R3.
**Completion status:** See `extractor/documents/MASTER_EXTRACTION_CONTRACT.md`; the implementation
note below is retained as evidence and does not replace that ledger.

## Legend — exceed-behavior
- **CLAMP-DROP**: `.filter(<=MAX)` → value becomes `None`/absent, frame stays CLEAN. Silent on
  a modded save. *Mod-unsafe without an attribution sample.*
- **CAP-TRUNCATE**: a row/vector walk stops at MAX; extra rows silently omitted. Silent.
- **FAIL-SAMPLE**: exceeding trips a fail-closed path that emits a `_sample`/attribution unit
  (the d69/d47 law) — a modded trip is visible in one capture. *Mod-safe.*
- **STRUCTURAL**: the bound equals a genuine engine invariant (e.g. state/province id space,
  research-slot hardware cap) a mod cannot exceed without a game-code change → not a real risk.

## Mod-CRITICAL content ceilings (a content/overhaul mod can legitimately exceed)

| Constant | Value | Evidence basis | Exceed-behavior | Mod-safe? |
|---|---|---|---|---|
| `MAX_UNIT_LEADER_SKILL` | 15 | vanilla leader skill cap | CLAMP-DROP (`.filter`) | **NO** — an overhaul raising the skill cap silently drops skill; add FAIL-SAMPLE |
| `MAX_FOCUS_COUNT` | 4000 | vanilla largest focus tree | CLAMP-DROP | **NO** — mega-tree overhaul; add sample |
| `MAX_COMPLETED_TECH_COUNT` | 2000 | vanilla tech count | CLAMP-DROP | **NO** — tech overhaul; add sample |
| `MAX_ACTIVE_IDEAS` | 400 | vanilla idea slots observed | FAIL (size>cap → rowless) | PARTIAL — fails closed but check the sample carries the raw size |
| `MAX_COMPACT_OWNER_TAG_DB_ID` / `_MAX_MEMBER_TAG_DB_ID` / `_MAX_PLAYER_TAG_DB_ID` | 2048 | vanilla + DLC country count | CLAMP-DROP / row-skip | **NO (high risk)** — a >2048-country overhaul (full-world mods) silently drops owners/members/players; this is the single most likely overhaul trip. Needs FAIL-SAMPLE at every tag gate |
| `_MAX_TAG_DB_ID` (d17) | 1024 | narrower war-relations gate | CLAMP-DROP | **NO** — even a moderate country-count mod trips this before 2048 |
| `DOMAIN69_MAX_MEMBERS_PER_GROUP` | 512 | tech-sharing group size | FAIL (torn) — but shares the count fail path | REVIEW — a >512-member group (unlikely) skips the group PARTIAL; sample present |
| `DOMAIN69_TECH_SHARING_MAX_GROUPS` | 256 | 2026-07-12 fix ceiling (D04=9) | FAIL-SAMPLE (fallback→PARTIAL) | **YES** — >256 groups → fail-closed PARTIAL, retains prior (the d69 fix pattern) |
| `_FACTION_MAX_MEMBERS` / `_FACTION_MAX_SLOTS` | 256 / 512 | vanilla faction size | CAP-TRUNCATE | **NO** — a mega-faction overhaul truncates members; add sample |
| `MAX_DECISION_STATUS_LIST` | 4096 | vanilla decision count | CLAMP/CAP | REVIEW |
| `MAX_PROJECTS_PER_COUNTRY` | 64 | vanilla special-projects | CAP-TRUNCATE | REVIEW — overhaul with more projects |
| `MAX_RESEARCH_SLOT_COUNT` | 32 | engine research-slot array | STRUCTURAL-ish | LOW — mods rarely exceed 32 slots |
| `MAX_STATE_ID` / `MAX_PROVINCE_ID` | 65535 / 70000 | engine id space (u16-ish) | STRUCTURAL | LOW — a bigger-map mod exceeding 65535 states is rare but POSSIBLE (e.g. Millennium Dawn-scale); flag for the overhaul session |
| string caps `_LEN` (96/128/160/256/512) | — | vanilla token/name lengths | CLAMP-DROP (truncate/absent) | REVIEW — long modded tokens truncate silently; low-severity but list |

## Value/magnitude sanity limits (NOT mod gates)
`MAX_COUNTRY_CONVOYS_DESTROYED`=100M, `MAX_DIVISION_STRENGTH_RAW`=1e9, `WR_MAX_CASUALTIES`=50M,
`STATE_RESOURCE_MAX_ABS_RAW`=100M, the `*_RAW`/`*_MANPOWER` 1e6–1e9 guards — these guard against
**torn-read garbage**, not content. A mod cannot make these values legitimately huge; a trip
means a bad read → fail-closed is correct. No R2 action.

## Internal-safety limits (NOT mod gates)
Byte budgets (`MAX_*_PAYLOAD_BYTES`, `COMPACT_MAX_PAYLOAD_LEN`), worker/timeout limits
(`MAX_WORKER_DURATION_MS`, `HOUR_LATCH_*`), and cache-row caps (`MAX_*_CACHE_ROWS`=20000,
bridge-row caps 10k–250k) bound the extractor's own resource use, not game content. No R2 action.

## R2 → R3 handoff (what the cross-modset matrix must watch)
The **NO** rows above are the R3 tripwires. For the three R3 sessions:
- **(a) cosmetic mod**: likely trips nothing (no content-count change) — a clean baseline.
- **(b) DLC-disabled**: FEWER of everything → trips nothing; validates the resolver, not bounds.
- **(c) major overhaul (Kaiserreich-class)**: the real test — expect trips on the tag ceilings
  (2048/1024), `MAX_FOCUS_COUNT`, `MAX_COMPLETED_TECH_COUNT`, faction/skill caps. **Because most
  are CLAMP-DROP (silent), R3 CANNOT rely on a fail-sample surfacing the trip** — it must diff
  per-domain row counts/fields against a vanilla baseline AND (ideally) the overhaul's melt
  oracle. This is the concrete argument that R3's overhaul session needs a melt-oracle cross-check,
  not just a "did it run" check.

## Follow-up disposition

The required pattern is a `*_bound_trip_sample` carrying bound id and observed raw value, with
last-value rather than summed sample semantics, so any trip is attributable in one capture.

**Historical implementation evidence (2026-07-12):** `compact_contract::bound_trips` registry +
`bt_*` units on probe domain 250, drained once per frame in every profile; acceptance behavior
was unchanged.
Wired bounds (8 ids): `MAX_UNIT_LEADER_SKILL`, `MAX_FOCUS_COUNT` (filters + cap arm),
`MAX_COMPLETED_TECH_COUNT` (filters + cap arm), `MAX_COMPACT_OWNER_TAG_DB_ID` (all five
collector 2048 gates: d47/d48/d68-roster/market-trade/frd helpers), `DOMAIN68_MAX_PLAYER_TAG_DB_ID`,
the narrow 1024 tag/count gates (bridges, air-wing owner, map state/province controller,
country-table walk — 14 sites, shared `bt_tag_db_id_1024` id), `DOMAIN53_FACTION_MAX_SLOTS`,
`DOMAIN53_FACTION_MAX_MEMBERS`. `*_trips` counters sum in the recorder; `*_raw_sample` units
are last-value. Manifest surfaces `compact_stream.bound_trips` (empty on vanilla). The
`_LEN` string caps and REVIEW rows (decision-status 4096, projects 64) remain unwired —
lower severity, revisit with R3 evidence. Escalating nonzero bound_trips to a run-failing
problem is deliberately deferred to the C1 disposition design (a modded trip is accounted,
visible loss — not yet a gate verdict).
