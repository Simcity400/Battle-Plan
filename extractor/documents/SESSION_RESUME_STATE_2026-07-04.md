# Session resume state — 2026-07-04 (pre-compaction snapshot)

Read this first after a compaction. Captures what is RUNNING, what is DONE, what is
NEXT, and the operating protocol so nothing is lost.

## ENVIRONMENT (verify before any live work)
- Worktree: `C:\Users\Example\OneDrive\Documents\Paradox Interactive\Hearts of Iron IV\mod\battle-plan\.claude\worktrees\elated-wiles-1be9d2` (branch `claude/elated-wiles-1be9d2`, off `master`).
- `hoi4.exe` running, PID was **28044**; build SHA-256 gate = `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`. Save `D04_1944_02_11_18.hoi4` LOADED + PAUSED (1944.2.11.18), rich wartime save (4 active naval battles, 88 delayed events, operatives, garrisons). Resolve PID: `powershell -NoProfile -Command "(Get-Process the game).Id"`.
- Same-tick melt oracle (94MB HOI4txt, grep for ground truth): `extractor/native/runs/division_manpower_assignment_870ebf_20260703_wave2/00_d04_1944_melted.txt`.
- Python probes: `PYTHONPATH=extractor/memory_reader`. Rust workspace: `extractor/native`. `extractor/native/runs/` is GITIGNORED (evidence; read by absolute path).

## RUNNING NOW (background)
- **Cracking agent `a2da727701ea4d8d7`** (Fable, discovery-only) — resumed on the THREE not-user-blocked deep-reverse items: (1) operations/spy-intel-map/diplomacy-action-queue containers (domain 53), (2) market-stockpile `CCountry->CEquipmentVariantPool` owner chain (reach 70/70 vs market.json), (3) projects/rewards per-country container for owner-join. Writes evidence to `runs/attemptcrack_*/`, NO source edits, NO commit. Resume/continue via `SendMessage` to that agentId. It will notify on completion; VERIFY its cracks (offsets vs oracle) + capture into a doc before any wiring.
- No other subagents running. Wiring agent `a11abf9bdb027a90c` is DONE (all 3 domains landed + committed).

## GIT STATE — HEAD `3dfcd2776`, working tree CLEAN (only gitignored runs/ evidence untracked)
Session commits (newest first), all gated (my own `cargo test --workspace` + independent adversarial Codex gate + paused-capture emission proof + `pack_replay verify` before each commit):
- `3dfcd2776` Wave-4 attempt-crack evidence (6 cracks + needs-user list) — doc only
- `02ea83faf` Wave-3 wiring 3/5: d47 advisor/traits + division-order + delayed-event
- `c4b60e674` Wave-3 wiring 2/5: garrison scalars + air cumulative counters
- `bb725b054` Wave-3 reverse cracks (6 items evidence: d47/garrison/air/division/delayed/market)
- `f68a6deed` Naval 49/50 production integration (emits+folds+verifies)
- `55392d989` d47 correction (offsets validated live, not broken)
- `aa7336ed3` Event-hooks delayed-event decode + dispositions
- `28cfcc826` Naval 49/50 port (CFEXGroup chain, live gate passed)
- `6be6319b4` Wave-2 naval crack + division-oracle robustness + fixture batch
- `a97f92a48` Domain 53 discovery cache (killed 7.4s/frame blocker)

## DONE — in production (emitting + folding + replay-verifying on paused D04)
- **d53** per-frame cost fix (discovery cache).
- **Naval 49/50** (lifecycle+members) fully wired.
- **All 5 wave-3 domains wired**: garrison scalars (domain 44), air cumulative counters (domain 42: air_accidents +0x74, experience +0x208), d47 advisor+traits (domain 47: 0->26,946 rows), division->primary-order (domain 20), delayed-event rows (domain 51, incl due_remaining_hours CDelayedEvent+0xC4).
- Dispositioned (proof-backed partial rejections): weather causal internals, mission/decision scripted state, on-action/effect runtime, direct focus/decision events.

