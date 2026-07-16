# Project Vision

**Purpose:** State the product goal and durable project-wide design principles; implementation
status lives in `extractor/documents/MASTER_EXTRACTION_CONTRACT.md`.

This project is trying to build a full-game replay system for the game
multiplayer on Windows: a recorder that observes a live game without changing it, writes a
durable `.battleplan` file, and lets a separate replay interface reconstruct the
war from any perspective after the session is over.

The target is not a match summary, an after-action report, or a handful of
charts. The target is a real replay: the player should be able to scrub through
the game and understand what happened on the map, in the armies, in the air and
navy, in the economy, in politics, and in diplomacy with enough fidelity that
the replay feels like watching the original game unfold.

## What We Are Building

The application has two major halves.

The supported product platform is Windows only.

The first half is the recording layer. It watches `hoi4.exe` while the game is
running and records typed facts about the game at an hourly-or-better cadence.
Those facts become compact records and snapshots in a `.battleplan` file. The
recording layer must keep up with speed five, must fail closed when a layout is
not proven, and must never depend on save files or Python in the production hot
path.

The second half is the replay interface. It consumes the `.battleplan` file and
renders the war back to the user: map state, front lines, unit movement,
combat, air and naval activity, country state, decisions, focuses, production,
logistics, trade, diplomacy, and the visible context needed to explain why the
game evolved the way it did.

The active production capture path is:

```text
hoi4.exe -> one injected DLL collector/coordinator -> typed compact records -> Rust recorder -> .battleplan
```

Python, save files, and external memory-reader scripts are development tools.
They can validate, probe, and help reverse-engineer the game, but they are not
part of the production recording loop.

## Production Capture Scope

STATUS: RATIFIED (owner, 2026-07-16 — "It makes literally no sense to include the host local UI state... completely useless in the replay data"); host-local presentation state is normatively OUT of production capture scope.

The production replay records the authoritative multiplayer world, not a recording of the host's
desktop or local client session. The default inclusion rule is:

- every datum serialized by the save game is in scope for semantic coverage;
- authoritative host-side simulation state and events that affect or explain the shared world are
  in scope even when they are transient or omitted from the save; and
- stable identities, ownership, relationships, timing, provenance, and completeness witnesses
  needed to interpret that state are in scope.

In-scope does not mean copying every value verbatim every hour. Stable facts may be carried by
static catalogs or one-time records, changing facts may be delta encoded, and transient changes may
be represented as events. The replay must preserve the authoritative meaning while remaining
compact enough for a full multiplayer campaign.

Host-local presentation state is permanently outside the production recording scope, even when it
is technically reachable in the host process. This includes camera and viewport state, selections,
hover targets, open screens or dialogs, map-mode and layer toggles, outliner state, local tooltips,
drag state, and other mouse/keyboard/UI preferences. A multiplayer host does not possess a useful
authoritative record of every remote player's local UI, and the host's own UI is not shared game
state. The replay interface owns its camera and selections and populates HOI4-like panels, map modes,
tooltips, and alerts from captured world facts as if observing the real game.

## Replay Fidelity Goal

A good replay should answer the questions players naturally ask after a game:

- Where were the divisions, fleets, air wings, supply hubs, and front lines?
- What battles happened, who fought, how did strength and organization change,
  and what losses were taken?
- What was each country building, researching, trading, buying, producing, and
  stockpiling?
- Which focuses, decisions, laws, advisors, leaders, spirits, projects, and
  events shaped the game?
- How did occupation, resistance, compliance, construction, damage, weather,
  supply, attrition, naval control, air superiority, and intelligence affect the
  map?
- What changed hour by hour, and what can be reconstructed exactly versus what
  must be badged as inferred, unavailable, rejected, or blocked?

The ambition is maximal authoritative replay coverage. Save-serialized state and authoritative
host simulation state belong in the recording backlog until captured or conclusively blocked.
Merely existing in the host process is insufficient: local presentation state is explicitly
excluded and must not be treated as an extraction or reverse-engineering gap.

## Design Principles

The recorder is observational. It must not mutate the game, automate gameplay,
or rely on multiplayer-host advantages beyond reading the owner's own local game
process.

The recorder is production-shaped. Performance judgments are based on the
DLL-to-compact-stream-to-Rust-recorder path, not on debug JSON, save parsing, or
Python memory walking.

The recorder is honest. Unknown fields are not guessed. If an offset, root, or
layout cannot be validated for the current build, the field is omitted or badged
instead of fabricated.

The replay file is durable. A `.battleplan` should contain enough schema,
manifest, provenance, and compact data to be decoded later without needing the
live game, a save file, or a pile of temporary proof artifacts.

The UI should feel like the game, not a generic dashboard. The replay interface
should use real game assets and map behavior where practical, preserve the
meaning of in-game symbols and map modes, and make time travel through the war
fast and understandable.

## What Counts As Done

The recording layer is not complete because a slice produced some data. It is
complete only when every replay-relevant, extractable domain is either captured
through the production path or has a precise documented blocker or conclusive
rejection.

For extraction progress, the source of truth is
`extractor/documents/MASTER_EXTRACTION_CONTRACT.md`.

For replay file shape, the source of truth is
`replay_interface/documents/HOI4REPLAY_CONTAINER_SPEC.md` and
`replay_interface/documents/REPLAY_SCHEMA.md`.

For map and interface parity, the source of truth is
`replay_interface/documents/MAP_REPLICA_PLAN.md` and
`replay_interface/documents/UI_SOURCE_OF_TRUTH_REPORT.md`.

## Non-Goals

This is not a cheat tool, bot, or gameplay automation system. It is a private,
read-only replay and stats tool for the owner's own multiplayer group.

This is not a save-game viewer. Saves can be useful as development or validation
fixtures, but a finished replay recording must come from live capture.

This is not a collection of disconnected extractors. The production model is one
collector/coordinator, one ordered compact stream, and one replay file.
