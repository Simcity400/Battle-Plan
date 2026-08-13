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

**Owner decision, 2026-08-03, extended 2026-08-13.** These domains are not scheduled in any production
capture:

| # | Domain | Ruled on |
|---|---|---|
| d72 | `resolved_tooltip_instances_live` | 2026-08-03 |
| d75 | `fired_event_popups_live` | 2026-08-03 |
| d79 | `hud_surface_activation_live` | 2026-08-03 |
| d85 | `selected_designer_identity_live` | 2026-08-13 |
| d88 | `active_command_abilities_live` | 2026-08-03 |
| d89 | `national_focus_presentation_live` | 2026-08-03 |

The rulings are separate events and each domain carries its own date and reason on the wire; a reader of
a sealed manifest never has to guess which ruling covered the domain in front of them.

The 2026-08-03 five capture POV-local presentation — what one client's screen had already resolved — which **Observer
only** puts outside what the recorder records. The replay is an observer-mode model: it derives each of
these panels from authoritative captured domains at render time rather than replaying one player's
screen, so capturing them adds nothing the viewer can use.

This is a scheduling decision, not a deletion. The producers stay in the tree and the domains keep their
production-complete status, so the decision is cheap to reverse and the domains remain individually
schedulable for evidence work.

**d85 `selected_designer_identity_live`, owner decision 2026-08-13.** The 2026-08-03 decision explicitly
left it out; this reverses that. It is client UI selection state, the same class as the five above, and
lane HH measured the second half of the case: **as built d85 can never publish a clean word.** Its source
returns an error whenever no designer family has been observed, the collector can only encode that as
`SOURCE_UNAVAILABLE`, and the promotion gate on this page makes source-unavailable non-promotable for a
`production_complete` domain. So every passive capture was permanently non-promotable on d85 for what is
the ordinary state of the game — nobody has a designer open. Measured identically on t48, t49 and t50:
`extractor/native/runs/hh_zero_rows/VERDICTS.md`.

Because it is descheduled, the `0x1410b6280` mis-scope in its selection hook is not being fixed: nothing
downstream reads its verdict any more. It was **not** inert when the ruling landed — the production hook
registry a passive capture always builds installed d85's detours unconditionally, taking 1,229–1,929
callbacks per capture for a domain nobody asks for. That unit patched **eight** sites: seven in the
selected-designer module (five selection observers plus two destruction observers that retire the
`handler -> view` join before the engine frees the memory it names) and d78's designer-refresh
prerequisite.

**Closed 2026-08-13.** The seven are no longer installed and no longer pinned; the d78 prerequisite
stayed, and the group it travelled in is now `StandardHookReceiptGroup::D78` holding that one site. The
rule is derived, not hand-listed: `compact_contract::production_hook_site_installs` says a
producer detour is installed unless the owner descheduled the domain that owns it AND no domain the
shipped roster schedules holds that exact site as an install prerequisite. Two consequences a reader should not have to re-derive —
d78's site stays because **domain 78 was never descheduled**, so dropping it would need its own ruling
about d78, not this one; and the pinned roster shrank by seven, which moves `allowed_hook_count` and the
allowlist digest together, so a clean capture still reports every allowed hook active.

d85 was not unique in having production-installed hooks while descheduled, and the other case is what the
rule is shaped around. d75's producer edge is installed in production too, because it supplies d64's
dispatch prerequisite and the two share one install unit by design. That hold is recorded in
`compact_contract::DESCHEDULED_DOMAIN_HOOK_PREREQUISITES`, names the three `gameplay.d75.*` sites
exactly rather than the domain, and is conditional on d64 still being in the shipped roster — so a later
ruling on d64 releases the edge rather than silently preserving it, and a future d75 site added for some
unrelated reason does not inherit the exception.

The seven d85 addresses did not simply disappear: they moved to
`compact_contract::passive_ready::FORBIDDEN_PASSIVE_PRODUCTION_HOOK_SPECS_V1`, which the recorder host
baselines before injection and re-reads at READY and at teardown. "Not installed" therefore remains an
independent observation of the game's own memory rather than the collector's word — the thing that would
otherwise have been lost by deleting them from the roster the host walks.

Reviving d85 means giving it a presence read that can say "nothing is selected" (the owner slots at
`0x140B52390 +0x250/+0x258/+0x260/+0x268`, the way d97 uses its container count) and closing that
mis-scope first.

**Not covered by any of this:** d95 `pov_spotted_units_live` and d97 `peace_conference_live` stay
scheduled. d95 in particular is in-scope class A, and d97 already publishes a clean `SOURCE_EMPTY` for
its own idle state — exactly the vocabulary d85 lacks.

### Optional companions patch nothing, owner decision 2026-08-13

**A passive capture installs no optional fail-closed companion's producer detour.** The companions are
`compact_contract::OPTIONAL_FAIL_CLOSED_COMPANION_DOMAIN_IDS` — d57, d78, d84, d98, d99 — domains kept
out of the mandatory production profile and never scheduled by
`compact_contract::passive_production_roster()`. Three of them owned pinned detours a shipped capture
patched in anyway: d57's three (`pause_forced`, `pause_toggle`, `speed_setter`), d78's one
(`designer_refresh`) and d99's one (`land_combat_round_entry`). Five detours in a player's game, for
reads nobody takes.

This is a **second, separate removal authority**, not an extension of the deschedule. A deschedule says
the owner ruled a domain out of the product; this says a domain a passive capture never schedules should
not be patching the game. Both live in `compact_contract::production_hook_site_installs`, as two clauses
of one rule rather than a list of exceptions, so a domain later promoted out of the companion set gets
its detours back without anyone editing anything. The companion clause deliberately does **not** consult
`DESCHEDULED_DOMAIN_HOOK_PREREQUISITES`: a hold exists so a *scheduled* domain can keep a descheduled
one's site alive, and a companion is by construction not scheduled, so it can hold nothing.

Same treatment as d85's seven: the addresses move to
`compact_contract::passive_ready::FORBIDDEN_PASSIVE_PRODUCTION_HOOK_SPECS_V1` unchanged field for field,
so the recorder host still baselines them before injection and re-reads them at READY and at teardown,
and "not installed" stays an observation of the game's own memory. Their lanes keep every RVA, patch
length and sentinel, and an evidence capture still installs them.

`StandardHookReceiptGroup::D57`, `::D78` and `::D99` are gone with their sites — a group owning no site
installs nothing — and the pinned roster went 50 → 45, moving `allowed_hook_count` and the allowlist
digest together.

**All five are out.** d99's went last: while the ruling was being applied another lane had the
d21/d49/d50/d55/d99 combat family open, so `gameplay.d99.land_combat_round_entry` was deferred BY NAME in
`compact_contract::OPTIONAL_COMPANION_SITES_DEFERRED_TO_ACTIVE_LANES` rather than by silence, and moved
the moment that lane landed without having touched d99. That constant is now empty and is kept empty: a
deferral is a deviation from the law rather than a clause of it, and the shape — read only inside clause
2, held to being a real companion site that is really still pinned — is what the next lane needing one
should inherit instead of inventing a carve-out under time pressure.

Two consequences of d99 in particular. Its `uninstall` SEALS the producer rather than disarming it, so
the forbidden-site refusal calls its ABORT, which discards the queued combat rounds with an exact count —
the only honest thing to do with rows whose frame was never going to exist. And
`CUSTODY_OUTLIVES_UNINSTALL`, the collector roster of lanes whose custody outlives their detour, had d99
as its only member; it is now empty, kept rather than deleted for the same reason the forbidden roster
existed at length zero before d85 was descheduled into it.

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
