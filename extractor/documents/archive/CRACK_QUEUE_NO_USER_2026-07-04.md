# Crack queue — everything still crackable WITHOUT the user (2026-07-04)

> **RESOLVED — see `CRACK_QUEUE_RESULTS_2026-07-04.md`.** C1 (market owner,
> `CCountry+0xFB8`) and C3 (diplo queue, **`CCountry+0xFF8`** — the +0x4E0 root
> named below was WRONG) are CRACKED + independently verified. C2 (speed) and C4
> (intel graph) DEFERRED with precise negatives. The brief below is the original
> task list, retained for provenance.

Every reverse-engineering item that does NOT need in-game action from the user.
One Fable discovery agent works this list top to bottom. DISCOVERY ONLY: no
tracked-source edits, no commits, no wiring — write evidence to gitignored
`extractor/native/runs/crackqueue_*/`. Each crack: pin the offset/chain, VALIDATE
against the same-tick melt oracle, then REFUTE with Codex before claiming it.

Ordered easiest → hardest. C4 (speed) also has a 10-second user fallback (fixture
B0), but TRY the disassembly path first — that's the whole point of this queue.

## ENVIRONMENT (verify first)
- Worktree: `C:\Users\Example\OneDrive\Documents\Paradox Interactive\Hearts of Iron IV\mod\battle-plan\.claude\worktrees\elated-wiles-1be9d2`.
- `hoi4.exe` PID 28044, build SHA gate `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`, save D04 1944.2.11.18 loaded + PAUSED. Resolve PID: `powershell -NoProfile -Command "(Get-Process the game).Id"`.
- Melt oracle (94MB HOI4txt ground truth): `extractor/native/runs/division_manpower_assignment_870ebf_20260703_wave2/00_d04_1944_melted.txt`.
- Python: `PYTHONPATH=extractor/memory_reader`; primitives in `events/probe_common.py` (`open_process_with_sha_or_report`, `resolve_country_roots`, `try_u8/u32/u64`, `heap_ok`, `rtti_name_for_vtable`, `rediscover_rtti_by_terms`, `read_module_chunks`). `winmem.Process` has `scan_module(pattern)`, `rip_target(instr,disp_off,len)`, `iter_regions`, `in_module`. **capstone 5.0.7 IS installed** (`import capstone`) — use it for the disassembly items.
- All offsets below are RVAs off the module base unless said otherwise.

## C1 — Market manager owner chain (disassembly)  [unblocks market wiring — HIGH value]
The stockpile LAYOUT is fully validated (70/70): `CEquipmentVariantPool` vtable
`0x27330A0`, entry array begin @pool+0x20, packed u64 @pool+0x28 (count = hi32,
capacity = lo32), entries stride 16 = {u64 variant_ptr, **i64** amount_fx100000},
valid entry = variant_ptr→vtable `0x293AFB8` (CEquipmentVariant, id u32 @+0x0C).
What's MISSING is the per-country owner: production has no oracle, so it cannot
attribute a pool to a country by content. The pool objects have ZERO incoming
heap pointers and are not a contiguous array (`pool-0x18` is a coincidental
neighbor — REJECTED, do not resurrect it).
- TASK: find the market manager's per-country pool accessor. Disassemble the
  code xrefs to pool vtable `0x27330A0` (scan `.text` for the RIP-relative
  `lea reg,[rip+disp]` that targets `module+0x27330A0`; ~23 sites). Walk back to
  the manager object + the index/map that hands out a pool per country. The
  manager likely hangs off `CCountry` or a global market singleton.
- GROUND TRUTH: `runs/attemptcrack_market_close/full_pool_to_tag.json` (the
  content-derived 70/70 pool→TAG map). A correct owner chain must reproduce that
  exact map by pointer, for all 70, with no oracle.
- DONE = a pointer-only chain (CCountry+off → … → this country's pool, OR
  manager+index[country]→pool) that reproduces full_pool_to_tag.json 70/70.

## C2 — Game-speed root (disassembly; user fallback exists)  [unblocks R4]
Pause is SOLVED and live in D57: `CInGameIdler` singleton at `module+0x33172E0`
(mirror +0x33172E8, vtable `0x2952510`), `is_paused` u8 @idler+0x6C1. The speed
field is NOT a clean 0..4 int adjacent to the pause byte (probed: the `04` at
+0x6B9 is a heap-pointer fragment; a full idler+0x800 scan found only generic 1s
and a `100 @+0x66C` ms-ish value). Snapshots banked: `runs/speed_root_probe/`
(idler_snapshot_paused.bin, globals_window_0x3316000.bin, meta.json).
- TASK: disassemble. The idler's per-frame update reads the speed to compute the
  tick delay; find the field it loads. Xref the idler vtable / the pause byte
  `+0x6C1`, or find the speed hotkey/command handler (search `.text` near the
  pause toggle) and see which field it writes 0..4. the game speed is 0(paused-ish)
  through 5; the stored field may be an index 0..4 or 1..5, or a float tick
  interval (fastest speed ≈ smallest interval).
