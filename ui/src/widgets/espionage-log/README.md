# Espionage operations log

## What it shows

Observed preparing, running and ended operations with type, endpoints, progress and captured completion outcomes.

## Products it reads

[`espionage-log`](../../../../crates/dashboard-intelligence/src/products/espionage_log.rs) supplies window discovery/detail and completion coverage. Shared log country facts supply dated names/flags.

## Settings

All/manual/group countries, campaign/follow/pin/range; content operation types, owner/target/either matching, ongoing order and shared log options.

## Notable behaviour

- Live puts ongoing operations first; Campaign uses branch-end history. Observed endings say By unless a native completion supplies an exact date. Endpoints use the read supplying their tags, independently of event date.
- Native handle/owner/target/type retain one operation through preparation and launch. Reopening after finish starts a new episode; mutable duration, slots, phases or tags do not.
- Preparation is recorded civilian preparation, not equipment readiness. Execution elapsed/duration uses native sentinels; held reads retain their dates, never recomputed progress.
- Completion hooks can supply census-unseen outcomes. Bonus, detection and outcome text remain independent; unavailable descriptions have a short status. Slot counts show assigned / available with names listed in slot order, without handles. Non-UTF-8 text is refused by name. Do not request future outcomes.
- Each row states operation status once and retains its progress meter. Missing row artwork uses a target fallback.
- Unavailable inline country flags keep a 25-by-16 placeholder slot; decoded flags use the log's 18-by-12 artwork slot, independent of stylesheet load order.
- Prepared day windows share one stored range per replay block and use shared coverage polling. The scalar parity hook is **Espionage operation values**, alongside the complete log model.
- Load wanted windows, retaining preceding identity/ending witnesses. Gaps, missing lexical names and unknown endpoint tags never borrow later data. All countries keeps unknown actors visible.
- After visible captures, parity builds retain every real manifest window intersecting the selected history through the normal reader. All windows and the exact boundary must complete, including when the newest window is empty. Complete models preserve ordered episode operands, recorded type names and exact status, outcome and progress inputs; dated labels, flags and stored description details remain ordinary viewport/tooltip demand. Visible text and screenshots cover mounted rows. Ordinary builds expose no preparation or extra history demand.

## Files

| File | Responsibility |
|---|---|
| [EspionageLogWidget.tsx](EspionageLogWidget.tsx), [useEspionageLog.ts](useEspionageLog.ts) | Virtual log, window demand and visible-row facts. |
| [espionageData.ts](espionageData.ts) | Validation, episodes, progress/outcomes and wording; adjacent tests. |
| [definition.ts](definition.ts), [index.ts](index.ts), [espionageLog.css](espionageLog.css) | Options, registration and layout. |

The manifest lists every recorded day while its type and availability facts grow
with preparation. The newest day is proved from its day and supplying blocks
when episode boundaries are witnessed locally. An inherited episode, or an
unmatched late completion for an earlier handle, leaves it pending. The log labels its exact newest day and prepared prefix, and keeps
polling both manifests and requested day windows as their gates advance.
