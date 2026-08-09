# S4 for d47: the copy plan exists, and it costs 4.2× what it replaces

Lane FW. Evidence in `extractor/native/runs/fw_d47_copy_bracket/`.

The task was to convert d47's prewarm to a copy-only bracket: bulk memory copies
in the bracket, every classification, identity decode, portrait read and
admission decision after it, on the hour-labelled copy. This is what that ran
into, in the order it was found.

## 1. The substrate could not host the conversion, and three of the four gaps were table rows

`snapshot_recipes` was written before any reader was converted and says so. d47
is the first conversion. It does not fit, for four reasons; three were missing
data on an existing op and one was a mislabelled constant. All four are closed
(commits `45351a1a`, `ad31273c`).

| gap | what it blocked | closed by |
|---|---|---|
| no conditional dereference | An MSVC `std::basic_string` is a union: at capacity ≤ 15 the characters ARE the first sixteen bytes; at ≥ 16 those bytes are a heap pointer. An unguarded extent dereferences the ASCII of every short string and takes a whole-recipe `MemoryUnreadable` on the first one. d47 reads six of these per character. | `ExtentGuard { offset, width, min, max }` on `DecodeExtent` **and** `DecodePointer`; outside the band the branch ends cleanly, exactly as a null pointer ends `DecodePointer`. |
| no trailer | The string decoder refuses a buffer whose byte at `[size]` is not NUL. A recipe copying `size` bytes leaves that check reading bytes the observation never took, so the copied path would refuse every string the live path accepts. | `trailer_elements`, counted inside `max_count`/`max_bytes`, excluded from the registered element count and the per-element decode loop. |
| no terminal extent | `element: [Freeze]` pushes one decode site per element per op — one 24-byte site per BYTE of every string, over half a million per hour at d47's shape, to reach a counter. | `element: []` is legal and means the array ends the branch; accounted as one frozen branch. |
| the depth ceiling was a measurement | `MEASURED_MAX_DISCOVERY_DEPTH = 12` is t35's measurement of the chains the CENSUS followed, and it was also the ceiling on what a recipe may declare. A recipe that copies a self-balancing tree pays the tree's HEIGHT on top of the chain that reached it. | `MAX_RECIPE_DEPTH = 30`. The measured constant keeps its meaning and stays the `Limits::default()`, so nobody is opted in. |

The depth arithmetic, because it is the one that decides whether d47 is
expressible at all. The chain is manager (0) → the manager's pointer vector (1)
→ `CCharacter` body (2) → the map's `_Myhead` sentinel (3) → the tree root
through `_Myhead->_Parent` (4) → the tree's height `h` → the role object at
`5 + h` → its back-pointer or its heap string at `6 + h`. A red-black tree of
`n` nodes has height at most `2·log₂(n+1)`:

* at live widths — single-digit entries, one country-leader role per ideology —
  `h ≤ 6` and the deepest branch lands on **12 exactly, with no margin**. One
  character holding ten roles would have refused the whole recipe.
* the reader's accepted contract is `_Mysize ≤ 4096`
  (`DOMAIN47_SHADOW_WALK_MAX_NODES`), where `h ≤ 24` and the deepest branch is
  **30**.

A ceiling a legal container can cross is a bound on the world, not on the
recipe, and this substrate refuses rather than truncates — so leaving it at 12
would have converted an unusual but legal game state into a lost domain-hour.

## 2. The copy plan

`collector/src/attach_log/d47_copy_bracket.rs`. Twenty-three ops (twenty-seven
with the manager prefix), no decisions:

* `CopyHeader` 0xC0 on every `CCharacter` — wider than
  `CCHARACTER_RAW_FIELDS_SPAN`'s 0xB0 so the advisor map header at `+0xB0`/`+0xB8`
  is inside the body copy and costs no transaction of its own.
* the name (`+0x28`) and token (`+0x48`) strings as guarded, NUL-trailing byte
  extents.
* both role maps: the `_Myhead` chase **guarded on `_Mysize ≥ 1`**, then a node
  op that chases `_Left`, `_Parent` and `_Right` unguarded and the value slot
  guarded on `_Isnil == 0`.
* the role bodies, their back-pointers into the same `CCharacter` op (pruned by
  the visited set, so a body is copied once however many roles name it), and the
  advisor's idea-token and slot strings.

Three properties are worth stating because each was a decision:

**`_Parent` is chased and it is load-bearing.** `_Myhead->_Left` and
`_Myhead->_Right` are the extreme nodes and neither reaches the root; the root
is `_Myhead->_Parent`. Dropping that chase silently loses every node above the
extremes.

