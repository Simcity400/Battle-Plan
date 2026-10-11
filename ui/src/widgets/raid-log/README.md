# Raid log

## What it shows

Observed raid episodes, newest first, with target/location, native phase/outcome, artwork and preparation hours.

## Products it reads

[`raids`](../../../../crates/dashboard/src/products/raids.rs) shares one branch scan with [Raid tally](../raid-tally/README.md). Shared log facts name only visible countries.

## Settings

All countries by default, manual/groups, campaign/follow/pin/range; observed content raid types and shared log controls.

## Notable behaviour

- One row per episode re-enters at its latest phase/outcome/target/type change; Jump uses that read. Starts/changes say By, never invented exact launch times.
- Filters match the raider’s native holder at observation hour, following aliases and excluding unrelated reuse. No player/AI gate.
- Gaps leave current phase/preparation unknown while keeping explicit historical observations; outcomes persist after native removal. Between-read raids may be absent.
- Preparation is labeled integer hours, without an invented percentage. Native target precedence and ended-target freezing live in Rust.
- Content or eligible recorded types establish feature availability; country existence alone does not. Missing text/art/identities retain counted reasons and keys in details; titles use localized or unavailable wording. Repeated localized titles use numbered choices.

## Files

| File | Responsibility |
|---|---|
| [RaidLogWidget.tsx](RaidLogWidget.tsx) | Shared virtual log, type chips and dated facts. |
| [raidLogData.ts](raidLogData.ts) | Validation, row dates and wording; adjacent tests. |
| [definition.ts](definition.ts), [index.ts](index.ts) | Availability, options and registration. |
