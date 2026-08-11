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

### Out of capture scope: POV-local presentation

**Owner decision, 2026-08-03.** These five domains are not scheduled in any production capture:

| # | Domain |
|---|---|
| d72 | `resolved_tooltip_instances_live` |
| d75 | `fired_event_popups_live` |
| d79 | `hud_surface_activation_live` |
| d88 | `active_command_abilities_live` |
| d89 | `national_focus_presentation_live` |

They capture POV-local presentation — what one client's screen had already resolved — which **Observer
only** puts outside what the recorder records. The replay is an observer-mode model: it derives each of
these panels from authoritative captured domains at render time rather than replaying one player's
screen, so capturing them adds nothing the viewer can use.

This is a scheduling decision, not a deletion. The producers stay in the tree and the domains keep their
production-complete status, so the decision is cheap to reverse and the domains remain individually
schedulable for evidence work.

**Not covered by this decision:** d85 `selected_designer_identity_live`, d95 `pov_spotted_units_live` and
d97 `peace_conference_live` stay scheduled. d95 in particular is in-scope class A.

A domain that was deliberately not scheduled must never be indistinguishable from one that failed to
produce. The set is owned by `compact_contract::DESCHEDULED_CAPTURE_DOMAIN_IDS`, mirrored into
`extractor/native/domain_registry.json` as `capture_scheduled: false`, and published positively into
every sealed manifest as `compact_stream.descheduled_domains` — absence from a ledger is never the only
record of the choice.

## Sessions, campaigns and files

- **One campaign is one file, for its whole life.** Across any number of game or recorder restarts. The engine campaign id and seed decide identity.
- Process identity — target PID plus creation time — is an attachment lease and session marker only. It is never part of the file key.
- **Reloads keep both branches.** Playback follows the surviving line.
- Sealing a session must not rewrite the whole campaign file.
- Two PCs recording the same campaign produce two files. Two simultaneous games on one machine record independently.
- Files are auto-named and renameable. Deletion is always explicit.

## Viewing

- Viewing refuses unless the exact recorded playback capability can be reconstructed from required content, DLC state, and settings. Extra unused local content is irrelevant.
- Mod graphics are resolved from the player's effective mod files at replay time.
- Before opening, Replay eagerly parses, authenticates, and folds the complete certified stream. Later hour access may reparse and rehash lazily, but cannot postpone certification or alter the admitted fold.
- Declared gap hours are unavailable in playback; the viewer never substitutes or presents a stale frame as current.
- A replay opens where the user stopped watching.
