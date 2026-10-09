# Espionage operations log

## What it shows

Observed preparing, running and ended operations with type, endpoints, progress and captured completion outcomes.

## Products it reads

[`espionage-log`](../../../../crates/dashboard/src/products/espionage_log.rs) supplies window discovery/detail and completion coverage. Shared log country facts supply dated names/flags.

## Settings

All/manual/group countries, campaign/follow/pin/range; content operation types, owner/target/either matching, ongoing order and shared log options.

## Notable behaviour

- Live puts ongoing operations first; Campaign uses branch-end history. Observed endings say By unless a native completion supplies an exact date. Endpoints use the read supplying their tags, independently of event date.
- Native handle/owner/target/type retain one operation through preparation and launch. Reopening after finish starts a new episode; mutable duration, slots, phases or tags do not.
- Preparation is recorded civilian preparation, not equipment readiness. Execution elapsed/duration uses native sentinels; held reads retain their dates, never recomputed progress.
- Completion hooks can supply census-unseen outcomes. Bonus, detection, description/refusal, byte length and language remain independent; non-UTF-8 text is refused by name. Do not request future outcomes.
- Load wanted windows, retaining preceding identity/ending witnesses. Gaps, missing lexical names and unknown endpoint tags never borrow later data. All countries keeps unknown actors visible.

## Files

| File | Responsibility |
|---|---|
| [EspionageLogWidget.tsx](EspionageLogWidget.tsx), [useEspionageLog.ts](useEspionageLog.ts) | Virtual log, window demand and visible-row facts. |
| [espionageData.ts](espionageData.ts) | Validation, episodes, progress/outcomes and wording; adjacent tests. |
| [definition.ts](definition.ts), [index.ts](index.ts), [espionageLog.css](espionageLog.css) | Options, registration and layout. |
