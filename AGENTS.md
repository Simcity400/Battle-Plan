# Repository agent guidance

## Product (owner)

- Four goals, everything else is cost: perfect clean captures; smallest size
  (≤50 MB compressed per recording); plugs straight into the replay layer;
  minimal performance hit. Simple to use.
- A bad hour costs exactly that hour, with a named reason — never the recording.

## Capture rules

- Never make the game wait: no pausing, no synchronous work inside hooks, no
  freezing threads on the simulation path.
- Capture is attach-and-play. The operator never touches the client for the
  recorder's benefit — no opening panels, no timed actions, no staged moments.
  Find every value's authoritative location by reverse engineering and read it
  passively. If a value only exists after client interaction: find its
  upstream source, derive it at replay, or refuse it with a named reason.
- Fail closed: an unproven value is refused with a named reason on the wire.
  Never publish data under the wrong hour.
- Assume max game speed (48 ms budget per hour). No extra worker threads.
  Must run on 4 cores / 16 GB.
- No hardcoded game content, ever. Engine constants need reverse-engineering
  proof. Size caps must be proven structural or discovered at attach; overflow
  refuses loudly. Mods must just work.

## Code rules

- Pre-production: no backwards compatibility. Old captures are disposable;
  formats change cleanly.
- Delete retired code in the same change, with proof it is dead. Dead code
  hides here behind: Rust spawning Python, dot-sourced PS1 files, sys.path
  injection, relative imports, constructed paths. Run the full suite before
  and after a purge and compare pass counts.
- Docs state current intent only; history lives in git. Evidence under
  `extractor/native/runs/` is append-only. Force-add only your own run
  directory (`git add -f extractor/native/runs/<yours>`), nothing broader.
- Every test carries a one-line `kills:` comment naming the production change
  that would fail it. No empirical mutation testing. Don't delete a test
  without proving it checks nothing unique.

## Working method

- Clean up when done: retire your worktree and delete your branch as soon as
  your merge is pushed. Your build output belongs in your own checkout's
  `.cargo-target` and dies with it. Delete your temp files when you finish.
- At lane start: lock the worktree, make an empty commit, and check your
  merge-base against the current origin tip.
- Git: never stash (shared repo). Fetch and compare both master refs before
  any push. Never commit a file over 99 MB.
- Finish your review loop before reporting, and report once. Never end a turn
  while a review or build is still running — it gets killed. Don't poll a
  child process; bound the call and report a hang as a finding.
- Lead with the deciding number or table. Give reviewers raw evidence paths,
  not summaries.
- Measurement: state the basis (per call, per frame, or per record). Compare
  A/B inside one process only. Re-measure baselines on the current tip. A
  busy machine hides loss — measure against a free-running game.
- To prove something is absent: `git grep <pattern> HEAD -- <scope>`.
  "Couldn't determine X" must name the one step that would determine it.
- Fix what you find. A leftover is only legitimate if it (a) needs an owner
  decision, (b) needs a live game session, (c) belongs to another active
  lane, or (d) is outside your assigned scope. "Nobody's working on it" is
  not a reason — fix it, test it, get it re-reviewed.
- At most 10 lanes at once; queue beyond that.

## Delegation

- Lanes do their own research, reverse engineering, code, and tests — no
  child agents, except exactly one read-only reviewer, started only after the
  work is complete and validation is green. The same reviewer re-reviews
  after every fix, including comment-only fixes, until it approves cleanly.

## Ghidra (static analysis)

- Call `ghidra_status` first. The broker is read-only: 20 shards, image base
  `0x140000000`, SHA-256 `870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad`.
  Never reconfigure or reset it.
- Search the index before exporting (`ghidra_coverage`, `ghidra_search_*`,
  `ghidra_functions_for`); export only what the index doesn't already have.
- Record conclusions with `ghidra_annotate_function`, or the work gets paid
  for twice. Cite `ghidra_verify`'s identity before publishing a claim.
- Never edit files inside a completed job directory; derive into a new job.
- Guide: `replay_interface/tools/ghidra/README.md`.

## Live analysis (`hoi4_re`)

- Start with `re_status`. Use IDs, not raw paths or addresses. Live reads are
  read-only; hook-based traces are never promotion evidence on their own.
- Guide: `replay_interface/tools/hoi4_re/README.md`.

## Builds and tests

- One validation runner, run serially — never invoke cargo directly against
  shared trees or start competing runners:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File <your-checkout>\extractor\native\scripts\Invoke-CentralRustValidation.ps1 -Mode Final -Workspace All
```

- Run YOUR OWN checkout's copy of the script — it validates the tree it lives
  in. The main repo's copy silently validates the main repo, giving you a
  green result about someone else's code.
- Each checkout builds into its own `.cargo-target` (automatic when you run
  from your checkout). Never point `CARGO_TARGET_DIR` at another checkout.
  First build per worktree is cold; that's the accepted price.
- Narrower runs: `-Workspace Extractor|Replay`, `-Mode Check|Build|Fast|
  Integration|Test`, `-Package` selectors. `Final` is the full battery.
- Always capture the cargo exit code itself before piping output anywhere.
- Tests must be deterministic and need no game install. Evidence-dependent
  tests go in the evidence tiers and never pass silently when evidence is
  missing.
- Test-quality audit: `python extractor/native/scripts/Audit-RustTests.py`.
