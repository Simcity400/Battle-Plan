# Operatives roster

## What it shows

Recruited agency operatives with recorded name, skill/experience, assignment and capture status; countries can compare.

## Products it reads

[`operatives-roster`](../../../../crates/dashboard/src/products/operatives_roster.rs) supplies operative values, roster coverage and separate operation observations. Shared country facts supply dated roster/owner/target/capturer looks.

## Settings

Countries/groups and campaign/follow/pin; include departed in campaign mode, rank and shared table sort/presentation. Follow and name-ascending sort are defaults.

## Notable behaviour

- Recruitment pools are excluded. Portraits need a proven character link; missing portraits use a helmet icon. Unnamed differs from unknown name. Native progress ratios/sentinels govern skill and capture meters, without an assumed skill maximum.
- First presence/departure say By the observing read; causes are named only when recorded. Capture uses its native date. Jump offers first presence, recorded changes and departure.
- Roster continuity and native slot/id aliases both bound data; identity alone cannot bridge a missing roster or replacement. Failed reads are not empty rosters. Proven feature absence remains distinct from unknown capture.
- Departed rows retain last details and projected country hour, explicitly dated. Missing owner never becomes roster country. Operation definitions keep separate supplying dates.
- Request country looks only for mounted rows, retaining pending/refused flag reasons for each actual role.

## Files

| File | Responsibility |
|---|---|
| [OperativesRosterWidget.tsx](OperativesRosterWidget.tsx) | Shared table, meters and visible-row facts. |
| [operativesRosterData.ts](operativesRosterData.ts) | Validation, continuity, exact values and dated actions; adjacent tests. |
| [definition.ts](definition.ts), [index.ts](index.ts), [operativesRoster.css](operativesRoster.css) | Options, registration and layout. |