**The `_Isnil` guard is not defensive.** MSVC's single sentinel is allocated and
its value is never constructed, so the eight bytes a real node holds its
`CCountryLeader*` in are, on the sentinel, whatever the allocator last left
there. Today's walk never reads it — it compares each child against the head
address and stops. A recipe has no equality op, so `_Isnil` is the
discriminator. The substrate test asserts the UNGUARDED chase really does refuse
before asserting the guarded one does not.

**The `_Mysize ≥ 1` guard is worth 26,806 transactions.** Most characters hold
neither role, and `domain47_shadow_walk_map`'s own `if size == 0 { return true }`
is most of that instrument's saving. Without the same test the plan copies two
sentinels per roster character — at t32's width that was **half of everything the
observation did** (56,362 → 44,838 planned ranges, 39.2 → 26.4 ms).

## 3. Equivalence: proved for discovery and for the identity reads

On lane FU's `fu_acceptance_world`, which stages the tears, duplicate bodies,
dead-pointer objects and sweep-only stale objects the walk-vs-sweep A/B already
runs on:

```
country-leader objects: walk 11 / plan 11 / sweep 16 (of 16 in the arena)
advisor objects:        walk  7 / plan  7 / sweep 12 (of 12 in the arena)
plan copied 8224 B over 78 intervals, custody held 40968 B, 5 forward waves
```

The plan reaches **exactly** the walk's population — not the sweep's. That is the
claim that matters: the copy plan inherits lane FT's ownership argument because
it walks the same containers, so the stale-but-alive objects that could lose a
published row or invent one are outside it on both mechanisms.

The decode half, for the reads that are already memory-parameterised:

```
d47 identity reads, copy vs engine: 11 country-leader objects (2 refusing
identically), 7 advisor objects (0 refusing identically)
```

`read_character_raw_fields` and `read_advisor_raw_fields` are generic over
`LaneMemory`, so the SAME code with the SAME offsets runs over
`Domain47ViewMemory` (the frozen copy) and `Domain47InProcessMemory` (the game)
and is asserted equal — including the errors, because a dead-pointer object has
to classify dead on the copy exactly as it does live.
`d47_the_generic_identity_read_is_the_production_one` pins the generic spelling
of `domain47_country_leader_identity` against the shipped one on live memory, so
the A/B is not comparing the copy against a second implementation.

Refusal: an unreadable body refuses `MemoryUnreadable` **by name** and the
consumer then has no view at all. A gapped hour is not a partial one.

## 4. The price, and it is the result

`d47_cost_copy_plan_vs_live_discovery`, t32 widths (13,403 characters, 1,964
live + 388 dead leader objects, 6,407 live + 1,005 dead advisor objects), each
arm 7 interleaved pairs of 3, three whole runs:

| run | live discovery only | live census + discovery | copy plan | vs census+discovery |
|---|---:|---:|---:|---:|
| 1 | 6,824 µs | 6,772 µs | 28,555 µs | **+321.7 %** |
| 2 | 6,492 µs | 6,771 µs | 29,891 µs | **+341.5 %** |
| 3 | 5,918 µs | 7,194 µs | 29,019 µs | **+303.4 %** |

**The copy plan costs 28–30 ms in-bracket against a 5.9–7.2 ms live path: about
4.2×.** The S4 budget for the whole frame is 32–42 ms and d47's copy share was
meant to be single-digit ms. It is not close.

Shape: 44,838 copy tasks, 44,838 planned ranges (no coalescing — one object per
page, which is what the live heap looks like), 4.99 MB copied, 20.78 MB custody
held, 5 forward waves.

**Where it goes, measured rather than guessed.** 28.5 ms over 44,838
transactions is 618–672 ns each. The substrate's own bench
(`bench_executor_overhead_at_one_hundred_thousand_ranges`, same machine, same
session) measures **177 ns/txn of bookkeeping on a contiguous fixture with no
scatter at all**, against 7.6 ns/txn for the bare copies it wraps — a 23×
multiplier over the memory movement. The remaining ~450 ns/txn is this fixture's
one-object-per-page scatter and the guarded-read gate, and BOTH arms pay that.

Two smaller contributors, stated rather than folded in: each observation
allocates and first-touches a fresh 4.99 MB arena and ~20.8 MB of tables (order
2–3 ms at DRAM speed), and the live arm's census row reads each body immediately
before walking it, so its second touch is warm — which flatters the live arm, not
the plan.

