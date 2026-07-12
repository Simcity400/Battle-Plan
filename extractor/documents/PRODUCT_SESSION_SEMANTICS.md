# Product session semantics (owner's vision of record — 2026-07-12)

Owner's sanity-check statement, recorded verbatim in intent (source: session
2026-07-12, direct quote paraphrased):

> You boot it up and then start playing the game. It automatically understands
> which game and which save is attached to it. It saves the progress and spits
> out the replay file at the end. It doesn't matter if you have a reload, or if
> you close the game down and reopen it with an old save — it will try to
> stitch everything together so it looks normal. Settings should be bare
> bones: plug and play, minimal changes to the actual app.

This is the UX contract the extractor is being built toward. Status per pillar
(honest, as of 2026-07-12):

## 1. Boot-and-play (auto-attach) — NOT BUILT YET (packaging layer)
Today the recorder attaches to a running hoi4.exe by PID via dev tooling.
OWNER DECISION 2026-07-12: BOTH launch orders must work, symmetrically —
(a) app first: it waits/watches and attaches when the game launches;
(b) game first: booting the app mid-session finds the running game and
attaches immediately. "It always works" regardless of order. No design doc
yet — needs one covering both arms (watcher loop + running-process discovery,
which the modset resolver's PID plumbing already half-provides).
Constraint (standing): must work on arbitrary end-user machines (any install
location, any Steam library, OneDrive-redirected user dirs) — everything
probed at runtime, nothing hardcoded (memory: portability-no-rig-hardcoding).

## 2. Auto-identify game/build/modset — LARGELY BUILT (the modset binding)
The resolver auto-binds exe hash/size/path, Paradox user dir (probed,
PID-ownership-corroborated), enabled mods (descriptor+content sha256), and the
DLC census — zero configuration, fail-closed on any uncertainty. This is the
G2/modset workstream, currently under its final adversarial gates.

## 3. SAVE/CAMPAIGN identity — GAP (required for pillar 4)
"Which save is attached" is NOT yet bound. Stitching across sessions requires
recognizing that two recording sessions belong to the same campaign (and where
they overlap). Needs an engine-derived campaign identity (e.g. game/session id,
start scenario + seed, or save-file lineage) read from live memory and stamped
into every stream. NOT designed yet — prerequisite for pillar 4.

## 4. Reload / relaunch stitching — FORMAT ANTICIPATES IT, LOGIC NOT BUILT
- The snapshot container already round-trips SEGMENT-PREFIXED ROLLBACK
  filenames (snapshot/src/container.rs, pack.rs; tests pin both filename
  forms) — the file format was designed for branch/rollback timelines.
- The archived RECORDING_LAYER_COMPLETION_PLAN carries the intended semantics:
  time rollback creates a NEW SEGMENT rather than corrupting the index;
  gaps/rollback spans surfaced, never silently blended.
- The live-capture harness already detects continuity breaks (time_rollback,
  time_jump_forward, pid change) diagnostically.
- The CURRENT recorder fold has NO segment creation on rollback and NO
  cross-session (relaunch with older save) stitching. This is the largest
  unbuilt pillar.
- POLICY CONFIRMED BY OWNER 2026-07-12: whatever comes LATER (in wall-clock
  play order) supersedes whatever came prior on conflict. Canonical example
  (owner's, multiplayer): game crashes; the newest save anyone has is a month
  of in-game time older; the group reloads it and replays that month — the
  REPLAYED month is the main continuity, the pre-crash original of those
  hours is superseded. Superseded spans ARE STILL SAVED (zero-data-loss law:
  retained as rollback segments in the container, visibly superseded) — they
  just don't form the main replay timeline. Playback follows the last-played
  branch end to end.

## 5. Bare-bones settings / plug-and-play — ALIGNED BY DESIGN
Zero-config probing with optional overrides only (e.g. --pdx-user-dir); all
correctness comes from fail-closed evidence, not user tuning. Any future
setting must justify itself against this contract.

Every new session-lifecycle/stitching spec MUST cite this doc; changes to the
vision get recorded here in the same commit.