- ORACLE: none on a paused save (speed UI value unknown to us). So VALIDATE
  structurally: the field must be in the idler or a global the idler's update
  reads, with a plausible small-int/interval domain, AND be the operand of the
  tick-scheduling arithmetic. If you cannot pin it structurally with confidence,
  say so and defer to fixture B0 (user taps speed once → diff the banked
  snapshots). Do NOT guess-promote.
- DONE = either a structurally-proven speed field (disasm shows it drives tick
  cadence) OR a clean "needs B0 diff" writeup naming the ≤3 candidate offsets.

## C3 — Diplomatic-action queue payload decode  [root already pinned]
Root FOUND: the per-country pending queue is `CCountry+0x4E0` on the RECIPIENT
country (a PdxMap/hash, rtti None). Melt id 150442 was located inside CZE's
+0x4E0 container at +0x3CC, preceded by handle {id, type=4713}.
- MELT GROUND TRUTH (concrete, verified): CZE's `incoming_diplomatic_action`
  map index 0 = `{ id={id=150442,type=4713}, market_access_rights={ type=0,
  actor="GER", recipient="CZE", date=1944.2.3.1, value=yes,
  last_command_date=1944.2.3.1, envoy=-1 } }`; index 1 = id 150608 actor ITA;
  the ITA one (id 150443 actor GER) sits in another recipient's map. Many
  countries have empty `incoming_diplomatic_action={ }`; CZE (and the ~others
  with 685 total `market_access_rights`) are the populated fixtures.
- TASK: decode the PdxMap at CCountry+0x4E0 → entries, and each entry's payload
  {actor tag_db_id, recipient (= owner country), action-type token, date}.
  The entry starts with the {id, type=4713} handle; the action-type-specific
  body (market_access_rights here) carries embedded tag fragments rather than
  clean adjacent tag_db_ids — reverse the actor/date fields off the handle.
  Walk the hash structure generically (don't assume a vector).
- DONE = for CZE (and ≥2 more populated recipients), read {id, actor, date}
  matching the melt exactly for every queued action; document the entry stride
  + field offsets + the map-walk.

## C4 — Intel network graph / intel-map %  [deepest unknown — container not found]
The melt has 354 `intel_network` blocks (per (owner,target) pair): each is
`{ target="ENG", vertices=N, edges={…}, state_id={…}, gain={…}, … }`. The
spy-network graph container did NOT surface under the RTTI terms tried
(CIntelNode-like names). This is the one item with no pinned root yet.
- MELT GROUND TRUTH: e.g. owner-with target="ENG" has vertices=14, state_id list
  {126 127 125 338 129 123 128 122 130 132 131 931 121 133}; target="SOV" has
  vertices=43 with a 43-entry state_id list. These vertex counts + state_id sets
  are strong fingerprints to match a live container against.
- TASK: rediscover the container. Try broader RTTI terms (Intel, Network, Graph,
  Vertex, Node, Cryptology, Agency). Failing RTTI, content-scan the heap for the
  distinctive state_id integer runs above (e.g. the 14-int ENG run or 43-int SOV
  run as an AOB of little-endian u16/u32) and walk back to the owning object;
  then find its root off CCountry (likely near the intelligence_agency the
  operatives already use) or a global intel manager.
- DONE = a live container whose vertices + state_id set reproduce a melt
  `intel_network` block for ≥2 (owner,target) pairs, with the root chain
  documented. If genuinely not found, a precise negative with everything tried.

## OUT OF SCOPE for this agent (handled elsewhere — do NOT touch)
- Naval supremacy port (N2), cosmetic_tag wire (N1), localisation bundles (N4),
  naval fold verify (N5): already solved/queued, not cracks.
- The 7-packet wave-4/5 WIRING queue is running in another agent — do NOT edit
  any tracked source, especially the shared hot files
  (`collector/src/lib.rs`, `recorder_host/src/{main,world_fold}.rs`,
  `compact_contract/src/lib.rs`). You are discovery-only.
- `replay_interface/` is another owner's — never touch.

## PROTOCOL (mandatory)
- Codex is the adversarial reviewer; on the Bash PATH; read
  `C:\Users\Example\.claude\CODEX_PROTOCOL.md`. Refute EACH crack before claiming
  it: `printf '%s' "$PROMPT" | codex exec --ephemeral -s read-only -c approval_policy=never --skip-git-repo-check -c model_reasoning_effort=xhigh -o /tmp/cdx_<uniq>.txt -`.
  Confirm exit 0 AND `-o` non-empty; unique `-o` per call; NEVER `resume --last`.
  Codex has intermittently failed with network `os error 10013` — if it does,
  SAY SO; never claim a review that didn't run.
- Fail closed BOTH directions: never promote an unproven offset; absent != 0.
- Per crack, write `runs/crackqueue_<item>/EVIDENCE.md` with: the chain/offsets,
  the melt validation (exact numbers), the Codex verdict, and DONE/DEFERRED.
- Report back a single summary: per item CRACKED (with offsets) / DEFERRED (why +
  what's needed). This list is what remains between us and V1-without-user.