**The lever is the substrate's per-transaction cost, not d47's plan.** The plan
is already at the floor of what it can ask for: it copies each object once, the
two guards removed the only two classes of copy that were not needed, and there
is nothing to coalesce because the objects are on separate pages. The substrate's
own bench prints a floor budget of "3–8 ms dev for 200k–360k copy transactions"
directly beneath its own measurement of 17.7 ms for 100,000 — scale that to
200,000 and it is 35.4 ms against a 3–8 ms line, i.e. **4–12× over the budget it
quotes, on its own contiguous fixture, before any scatter**. That gap is where an
S4 that fits has to come from, and it is a substrate lane, not a d47 one.

## 5. What did not move, and why

The bracket is a copy plan; the DECODE is still live-reading. That is one cause
with one shape.

d47's decode side — `domain47_probe_advisor`, `domain47_character_root_strings`,
`domain47_character_portrait_inputs`, `read_msvc_string_with_snapshot`,
`ObjectSnapshot::read`, `read_u64` — is written against **free functions that
read the live process**, not against an injected memory. `Domain47ViewMemory` is
the adapter that points a reader at the copy instead, and it works today for
every reader that already takes a `LaneMemory` (which is why §3's identity A/B
exists at all). The readers that do not are the remaining work, and it is not a
d47 change: `ObjectSnapshot`, the `read_*` family and the `read_msvc_string*`
family are the collector's shared read layer, used by every one of the 73
domains. Threading a memory parameter through them is a migration and belongs to
its own lane with its own owner sign-off.

Also not covered by the plan, each for a stated reason:

* **Portraits.** The map header at `CCharacter+0x80`/`+0x8C` is inside the body
  copy; the element array and its strings are not in the plan. They are the same
  guarded-extent shape as the character strings and are a bounded addition — they
  were left out only because they cannot be equivalence-tested until the portrait
  reader takes a memory parameter (see above), and an untested copy is worse than
  an absent one.
* **The `CUnitLeaderTrait` definition table.** Lane FU left it on the shared heap
  sweep because it hangs off no character and its owning container has not been
  found. Its ~194 objects are addresses the sweep already publishes, so under a
  copy plan they are recipe ROOTS rather than a walk — the cheapest shape the
  substrate has, one `CopyHeader` plus one guarded string extent per object,
  about 400 transactions against the plan's 44,838. It is deliberately not added
  here: it would put a second family into a plan whose price already fails, and
  it is the only thing that would need the recipe rebuilt per hour (the roots
  change with the sweep's generation) rather than built once.

  What it COSTS to leave it there is unchanged by this lane and is one thing: d47
  keeps a dependency on the shared sweep for one family, so a cold or
  coverage-incomplete pass costs the TRAIT family and only that family — which is
  the state lane FU already reduced it to (`SweepCoverageIncomplete` /
  `SweepNeedlesNotWarmed`, named on the wire, with the other seven families
  published). It adds no per-hour work to the bracket, because the sweep is a
  separate service and d47 only reads its published hit list.

## 6. Decisions this lane did not take

* **Whether to fund the substrate's per-transaction cost.** §4 is the number; the
  target is roughly a 4× reduction in the 177 ns/txn bookkeeping before d47's
  plan fits its share of the frame.
* **The torn-zero `_Mysize`, which is the same decision wearing a different
  hat.** A size word that tears to zero over a non-empty map ends the copy
  branch, and the post-bracket walk reads the same torn zero out of the copy and
  calls the map empty — publishing the family a role short. Today's walk reads
  the same zero and returns the same true; what catches it is the CLOSING header
  re-read, per axis, per character, mid-walk.

  Removing the `_Mysize >= 1` guard does not close it: it would copy the
  sentinel, but the `_Myhead->_Parent != _Myhead` test that would then be
  possible does not exist today either, so adding it is its own behaviour
  change — and it costs the 26,806 transactions back for nothing on its own.
  What closes it is a closing image the consumer may compare FIELD BY FIELD,
  which is §7 row 7 and FX's finding 7. The other direction (a size tearing
  nonzero over an empty map) is already equivalent: the sentinel is copied and
  `visited != size` refuses the axis, exactly as today.

* **`WitnessClose` and the double read.** Today each role identity is read twice
  and a disagreement costs ONE root (`skipped_root_rows`). The substrate's
  equivalent gaps a whole consumer, and it compares raw bytes where the live
  check compares the decoded `(country, char_id)` pair while deliberately
  excluding the resolved body address. The plan therefore does not enrol a
  witness, and what replaces the double read is an open decision. (Codex's Q5
  adds a correction worth carrying: any nonzero `skipped_root_rows` already
  blocks the whole freshly built prewarm, so the live path's final outcome is
  coarser than the per-root loop suggests.)
