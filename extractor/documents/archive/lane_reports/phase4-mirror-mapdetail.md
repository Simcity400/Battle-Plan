# Lane report — `phase4-mirror-mapdetail`

Date: 2026-07-10. Base commit: `779d65a17`. Branch: `lane/phase4-mirror-mapdetail`.
Method: STATIC RE (analyzed Ghidra project `C:\Users\Example\ghidra_hoi4_current`, read-only)
+ source reading of the shipped collector + the checked-in live census
(`extractor/native/runs/d23_speed/EVIDENCE.md`, measured 2026-07-05 on paused D04).
No live game this session. Codex adversarial review RUN (verbatim below).

## Mission

Bring `map_detail` (domain 23; 49.2 ms avg / 99.6 ms p95 read; 0.1 % of rows change/hour)
onto the dense-mirror + host-diff pattern that makes `map_control` (domain 22) 40× cheaper
(1.2 ms) for the same province set. Zero data loss is the governing law; per-frame CPU must go
down; legacy stays default until a live shadow run passes.

## TL;DR / verdict

**The lane's headline premise is not achievable, and I did not ship a speculative reader.**
map_detail's dominant field — building LEVELS — is **provably not dense-mirrorable**. It does not
live in any contiguous array; it lives at `+0x40` inside individually heap-allocated 0x1F0-byte
`CBuilding` objects reached only by a per-province/per-state `pdx::vector<CBuilding*>` pointer
chase. So map_control's "read one scalar from a dense array" trick does not transfer, and the
plan's projection **"map_detail 49.2 → 1.2 ms (48 ms saved) via dense mirror" is unsupported and
should be removed.** The 40× gap is a **data-shape** difference (one scalar inside an object the
topology already walks, vs a variable-length set of extra fat heap objects), **not** a
"change-driven vs full-read" difference — map_detail already emits change-driven (the 0.1 %).

