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
| **Pause is forbidden** | The shipped recorder never writes game state, and specifically never writes the engine pause byte. Today's collector does this before every sweep; that mechanism is being removed, not preserved. Only an unshipped, physically separate evidence build may pause. |
| **Observer only** | The recorder reads `CGameState`. It never drives the UI, never depends on what the player has selected or open, and captures no host-local presentation state. |
| **Always produce a recording** | Within an admitted campaign with writable storage, a recording is always produced. Coverage failures become typed, explicit gaps — never silent omissions and never a refused capture. |
| **Reach over depth** | Working for any mod set and DLC combination outranks capturing more per hour. Admission is structural, not an exact registry match. |
| **Unknown build refuses** | An unsupported game build records nothing and says so. It never guesses at offsets. |

## Fidelity and coverage

- **Comprehension-grade fidelity.** Enough to understand what happened and why, not pixel-exact reproduction.
- **Hourly target cadence.** A machine that cannot meet it produces typed gaps, not a failure.
- **Gaps are first class.** "Not recording" and "recording failed" are distinct. A gap carries its hour or range, its reason, the affected domains, and a witness.
- **Torn sweeps are gaps.** Sweeps are tick-bracketed; a sweep that spans a tick boundary is recorded as a typed gap rather than emitted as if coherent.
- **Instrumentation is minimised.** Executable instrumentation is permitted only where a value is provably unsampleable, through one audited primitive, and every point is registered and justified.

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

## What is not yet known

- **Whether instrumentation volume causes the injection crashes.** Injecting the collector kills
  the game in roughly two attempts out of three. The hypothesis is that ~249 instrumented code
  points cause it. That is untested. Removing them may fix nothing.
- Whether coherent reads are achievable at the target cadence without the pause barrier.
- The performance cost of sampling on constrained hardware.

Design decisions that depend on these must wait for measurement rather than assume the hypothesis.
