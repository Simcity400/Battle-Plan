# Product decisions

**This file is the authority on what the product is supposed to be.** Where any other document
in this repository disagrees with it, this file wins and that document is wrong.

It records current intent only. It is not a history, not a status report, and not a plan. Nothing
here says what was tried before or why it changed — that is what git is for.

Read this before designing anything. Several specifications in this repo describe the system as it
is built today, and parts of that system are being deliberately replaced. Those documents remain
accurate about the current implementation and are not a description of the target.

---

## What the product is

A recorder that watches a the game session from outside and produces a replay file, plus
a viewer that plays it back. It must work for any player, on any mod set, with any DLC combination.

The user experience is the contract:

> You boot it up and then start playing the game. It automatically understands which game and which
> save is attached to it. It saves the progress and spits out the replay file at the end. It doesn't
> matter if you have a reload, or if you close the game down and reopen it with an old save — it will
> try to stitch everything together so it looks normal. Settings should be bare bones: plug and play.

## Non-negotiable

| # | Decision |
|---|---|
| **Pause is forbidden** | The shipped recorder never writes game state, and specifically never writes the engine pause byte. The former pause-owning sweep barrier is deleted and is not a compatibility mode. Only an unshipped, physically separate evidence workflow may pause during setup or a deliberately non-production experiment. |
| **Observer only** | The recorder reads `CGameState`. It never drives the UI, never depends on what the player has selected or open, and captures no host-local presentation state. |
| **Injected observation is allowed** | The collector may be injected into HOI4. Injection is a transport and observation technique, not permission to control the client. The injected production collector has zero semantic engine-state writes and never pauses, changes speed, drives UI, launches, stops, focuses, minimizes, or sends input to the game. |
| **Minimal proven hooks are allowed** | A value that is provably unsampleable may use an evidence-backed hook through the single audited instrumentation primitive. Bootstrap hooks required to discover the rendering device remain allowed. Every hook is registered and justified; broad, speculative, UI-driving, or scheduler-added hooks are forbidden. Teardown must prove every pinned hook site was restored byte-for-byte before a capture is promotable. |
| **Speed five remains player-owned** | Capture must keep up while the player runs at speed five. The recorder observes and accounts for accepted or typed-gap outcomes; it never slows, pauses, or changes the client's speed to manufacture a clean result. A recording with typed gaps is retained, but a graded production validation receipt requires zero loss at speed five. |
| **Test the real boundary** | Low-level collector/extractor tests reproduce real attach, capture, sink, teardown and speed-five pressure without claiming to drive the entire game/UI workflow. Evidence setup may control the game before the graded window; from attach through authenticated teardown, the graded recorder performs zero client commands. |
| **A package gate is not capture proof** | Source, feature, import and ZIP-byte checks prove only what capabilities were shipped. They never certify that production domains worked. The final portable-package receipt additionally requires a real passive speed-five capture from the exact current artifacts, zero skipped hours or domain gaps, and a data record with no raw or effective failure flags for every current `existing`, `production_complete`, or `production_complete_base` domain. Source-unavailable, hard-disabled, partial, bridge-failed, or masked status is non-promotable. |
| **Always produce a recording** | Within an admitted campaign with writable storage, a recording is always produced. Coverage failures become typed, explicit gaps — never silent omissions and never a refused capture. |
| **Reach over depth** | Working for any mod set and DLC combination outranks capturing more per hour. Admission is structural, not an exact registry match. |
| **Unknown build refuses** | An unsupported game build records nothing and says so. It never guesses at offsets. |

## Fidelity and coverage

- **The replay must look and feel like the game.** Playback is not a summary of what happened; it is the game's own scene, redrawn. The viewer renders from the player's own content using the game's own rules, so what the capture owes it is every input that drawing needs — position, progress, orientation, ownership, the draw state d77 carries — not merely the facts an analyst would want. We never record pixels: fidelity is judged by whether the rendered replay matches the game, and the capture is sized by what the renderer consumes.
- **Hourly baseline cadence, finer where a domain needs it.** One sample per game hour is the default and suits state the engine itself only changes hourly. A domain whose appearance or behaviour is visibly wrong at that resolution declares a finer cadence and justifies it. A machine that cannot meet a domain's declared cadence produces typed gaps, not a failure.
- **Gaps are first class.** "Not recording" and "recording failed" are distinct. A gap carries its hour or range, its reason, the affected domains, and a witness.
- **Torn sweeps are gaps.** Sweeps are tick-bracketed; a sweep that spans a tick boundary is recorded as a typed gap rather than emitted as if coherent.
- **Instrumentation is minimised, not categorically banned.** Executable instrumentation is permitted only where a value is provably unsampleable, through one audited primitive, and every point is registered and justified. Readable state stays on the read path. Hooks do not grant authority to mutate simulation state or control the client.
- **The file is as small as it can be without costing fidelity, and the order matters.** Fidelity is never traded for bytes: no dropped field, no collapsed distinction between absent, zero and unknown, no quantization of a value whose exact magnitude explains what happened **or decides what is drawn**. Subject to that, size is minimised aggressively — silence when nothing changed, compression that sees across hours, symbols carried as ids. **3–8 GB per campaign is a ceiling, not a budget to spend.** A design that comes in far under it is better, and "we are within the ceiling" is not a reason to stop reducing.