Codex's adversarial review (below) correctly refuted a *second, stronger* claim I had initially
drafted — that "no dense mirror ⇒ the per-building read is irreducible." That does **not** follow:
a **page/region-coalesced bulk read** of the (already-known) building pointers is complete by
construction and can cut guarded reads from one-per-building to ~one-per-occupied-run **without a
semantic mirror**. Whether that pays off is an **empirical address-locality question** that cannot
be answered offline. I therefore shipped the **decisive live experiment** to size it
(`d23_locality_probe.py`) rather than a speculative reader whose benefit and real-layout
completeness I cannot verify without live memory. This matches Codex's own recommendation
("the decisive next experiment is a pointer-locality histogram … not another search for a
semantic dense mirror").

## What changed (files)

- **`extractor/native/runs/d23_speed/d23_locality_probe.py`** (new): READ-ONLY live probe. Reuses
  the proven access pattern of the sibling `d23_profile_live.py` (same offsets, same winmem RPM).
  Collects every heap-valid `CBuilding*` across all state+province vectors, then reports: unique
  4 KiB start-pages, readable-region count, buildings/page density, and — the load-bearing number —
  the guarded read count a coalescing reader would issue at several gap caps vs the current
  one-read-per-building count, **REGION-AWARE** (a span may only merge pointers whose whole
  `[ptr, ptr+0x42)` lies inside ONE committed-readable `VirtualQuery` region — the exact production
  `range_confirmed_good` constraint, so it will not overstate the reduction) plus
  **copy-amplification** bytes (a wide gap cap copies dead bytes; read-call count ≠ wall time). It
  brackets `gs+0x468` at both ends and prints the build fingerprint. Its coalescing simulator has
  offline self-tests including region-boundary and span-escape cases (below); the memory-walk half
  needs a paused game. This is the data that must exist before anyone writes the coalesced reader.
- **`extractor/documents/lane_reports/phase4-mirror-mapdetail.md`** (this file).
- **No collector/recorder source changed.** The legacy reader remains the sole, default path
  (zero data-loss risk introduced).

## Cost-structure finding (why 49.2 ms vs 1.2 ms)

Both domains share the SAME cached topology (`CompactMapDetailTracker.topology` /
`CompactMapControlTracker.topology`, rebuilt only when `cprovince_array_still_current` fails), so
topology re-derivation is NOT the gap. The per-frame difference is the field reads:

- **map_control** (`sample_compact_map_control_from_topology`, lib.rs:33516): per state 2× u32
  (owner `STATE_OFF_OWNER=0xC8`, controller `STATE_OFF_CTRL=0xCC`); per province 1× u32
  (`CPROVINCE_OFF_CONTROLLER=0x188`). Every field lives INSIDE the CProvince/CState object the
  topology already resolved ⇒ one scalar read per entity. → 1.2 ms.
- **map_detail** (`sample_compact_map_detail_from_topology`, lib.rs:33348): per state, resources +
  buildings; per province, buildings.
  - **Resources** (`state_resource_entries`, lib.rs:43752): chase `STATE_OFF_DEF=0x30` → StateDef,
    read the resources `pdx::vector` header at `STATE_DEF_OFF_RESOURCES_PTR=0xF8`
    (size `0x100`, cap `0x104`, stride `0x10`), then ONE contiguous `read_into` of the rows.
    ~3 guarded reads/state over ~1081 states. **Already contiguous / bulk-read — cheap, not the
    bottleneck.**
  - **Buildings** (`building_level_entries`, lib.rs:43858): read the `pdx::vector<CBuilding*>`
    header (16 B) at `STATE_OFF_BUILDINGS_VEC=0x158` / `CPROVINCE_OFF_BUILDINGS_VEC=0x1C8`, read the
    pointer array (`logical*8`), then for EACH pointer one guarded `read_into` of the CBuilding
    header span `[+0x00 vtable, +0x08 type, +0x40 level]` (0x42 B), vtable-checked and aggregated by
    type. **This per-object pointer chase over scattered fat heap objects is the dominant cost.**

Live census (`EVIDENCE.md`, paused D04 @1944): `building_vector_calls=14,494`,
`empty_vecs=9,512`, `total_building_ptrs=8,590`, all 8,590 valid + vtable-matching. The
per-building step is **already** batched (3.1× win landed 2026-07-05: `cbuilding_type_and_level`,
one 0x42 `read_into` replacing 3 scalar reads) and measured ~8.2–9.6 ms for those 8,590 objects
in that save. The deep-war corpus (`live_validation_20260709_221706`) reports 49.2 ms because
late-war has far more built-up states/provinces (more CBuilding objects) **and** the traversal also
pays 14,494 vector-header reads + ~4,982 pointer-array reads + a shared `Mutex<BTreeMap>`
building-levels cache (`CompactFrameBuildingLevelsCache`, lib.rs:6118) that clones a `Vec` on every
lookup while province sampling runs on multiple worker threads. So the 49.2 ms is an **impure**
measure — not attributable solely to scattered building reads.

## What IS / ISN'T dense-mirrorable (offsets + evidence)

| field | source | dense mirror? | evidence |
|---|---|---|---|
| province/state controller/owner (map_control) | scalar inside CProvince/CState | **YES** (the twin) | lib.rs:33516; offsets 0xC8/0xCC/0x188 |
| state strategic resources | contiguous `pdx::vector` in StateDef | **YES — already bulk-read** | lib.rs:43752; StateDef+0xF8/0x100/0x104 stride 0x10 |
| **building LEVELS (dominant)** | `+0x40` inside per-building `CBuilding` heap objects | **NO** | see below |

**Building non-mirrorability — RE evidence (Ghidra, image base 0x140000000):**
  back-ref@+0x1D8, type-template@+0x1E0. Object size **0x1F0**.
  pointer into the vector at manager+0x38. There is a dense `int` array at manager+0x20 mapping
  building-type-index → vector slot (or −1), but it only encodes **which types exist, not their
  levels**.
- The only `CBuildingLevel` container in the binary,
  `CPdxHybridInlineBufferAllocator<CBuildingLevel,7,int>` (vftable @142?9c50d8), is used **only** in
  `statehistory.cpp` (string "State History Error: Province buildings being overridden…") — a
  transient load-time parse buffer, **not** a persistent per-province store.
- Prior static scout (`PHASE4_DIRTY_SIGNAL_SCOUT.md`): the engine exposes NO cheap per-object
  dirty/version counter and NO usable change signal (MP checksum is an O(whole-world) walk).

Conclusion: a province's building levels can be observed **only** by dereferencing each of its
`CBuilding` pointers. No contiguous (type,level) array exists to bulk-scan.

## Completeness argument (the lane's design law, preserved)

The dense-mirror pattern is lossless because it **observes every entity's live value cheaply and
diffs downstream — it never trusts the engine to say what changed** (`CODEX_LANE_BRIEF.md`). The
current map_detail reader already honors this: it reads **every** building's live level every hour
and diffs each row against the retained baseline (`read_compact_map_detail`, lib.rs:32714–32770,
`tracker.previous_states`/`previous_provinces`), emitting only changed rows. The 0.1 % churn is a
**host/collector-side diff of a complete read**, not a trusted signal. Any future coalesced reader
MUST keep this: it may batch the *copies* (a page span covers many objects in one guarded read) but
must still cover **every** building pointer, fall back to a per-object read on any span failure, and
propagate "unreadable" identically (legacy fails the whole vector ⇒ SOURCE_UNAVAILABLE). Because it
reads the same level bytes, it is complete by construction.

## Plan correction (do not edit the plan doc — recorded here for the orchestrator)

`EXTRACTION_OPTIMIZATION_PLAN.md` DEEP-WAR LEVER RANKING row **"1b ├ map_detail change-driven
49.2→1.2 = 48.0 ms (MED, twin exists)"** is **unsupported**:
- map_control is not a valid *structural* twin — its payload is a scalar in the dense array;
  map_detail's is a variable list of fat heap objects.
- `proj = changed_frac × read` cannot be applied without a complete skip signal (none exists), and
  map_detail already emits change-driven, so there is no emission win left.
- Realistic map_detail read floor is NOT 1.2 ms. The only lossless read lever is a page-coalesced
  bulk read whose payoff is **locality-dependent and currently unmeasured** (`d23_locality_probe.py`
  will size it live). Recommend replacing the 48 ms line with "locality-gated; measure first."

## Gate evidence (this session)

- `d23_locality_probe.py`: `ast.parse` → **SYNTAX OK**. Region-aware coalescing simulator offline
  self-test → **REGION-AWARE SIM SELF-TEST OK** (adjacent-merge with byte accounting; page-apart
  split; empty; single; cross-region cannot merge even when adjacent; object whose 0x42 end escapes
  its region is its own read; two adjacent in one big region merge). Memory-walk half requires a
  paused game.
- No Rust source changed ⇒ no build/test regression possible; the legacy reader and its existing
  goldens (`building_level_entries_matches_golden_after_batched_read`,
  `cbuilding_type_and_level_fast_path_matches_contract`) are untouched.
- Byte-identity gate (task 4a) and shadow gate (4b): **not applicable this session** — there is no
  new reader to compare. They become mandatory the moment a coalesced reader is written; the design
  above specifies exactly how (per-object fallback + identical unreadable-propagation + shadow mode
  counting `domain23_mirror_shadow_mismatches`, mirroring d53's live-passed shadow gate at
  `diplomacy_intel_operatives_live/src/extractor.rs:1685`).

## Projected ms

- Dense-mirror swap for buildings: **not achievable** (no mirror). Floor is NOT 1.2 ms.
- Page-coalesced read: **unknown offline.** Upper bound if CBuildings were perfectly slab-packed:
  8,590 objects × 0x1F0 (496 B) ≈ **4.06 MiB ≈ ~1,041 pages ⇒ up to ~8×** fewer guarded reads on
  the building step (which is ~8–9 ms of the 1944 census; a larger absolute share deep-war). Lower
  bound if scattered / region-split: ~0. `d23_locality_probe.py` resolves which (region-aware, so
  it will not overstate — see below).
- The `Mutex<BTreeMap>` clone/contention on the shared building-levels cache is a separate,
  offline-provable CPU lever (out of this lane's "dense-mirror" scope; flagged for a cache lane).

## What must still be validated live (next session, in order)

1. Run `d23_locality_probe.py` on a paused deep-war save (bracket `gs+0x468`). Read
   `unique_4KiB_pages`, buildings/page, and coalesced-vs-per-building read counts. **Decision gate:**
   if coalesced reads ≪ buildings → build the coalesced `mirror` reader behind
   `map_detail_read_mode = legacy|mirror|shadow` (default legacy); else keep legacy and close the lever.
2. If built: prove byte-identity offline on synthetic clustered fixtures (spans straddling page
   boundaries, unreadable middle → fallback, empty vectors, dup-type accumulate), then shadow-soak
   live per the d53 template — zero `domain23_mirror_shadow_mismatches` over a deep-war soak — before
   enabling. Legacy stays default until then.

## Codex adversarial review — VERBATIM, with disposition

Codex (gpt-5.6-sol, xhigh, read-only) was asked to REFUTE the finding. **Verdict: REFUTED** — it
accepted the "no dense mirror ⇒ not 1.2 ms" half but refuted the over-strong "reads are
irreducible" half. Key passages (verbatim):

> "The rejection of the specific 'dense mirror ⇒ 49.2→1.2 ms' projection is sound. The finding is
> nevertheless wrong because it converts 'no semantic mirror' into 'one guarded read per building is
> irreducible.' That does not follow."
> spatial scattering. Same-size allocators commonly place allocations in shared slabs/segments. You
> already possess every relevant pointer, so no heap sweep is needed: 1. Snapshot all building
> vectors … 2. Sort pointers by readable region/page. 3. Coalesce nearby `[ptr, ptr+0x42)` windows …
> 4. Bulk-copy each span once … 6. Fall back to the existing per-object read if a span fails. …
> This observes every current level."
> "With 8,590 same-sized objects, perfect 0x1F0 packing would occupy only about 1,040 4-KiB pages.
> Whether the real reduction is 8× or negligible is an empirical address-locality question, not
> something the allocator call settles."
> "A. I see no demonstrated dense semantic level mirror. … But a semantic mirror is not required for
> a complete coalesced read. B. 'Individually allocated' is sound; 'therefore scattered and
> unbatchable' is not. … C. The zero-loss premise is correct, but the inference is not. …
> D. The plan's projection is unsupported. … But 'therefore no lossless optimization remains' is
> also unsupported."
> "the current one-span-per-building step [was measured] at about 8.2–9.6 ms for 8,590 objects, not
> 49.2 ms; the traversal also performs 14,494 vector-header reads and roughly 4,982 pointer-array
> reads." "Every entity lookup passes through one shared `Mutex<BTreeMap>`, with a second lock and
> clone on misses … makes the 49.2 ms an impure measure of memory shape."
> "So: discard the unproven 1.2 ms projection, but do not abandon optimization. The next decisive
> experiment is a pointer-locality histogram plus an A/B page-coalesced reader — not another search
> for a semantic dense mirror."

**Disposition (what I did about each):**
- *"no semantic mirror" ≠ "irreducible reads"* — **ACCEPTED.** I removed the over-strong "no
  lossless optimization remains" claim. The report now states the coalesced-read lever explicitly
  and preserves it as the recommended path.
- *coalesced page/region bulk read is complete + a real lever* — **ACCEPTED.** Documented as the
  design of the future `mirror` reader (with the fallback + unreadable-propagation completeness
  requirements spelled out).
- *real reduction is empirical, not settled by the allocator call* — **ACCEPTED**, and this is the
  crux of why I did NOT ship the reader blind: I shipped `d23_locality_probe.py` to measure it
  live first (exactly Codex's "decisive next experiment").
- *49.2 ms is impure (header/ptr-array reads + Mutex/clone)* — **ACCEPTED.** Corrected the
  cost-structure section; flagged the shared-cache `Mutex<BTreeMap>` clone as a separate lever.
- *keep the dense-mirror = 1.2 ms projection out of the plan* — **ACCEPTED**; recorded the plan
  correction here (I am barred from editing the plan doc this session).

I did not reject any Codex point. One residual doubt I am surfacing rather than resolving: I cannot
verify coalesced-reader completeness on the REAL heap layout this session (only synthetic), so even
after the locality probe is favorable, the coalesced reader must ship default-off and clear a live
shadow-soak before it is ever enabled.

### Codex second review (of the deferral decision + the probe) — VERBATIM, with disposition

**Verdict: REFUTED** — Codex accepted the deferral ("The narrow choice to keep the legacy reader
and defer an unvalidated implementation is sound under the explicit 'STOP and report' instruction")
but refuted the *first draft* of the probe as an optimistic, potentially-misleading estimator, and
flagged a report arithmetic error. Key passages (verbatim):

> "the probe is not a faithful production-read simulator and can produce an optimistic false GO."
> "**It ignores the production region guard.** Production accepts a read only when the entire span
> lies inside one currently committed, readable, non-guard `VirtualQuery` region; even two adjacent
> readable regions cannot be crossed … The simulator merges solely by address gap and `MAX_SPAN`.
> It therefore generally understates real reads."
> "**Read count is not performance.** Large gap caps can copy vastly more dead data. With 0x1F0-spaced
> objects, useful bytes are 0x42 each but merged bytes approach 0x1F0 each—about 7.5× copy
> amplification."
> "It assumes global coalescing. Production currently samples province vectors through worker shards
> … Globally sorting all state/province pointers requires a two-phase architectural rewrite.
> Per-vector or per-worker coalescing will issue more reads than this global lower bound."
> "The report also contains an arithmetic error: 8,590 × 0x1F0 is approximately **4.06 MiB**, not
> 1.6 MiB; that is roughly 1,041 contiguous 4-KiB pages, not 410."
> "The probe is genuinely non-mutating … Its risk is invalid evidence, not game modification."
> "It prints only the initial gate despite instructing the operator to bracket both ends … `ru()`
> assumes `p.read()` returns a failure value, but `p.read()` actually raises on short/failed reads.
> The probe therefore crashes rather than recording failure counters … no bounds checks on a
> corrupted `province_count`."
> "the claimed plan correction has not landed. The plan still says … projects 49.2→1.2 ms. The lane
> report merely recommends correcting it."
> "Single most important correction: Replace the address-only simulator as the GO gate with a
> **production-parity region-aware dry run**: … query actual readable regions, form spans only within
> one region and `MAX_SPAN`, preserve duplicate-to-vector mappings, and report total attempted reads
> … plus total bytes/copy amplification. … Until that exists, label the current output an
> **optimistic global lower bound**, never a prediction of production reduction."

**Disposition (ONE fix round applied):**
- *Region guard ignored* — **FIXED.** `simulate_coalesced_reads` is now **region-aware**: it queries
  `p.iter_regions` (winmem's committed-readable `VirtualQueryEx` walk) and a span may only merge
  pointers whose whole `[ptr, ptr+0x42)` lies inside ONE region; an object whose span escapes its
  region becomes its own read. Self-tested (cross-region no-merge, span-escape).
- *Copy amplification not modeled* — **FIXED.** The probe now reports bytes copied + amplification
  (`copy … MiB (amp Nx vs … useful)`) alongside the read-call count, and the decision gate requires
  BOTH a large call reduction AND low amplification.
- *Global-coalescing assumption* — **ACCEPTED, labelled.** The probe output is explicitly called a
  GLOBAL-coalescing estimate and states a real reader is worker-sharded unless rewritten two-phase.
- *Report arithmetic (1.6 MiB/410 → 4.06 MiB/1,041; ~20× → ~8×)* — **FIXED** in "Projected ms".
- *`ru()` crashes on failed read; single-ended gate; no `province_count` bound; no fingerprint* —
  **FIXED.** `ru()` wraps `p.read` in try/except → None; gate is read+printed at START and END
  (exit 4 if it advanced); `province_count`/`province_array` sanity-bounded before use; build
  fingerprint (sha256) printed.
- *Should I have shipped a default-off reader anyway* — **CONSIDERED, still deferred.** Codex's
  strongest case (correctness can be made layout-independent; only a real reader measures in-process
  cost) is real, but its own conclusion agrees deferral is "defensible" under the STOP-rather-than-ship
  law, and shipping ~200 lines of two-phase coalescing into the hottest read path with only synthetic
  completeness evidence and unknown benefit is exactly the risk the law guards against. The probe
  makes the next live session's GO/NO-GO data-driven in one run, after which the reader (if built)
  ships default-off + shadow-gated. I am surfacing this as a live decision, not resolving it here.
- *Plan not actually corrected* — **CONSTRAINT, not an oversight.** The orchestrator barred editing
  `EXTRACTION_OPTIMIZATION_PLAN.md` this session (concurrent-lane conflicts); the correction is
  recorded above for the orchestrator to apply.