* **Refusal granularity.** A recipe refusal gaps its whole consumer, where today
  a refused map costs one FAMILY for the hour. Preserving family granularity
  needs one recipe per family (4–8 recipes, cheap) — but per-character recipes
  are not available at all, because `observe` takes the whole recipe slice before
  execution and characters discovered from the manager vector cannot become
  recipes in the same observation.

* **The heap-window gate, and the one guard slot.** Today every node and value
  pointer passes `is_heap_pointer_aligned` before it is read, and a pointer that
  fails costs THAT MAP — one family for the hour. The copy plan has no such gate,
  so a node pointer outside `[HEAP_LO, HEAP_HI)` reaches `read_into`, fails
  `range_confirmed_good`, and refuses the WHOLE RECIPE. That is a granularity
  regression on a path today handles at family scale.

  It is expressible and almost free: `ExtentGuard` is a BAND, so
  `{ offset: <the pointer's own offset>, width: 8, min: HEAP_LO, max: HEAP_HI-1 }`
  is exactly the gate, and an out-of-window pointer would then end the branch
  without copying — after which the post-bracket `visited != size` check refuses
  that map and costs that family, which is today's outcome exactly.

  Lane FX reached the same gap from d81 (its finding 3) and its shape is better:
  an admissibility descriptor on the pointer fields that stops the branch **and
  retains the source bytes**, so the post-bracket reader can construct the exact
  typed refusal instead of inheriting a generic one. Whoever funds this should
  build FX's version rather than the sketch below. See §7.

  What stops the cheap version is that a decode op carries ONE guard. The three
  link chases (`_Left`, `_Parent`, `_Right`) are unguarded and could take it
  today; the map head already spends its slot on `_Mysize >= 1` and the value
  chase spends its
  slot on `_Isnil == 0`. Closing it properly means guards become a CONJUNCTION —
  a small fixed array rather than an `Option` — which is a second substrate
  change and wants its own review round rather than being appended to this one.

## 7. Against lane FX's nine gaps — the two conversions converge

FX ran the same premise check for d81 and found nine substrate gaps
(`runs/fx_d81_copy_bracket/substrate_expressibility_reply.txt`, merged at
`31aed979`). d47 is a different shape — a pointer forest, not a hash-indexed
table — so which of the nine it hits is a real test of whether that list is the
whole list.

| FX gap | d47 |
|---|---|
| 1. count expressions / hash probes | **avoided** in the plan. Both manager rosters are `{data, cap, size}` vectors — one decoded count, fixed stride, exactly `DecodeExtent`. |
| 2. cross-wave join context, instance-scoped tables | **avoided.** Every key d47 needs is a field of the object at the decode site; the plan registers no tables at all. |
| 3. pointer admissibility | **hit.** §6's heap-window gate is this gap, found independently. FX's shape — an admissibility descriptor that stops the branch while RETAINING the source bytes for the post-reader — is strictly better than the conjunction-of-guards note in §6, because it also lets the post-bracket reader build the exact refusal. |
| 4. superset over-read is fatal | **hit, and it is d47's central one.** The plan copies a superset BY DESIGN: it follows tree links without the `_Isnil`/`visited < size` bounds so the post-bracket walk can apply them. `d47_the_copy_plan_refuses_an_unreadable_body_by_name` is that behaviour pinned — one unreadable object gaps the whole consumer, where today it costs one family. |
| 5. adjacent A/B double reads | **hit, four times over.** Both role admissions double-read their identity; `domain47_probe_advisor` double-reads; `read_stable_msvc_string_snapshot` double-reads; `domain47_read_manager_vector_entries` bulk-copies each chunk TWICE and compares. Duplicate tasks are pruned, so none is expressible. |
| 6. conditional fallback read shapes | **hit.** `domain47_read_manager_vector_entries` falls back from a bulk chunk to per-entry reads on failure OR disagreement, and counts the fallback (`manager_vector_chunk_retries`); `read_msvc_string_with_snapshot_with_policy` falls back to by-address reads; `LaneMemory::read_array` falls back byte-by-byte. All are schedules conditioned on a copy's outcome. |
| 7. field-granular closure, typed torn witnesses | **hit, and d47's instance is sharper than d81's.** `domain47_shadow_walk_headers_describe_the_same_map` compares TWO WORDS of a 40-byte header per axis and documents why it must not compare the whole snapshot: `CCHARACTER_UNIT_LEADER_PTR_OFF` is at `+0xA8`, squarely between the two map headers, so a byte-for-byte comparison reads a perfectly legal unit-leader assignment as a role-container tear. d81's case is four dead bytes; d47's is a live production field that changes normally. `domain47_classify_leader_identity` is the same shape again — it compares the decoded `(country, char_id)` pair while deliberately excluding the resolved body address. |
| 8. closure order and elision | **hit, mildly.** d47's closing header re-read happens per character, mid-walk, inside the census loop. One final closing wave cannot place it. |
| 9. per-attempt failure provenance | **hit.** `Domain47PortraitFailureSite`, the manager roster's `character_locator`, and `refused_first_body`/`refused_first_axis` are typed first-failure witnesses that `Refusal::MemoryUnreadable { recipe, span }` cannot carry. |

