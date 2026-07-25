> **Read `documents/DECISIONS.md` first.** It is the target authority and outranks this file.
> This file describes the system **as built**.
>
> Superseded here: build-870ebf reader offsets plus a two-modset live matrix and the current patch-day profile process do not satisfy the target's any-player, any-mod-set, any-DLC requirement.

# Mod & game-update robustness plan (2026-07-10, user directive)

**Purpose:** Define fail-closed modset binding, cross-mod validation, and a repeatable
game-update recovery pipeline.
**Completion status:** See `extractor/documents/MASTER_EXTRACTION_CONTRACT.md`; this plan does not
restate the cross-modset or patch-day completion ledger.

Companion contract (replay side):
`replay_interface/documents/MOD_COMPATIBILITY.md` — capture evidence is modset-bound, bare
vanilla joins forbidden.

## Why mods mostly hold by construction (and where they don't)

Mods are script/content: they do NOT change exe memory-structure layouts, and the extractor
reads memory structures — offsets are exe-BUILD-bound, not mod-bound. The real mod surface is
everything derived from Installed content or tuned on VANILLA OBSERVATIONS:

| # | Robustness concern | Risk if violated | Required design |
|---|---|---|---|
| M1 | **Static catalogs must be modset-resolved** (templates_equipment, localisation, decisions, idea_tech_loc_xref, ship_definitions, gfx_icon_manifest) | **Silent wrong joins on a modded recording — worst gap, silent-loss class** | Catalog generation takes the recording's modset (mod file tree overlay, load-order aware); the launcher resolves catalogs for that modset or refuses (fail-closed on mismatch, never silently vanilla) |
| M2 | **Manifest-bound modset identity** | Without it, evidence cannot be modset-bound and a modded replay is indistinguishable from vanilla | Record enabled mods + DLC set (dlc_load.json / launcher db + .mod descriptors, with checksums) in control/manifest; catalogs carry their source modset; mismatch = gate failure |
| M3 | **Validation bounds tuned on vanilla** (d47 char_id ≤ 2,000,000 & skill ≤ 15; vector sanity caps e.g. BUILDING_VECTOR_MAX=64; tag-validity ranges) | A large mod trips fail-closed rejection → loud data loss (post-backstop) | Bounds audit sweep: each bound either derived from installed content, or generous + per-bound diagnostic counter so the FIRST out-of-range mod value is attributable, not just rejected |
| M4 | **Script-keyed enumerations** (ideologies, idea slots/categories, equipment archetypes) if any reader hardcodes vanilla token lists | Missing/misclassified rows on mods | Audit for hardcoded vanilla tokens; source them from the (modset-resolved) catalogs |
| M5 | **Scale**: total conversions add tags/states/provinces | d53 is quadratic in tags → bytes+time grow; map-size caps | Caps audited generous (state id 65535, provinces 2^20); scale is a perf concern not correctness |
| M6 | **Cross-modset validation matrix** | Unknown unknowns remain without live modset evidence | Run a short standard capture + full gate battery on 2–3 modsets (one big overhaul e.g. Kaiserreich/RT56, one total conversion, one QoL-mods stack). The existing gates/census machinery detects breakage cheaply; fix what fails, record per-modset evidence |

Definition of "mod-ready": M1+M2 landed (fail-closed modset binding), M3/M4 audits clean, and
the M6 matrix green on at least two non-vanilla modsets.

## Game-update preparedness (recurring cost → pipeline)

At the build-870ebf design baseline, offsets live centrally in `compact_contract::build_profile`
keyed to that build
(exe sha256 pinned); every scanned type is validated against its MSVC RTTI class name, so a
build shift FAILS CLOSED (refuses to extract) rather than extracting garbage. That's the
right failure mode; the goal now is cheap recovery. Plan, in order of leverage:

| # | Item | Why it pays |
|---|---|---|
| U1 | **RTTI auto-re-derivation tool.** Every pinned root already carries its MSVC RTTI class name; vtable RVAs are mechanically findable on ANY new exe by scanning RTTI type descriptors → complete-object locators → vtables. One tool run regenerates the majority of `build_profile` (all *_VTABLE_RVA) with zero hand-RE | Most of the profile is vtables; this turns patch-day RE into minutes |
| U2 | **Signature scans for globals** (CGameState global, other non-vtable anchors): byte-pattern signatures captured around their accessor instructions, stored alongside the profile with 2–3 redundant patterns each | Globals are few but currently hand-derived; sigscan makes them mechanical and resilient to small code motion |
| U3 | **Per-offset provenance table** for FIELD offsets (the long tail: +0x468 hour, CPROVINCE_OFF_*, relation +0xD0, ...): for each, record HOW it was derived (which oracle/diff/RE recipe) + its runtime VALIDATOR (tag echo, monotonic hour, vtable-typed neighbor). Re-derivation becomes following a recipe, not archaeology. Validators already make wrong offsets fail closed at runtime | Field offsets are what actually shift when Paradox adds struct members; recipes + oracles bound the work |
| U4 | **Build-profile as data, keyed by exe hash**: multiple profiles selectable at attach (unknown hash = refuse + name the runbook). Supporting a new patch = adding a profile entry, no code edits. Replay FORMAT stays build-independent (old replays already decode regardless) | Decouples "new build" from "new binary release" |
| U5 | **New-build acceptance runbook**: U1 → U2 → U3 recipes for any failed validator → short vanilla capture → gate battery + census → green = supported. Single documented pass/fail checklist | Makes patch-day a bounded, repeatable procedure; MP can pin the old version until green |
| U6 | Duplicated constants audit: offsets copied into python tools (e.g. d23 probe scripts) must be GENERATED from the profile, not hand-mirrored | Kills silent skew between tools and the DLL |

## Dependency order (not execution status)

Current completion remains in `MASTER_EXTRACTION_CONTRACT.md`. The design dependency order is:

1. M2 (modset in manifest/control — unlocks evidence binding) → M1 (modset-resolved
   catalogs, fail-closed) → M6 first matrix run → M3/M4 audits from what the matrix flags.
2. U1 tool (highest leverage) → U4 profile-as-data → U2/U3/U5 as the first real patch lands.
3. Every item gets the standard treatment: Codex-gated lane, tests, gate evidence; silent
   fallback to vanilla anything is forbidden throughout.
