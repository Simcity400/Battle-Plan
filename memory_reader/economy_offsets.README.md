# Per-country economy offsets (DRAFT)

Companion to `economy_offsets.draft.json`. Maps where per-country **economy
scalars** live in live the game memory for build `16022851` (Operation Postern
v1.19.1.0.b49d). **Uncommitted; awaiting merge into `layout.16022851.json`.**
Everything here is **read-only / observational** (the user's own MP game).

## How these were found (correlation, not single-value hunting)

We have ground truth from a melted save **time-aligned** to the running game
(`GER_1940_09_10_13`, save date 1940.9.10.13; melted with
`rakaly melt --format the game -u stringify`). The save gives every economy scalar
per country TAG.

1. **Bridge save TAG ↔ live running_index** using `political_power` as a
   Rosetta stone: it is known in *both* worlds — live at `record+0x20`
   (`i32 / 100000`, signed) and in the save at `politics.political_power`.
   Matching them pins each live array index to a save TAG. All 69 active
   countries bridged; the live PP matched the save PP to full precision
   (e.g. Australia −100.2423, exact incl. the negative).

2. **Restrict scoring to the 43 countries with DISTINCT, NON-ZERO PP.** The
   game has ~26 dormant/minor tags at PP = 0 whose index→TAG identity the PP
   bridge cannot disambiguate; including them would pollute the correlation.
   The 43 reliable countries (GER, ENG, SOV, SWE, ITA, JAP, FRA, RAJ, SIA, …)
   are pinned unambiguously.

3. **Cross-country correlation per candidate offset.** For each byte offset and
   each candidate decoder (`i32/100000`, `i32` raw, `f32`, `f64`, `i64`, …),
   decode the field for all reliable countries and require it to match the save
   value (relative error ≤ 1 % or absolute ≤ 0.002) for ≥ 85–90 % of them **and**
   for the high-variance anchor countries (GER .75, ENG 1.0, SOV .85, SWE .47,
   SIA .31, RAJ .48, TUR .4604, DEN). A clean match across all anchors + nearly
   all countries pins the offset unambiguously — far stronger than chasing one
   value (PP = 0 / stability = 0 minors match almost any zero-ish field).

4. **Verify**: re-read every confirmed offset *fresh* from live memory **twice**
   and require the two reads to agree (sentinel against mid-read mutation). All
   confirmed fields matched the save with median **and** maximum relative error
   = 0 and were stable across both reads.

## Object model

- The country DB is a contiguous array of fixed **0x120-byte** records (the
  index/handle slots `locate.py` finds). `rec = rec0 + index*0x120`.
- **`political_power` is cached flat in the record at `+0x20`.**
- **Every other confirmed scalar lives in the Country data object**, which is
  the heap struct pointed to by **`[record + 0x08]`** — the *same* struct whose
  `+0x10` is the localized name and `+0x30` the adjective (per `locate.py`).
  So the 0x120 record is a lightweight handle; the real per-country object hangs
  off `+0x08` and is large (≥ 0x8000 bytes).
- In-memory fixed-point is **`i32 / 100000`** (signed), confirmed for PP,
  stability, war_support and command_power. Raw counts (num_ships) are plain
  `i32`.

## Confirmed fields

| field            | path                         | width | encoding            | match |
|------------------|------------------------------|-------|---------------------|-------|
| political_power  | `record + 0x20`              | i32   | fixed `/100000`     | 43/43 exact |
| stability        | `[record+0x08] + 0x10D0`     | i32   | fixed `/100000` (0..1) | 43/43 exact |
| war_support      | `[record+0x08] + 0x10D8`     | i32   | fixed `/100000` (0..1) | 43/43 exact |
| command_power    | `[record+0x08] + 0x1F0`      | i32   | fixed `/100000`     | 43/43 exact |
| num_ships        | `[record+0x08] + 0x1330`     | i32   | raw count           | 17/17 exact (likely) |

`stability` and `war_support` are adjacent (`0x10D0` / `0x10D8`). All four
economy fields matched **every** reliable country exactly.

`num_ships` is labelled **likely** rather than confirmed: only 17 of the 43 have
a navy in the save, and the field could be a cached naval-summary aggregate
rather than the canonical fleet count. Harden it with dynamic validation (watch
it tick as ships are built/lost; check 0/small/large navies) before promoting.

### Read-size gotcha (implementation note)
Per-country objects can sit near the end of a committed region. Reading a fixed
large blob (e.g. 0x2000) from the object base can straddle the region boundary,
and `ReadProcessMemory` then returns **0 bytes for the whole span** (not a short
read). During verification this made VEN's object look unreadable ("42/43"); a
size-bounded read (clamp to the region end, or read per-field) returned VEN's
stability = 0.5 **exactly**, giving 43/43. The reader should read fields with
bounded sizes / clamp to the region, not assume a large fixed window is safe.

## Not yet pinned

- **fuel / max_fuel** — **searched hard, NOT pinned.** Not flat in the 0x120
  record; not in the first 0x8000 of the `[+0x08]` Country object; not one level
  deeper from that object's inner pointers; not behind record pointers
  `+0x10/+0x40/+0x48/+0x80/+0x98/+0xB0/+0xE0` (flat or one deeper). A full 7.7 GB
  committed-heap scan **did** find each anchor's fuel value as **f32 and f64**
  (e.g. GER 849000 → 8 f32 hits; ENG 432128.9 → 53 f64 hits — the save's
  fractional values already implied float/double), but none of those addresses
  sit near any country object and there is **no constant-stride per-country
  array** among them. Conclusion: national fuel is owned by a separate
  **logistics/resource manager keyed by country**, not by the country
  record/object, and is not reachable by shallow (1–2 level) traversal. It is
  already available from the monthly save oracle (`fuel_status.fuel/max_fuel`),
  so the daily memory layer can omit it until that manager is mapped.
- **manpower pool** — at this save date the save serializes manpower as a block
  rather than a single clean top-level scalar, so no clean correlation target was
  available. Deferred.
- **factories (civ/mil/dockyard)** — state-derived (summed from owned states),
  not expected to be a canonical country-object scalar. Better read from the
  save/state arrays than from the country object. Deferred.

## Fail-closed / safety

- The reader must refuse to run unless the live exe SHA-256 equals the build hash
  (same gate as `locate.py`/`layout.16022851.json`). Offsets are build-specific.
- Read-only: `PROCESS_VM_READ | PROCESS_QUERY_INFORMATION`, `ReadProcessMemory`,
  `VirtualQueryEx` only. No writes, no injection, no thread suspension. Never
  suspend the game in MP — fail/retry instead.
- Apply the layout `validators` bounds; mark the layout broken and stop
  publishing if a field reads outside its sane domain.