**d47 hits seven of the nine and avoids two. It adds four that d81's shape does
not reach — and all four were pure mechanism, and all four are now closed:**

1. **Union-arm discrimination.** Nothing in FX's list is a conditional
   dereference, because d81 has no union-shaped field. d47 has two: the MSVC
   `std::basic_string` capacity and the `_Tree` sentinel's `_Isnil`. Closed by
   `ExtentGuard`, on both `DecodeExtent` and `DecodePointer`.
2. **Trailer bytes past a declared count** — the NUL the string decoder tests
   for. Closed by `trailer_elements`.
3. **Terminal byte extents.** A byte-strided array with per-element decode ops
   pushes one site per byte. Closed by allowing `element: []`.
4. **Recipe depth versus container height.** The ceiling was t35's census
   measurement, and a self-balancing tree pays its own height on top. Closed by
   `MAX_RECIPE_DEPTH`.

That two independently-shaped domains land on the same seven blockers — with
d47's four additions all falling on the mechanism side of the line — is the
strongest available evidence that FX's list is complete rather than d81-specific.
The four owner questions FX raised (over-read fatality, byte-wise versus
field-masked closing comparison, value-conditional next-copy selection,
per-domain closure order) are the same four that block d47, and d47's §7 row 7
is an additional argument for the field-masked one.

## 8. The hand-built-bracket fallback does not apply to d47, and the number says why

FX's second suggestion is d81's own pattern — `d81_read_stable_bytes` derives a
whole-table extent from the engine's header, bulk-copies, double-reads, compares
images, then decodes — applied by hand to d47's biggest batchable reads.

**d47's biggest batchable read already is that pattern.**
`domain47_read_manager_vector_entries` copies the roster vector in 
`DOMAIN47_MANAGER_VECTOR_COPY_ENTRIES`-entry chunks, twice, compares the two
images, and falls back to per-entry reads on disagreement — the same
derive-extent / bulk-copy / double-read / compare shape, already shipping.

And there is nothing else to batch, which is measurable rather than asserted:

```
copy plan shape: 44838 copy tasks, 44838 planned ranges
```

Every task produced exactly one copy transaction. The coalescer — which merges
inside a proven object at 64 B and inside a decoded extent at 128 B — found **not
one pair of ranges it could merge** across the whole observation. That is the
structural difference between the two domains in one number: d81's hub is a
TABLE, so batching it is 2.97× fewer guarded calls at 4.4× the bytes; d47's
roster is a POINTER FOREST of 13,403 independently allocated bodies, each with
its own tree and its own strings. There is no second `d81_read_stable_bytes`
hiding in it, because there is no second table.

That is also the honest explanation of §4's price. The copy plan does not lose
because it copies badly; it loses because it adds per-transaction bookkeeping to
a workload that is already one cold miss per object and cannot be made into
fewer, larger transactions.

## Review

Codex `019fe3ba-1db6-7e32-84d8-16751d1d9a99` (premise: depth, tree traversal,
strings, refusal granularity, the double read) and
`019fe3eb-0646-7bd1-9881-d89a80c83a24` (the diff, two rounds). All replies are in
`extractor/native/runs/fw_d47_copy_bracket/`.

Round 1 was REQUEST CHANGES on two findings, with five categories clean. The
vacuous-test finding was real and is fixed — the population A/B named two
mutations and caught neither, because the acceptance world's maps are too small
to hide the root and its sentinels carry a zero value slot. The replacement
fixture was verified by applying both mutations and observing each fail
(`mutation_verification.txt`).

Round 2 converged: the vacuous test is CLOSED, and the torn-zero `_Mysize`
divergence is STILL OPEN with the disposition confirmed correct — *"there is no
existing drop-in closing step preserving the live field-by-field,
per-character/per-axis behavior; doing so would require owner-approved
interval/consumer repartitioning or a new selective closing-image guarantee."*
That is §6's item and §7 row 7, and it is the owner's.