## NEXT — priority order
1. **When cracking agent `a2da727701ea4d8d7` returns**: verify its 3 results (ops/market/projects) vs oracle + Codex-gate; append to `WAVE4_ATTEMPTCRACK_870EBF_D04.md`.
2. **Wire the 6 wave-4 cracks into production** (same process as wave-3: lane compact record + collector read/schedule + recorder decode + world_fold + tolerance + tests + paused-capture proof + my adversarial gate). Validated offsets in `extractor/documents/WAVE4_ATTEMPTCRACK_870EBF_D04.md`:
   - Naval missions/home-base/repair (CTaskForce+0x364 mission, +0x1E0 location, CFleet+0xD8 home_base, +0x4B0 repair_target, etc.)
   - Ace identity+wing-assignment+death (CAceEntry +0x0C/+0xF0/+0x158/+0x164/+0x168; CAirWing+0x2DC assigned ace)
   - Production licensing (CCountry+0xF68 -> mgr+0x198 variant vector)
   - Disrupted supply (CDivision+0xA0 i32/100000)
   - Attrition supply signals (CDivision+0x600, +0x5F8)
   - (delayed-event due-date already wired in commit 02ea83faf)
   Delegate to a Fable subagent using commit `02ea83faf` / `f68a6deed` as the template; ONE domain at a time, green after each.
3. **USER IN-GAME FIXTURES** — the only genuinely user-blocked items left (short list):
   - Set ONE supply hub to a non-2 motorization level (currently all level 2) -> unblocks hub motorization level offset.
   - Pause ONE national focus (none currently paused) -> unblocks focus-paused bit.
   - Have live AIR + NAVAL combat actively running -> unblocks air kill attribution + convoy combat losses (need combat ticks).
   - Create a naval-interdiction / blockaded-convoy situation -> unblocks convoy/port pressure.
   - One supply-map tooltip screenshot -> exact province remaining-supply (UI-computed, not saved).
   - (MP-only, needs 2nd player: chat/messages, hotjoin/tag-switches.)
   Full batch: `extractor/documents/USER_FIXTURE_BATCH_2026-07-03.md`.
4. **Resources/factory totals** = replay-side DERIVATION (proven no stored offset) — reconstruct from domain-23 state buildings + trade, not a memory crack.

## OPERATING PROTOCOL (do not skip — it repeatedly caught real defects)
- **Codex is always-on adversarial reviewer** (`C:\Users\Example\.claude\CODEX_PROTOCOL.md`). Mode A: `printf '%s' "$P" | codex exec --ephemeral -s read-only -c approval_policy=never --skip-git-repo-check -c model_reasoning_effort=xhigh -o /tmp/cdx_<uniq>.txt -`. Confirm exit 0 AND `-o` non-empty. Codex (OpenAI) is INDEPENDENT of Anthropic 529s but has had intermittent `os error 10013` network blocks — if blocked, say so, don't claim a review that didn't happen.
- **Every subagent result: I verify independently before commit** — my own `cargo test --workspace`, scope check (`replay_interface/` untouched, four shared hot files integrated one packet at a time), read key diff hunks, paused-capture emission proof, `pack_replay verify`, THEN my own adversarial Codex gate. Fix findings (mine directly, per the one-subagent-fix-round escalation cap), re-gate, commit. This gate found real fail-open/gate-breaking defects on EVERY wiring round.
- **Paused-capture validation**: `collector_record.exe --pid <PID> --dll <collector.dll> --out-dir <dir> --duration-s 25 --sample-ms 0 --scheduled-domain <lane> --stream-format compact --extraction-perf-gate`. The `minimum_game_hours_not_reached` gate failure + `native_session.failed.battleplan` naming are EXPECTED on a paused game (clock doesn't advance) — NOT a real failure; check `compact_stream.per_domain.<lane>` rows>0 + no worker_panic/source_unavailable, and `pack_replay verify` OK.
- **OneDrive gotcha**: the `Edit` tool intermittently fails with "modified since read" because OneDrive touches file mtime. Workaround: use `sed`/python for contained edits, or re-Read immediately before Edit. Content is stable; it's an mtime artifact.
- **Subagents**: Fable per user directive (`codex-implements-directive` is overridden — Fable subagents do delegated work). Two concurrent Fable agents burn quota faster (hit the account Fable limit once) and can trip transient 529s; both bank progress incrementally so nothing is lost. Discovery agents write only to gitignored `runs/` (no source collision); wiring agents edit shared source (verify hard).
- Commits omit AI-attribution trailers (project rule).