## Sessions, campaigns and files

- **One campaign is one file, for its whole life.** Across any number of game or recorder restarts. The engine campaign id and seed decide identity.
- Process identity — target PID plus creation time — is an attachment lease and session marker only. It is never part of the file key.
- **Reloads keep both branches.** Playback follows the surviving line.
- Sealing a session must not rewrite the whole campaign file.
- Two PCs recording the same campaign produce two files. Two simultaneous games on one machine record independently.
- Files are auto-named and renameable. Deletion is always explicit.

## Viewing

- Viewing refuses unless the exact recorded playback capability can be reconstructed: required content, required DLC state, settings. Extra unused local content is irrelevant.
- Mod graphics are resolved from the player's own mod files at replay time.
- The raw stream is canonical; folding is lazy and progressive.
- Gaps display the last known state, marked stale.
- A replay opens where you stopped watching.

## Scope of the rebuild

Acquisition, the wire format and the container are being rebuilt rather than adapted. The
`.battleplan` files that exist today will stop opening, and that is accepted.

**Reused as proven work, not re-derived:** the SEH read primitive and its page and region caches;
the content VFS and portable content manifest; the campaign stitching, append and rollover logic;
`rtti_rederive.py`. Where these touch the wire format or container, that edge is ported, not kept.

**The current collector is retained, unshipped, as a comparison oracle.** It is the only working
recorder and the reference the new acquisition path is measured against. It is not dead code.

## What is now known

**Instrumentation volume causes the injection crashes.** Measured 2026-07-26 on build a729, six
trials per arm, each trial bound to its own crash record by time window and classified by
exception address:

| Arm | Trials | Crashed | Survived |
|---|---:|---:|---:|
| Full — 79 scheduled domains | 6 | **6** | 0 |
| Minimal — 1 scheduled domain | 6 | 0 | **6** |

The authoritative ledger is `extractor/native/runs/crashrate/trials.jsonl`. Read the outcomes from
that file, not from this table: `trials_20260726_invalid_minimal_arm.jsonl` sits beside it and is
an earlier, invalidated run whose numbers differ.

Reducing instrumentation is therefore a real fix and not a guess. Three qualifications bound that
claim, and all three matter:

- **The comparison was 79 domains against 1, not against zero.** The floor is two hooks, not zero
  — `d3d11!D3D11CreateDevice` and `d3dcompiler_47!D3DCompile` must stay installed or the device
  cannot be discovered. Whether crashes reach zero or merely become rare is still unmeasured.
- **Three distinct crash modes exist, and they are not evenly split.** Of the six
  full-instrumentation crashes: **four** were a call through a null pointer (the
  half-written-detour signature), **one** faulted at `hoi4.exe+0xC66F46` in the game's own code
  with the collector confirmed loaded, and **one** faulted at `0x3F800000`. That last address is
  the IEEE-754 bit pattern for `1.0f`, so it is a call through a pointer read out of float data —
  a distinct mode, not a variant of the other two. All three occurred zero times in the minimal
  arm.
- **The per-mode counts do not carry the same weight as the arm totals.** Six-versus-zero across
  arms is a strong result; a 4/1/1 split within one arm of six is not a distribution anyone should
  reason from. Treat the mode list as a catalogue of what has been seen, not as frequencies.

The `+0xC66F46` offset is stable across runs under ASLR — absolute `0x7FF6BC5C6F46` in the valid
run and `0x7FF79B7D6F46` in the invalidated one both reduce to that same image offset — so the
site itself is reproducible even though its frequency is not.

What remains unestablished is the mechanism. Nothing here identifies *how* volume produces these
faults, and the two non-null modes fault in the game's own code. Whether any of them involves our
detours is a claim the trial ledger cannot support either way; settling it needs the crash dumps
themselves, captured with full memory.

Never measure this by counting crash-folder entries: that scores every mode identically, and the
folder also collects ordinary play-session crashes.

## What is not yet known

- Whether coherent reads are achievable at the target cadence without the pause barrier.
- The performance cost of sampling on constrained hardware.

Design decisions that depend on these must wait for measurement rather than assume the hypothesis.
