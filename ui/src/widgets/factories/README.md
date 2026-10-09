# Factories

## What it shows

Country factory counts as a race, lines or one-country type mix, with the game’s additions/deductions and a full type drill.

## Products it reads

[`country-factories`](../../../../crates/dashboard/src/products/country_factories.rs) supplies recorded groups and ranked histories. [factoryMeasure.ts](../../dashboard/factoryMeasure.ts) owns count choices shared with Faction strength.

## Settings

Top 10 by default, manual countries/groups; Free after withheld, Built or Top bar counts, content types, split bars and shared race/playback/axis/drill options.

## Notable behaviour

- Counts use native truncation and recorded factory groups. Content production flags define types; a mod’s factories share the engine group their flags select. Rocket sites are not recorded.
- Built includes damage and hatches it; Free and Top bar already deduct damage. Signed negative counts and reversed deductions keep their signs. Missing countries are omitted, never zero.
- The product ranks exact filtered counts; avoid a second ranking formula. Keep value runs separate from supplying-read and native identity runs.
- Drills bind the clicked slot/id at its original date, retain that holder through tag changes and exclude later reuse. Missing identity/read provenance stays unknown. Picking holds; Follow resumes playback; Pin retains actor and branch-relative day.
- Numeric changes invalidate pending picks; content-only refresh retains numeric ownership. Country looks use the recorded identity segment and dated visible/hover hours.

## Files

| File | Responsibility |
|---|---|
| [FactoriesWidget.tsx](FactoriesWidget.tsx) | Shared race and drill composition. |
| [factoriesData.ts](factoriesData.ts), [factoryDrill.ts](factoryDrill.ts) | Validation, counts, signed groups and actor-bound history; adjacent tests. |
| [definition.ts](definition.ts), [index.ts](index.ts) | Settings, content choices and registration. |
