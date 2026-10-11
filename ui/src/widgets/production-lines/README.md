# Production lines

## What it shows

One country’s active production table at a moment, or line histories across the campaign, with efficiency/output factor drills.

## Products it reads

`production-lines`, `production-line-detail` and `production-line-breakdown` are owned by [production_lines.rs](../../../../crates/dashboard/src/products/production_lines.rs). Detail loads in windows; a selected factor has its own branch history.

## Settings

Campaign/follow/pin/range; order/columns in follow and pin, ended lines in campaign and range, selected line, Show and drill placement. The breakdown body owns the factor selector. Whole campaign is the default.

## Notable behaviour

- A line keeps its native identity through equipment switches; deleted lines retain final output. Units made are captured deliveries, not the game’s progress toward its next unit. Unseen between-read lines lack factory census data.
- Weekly output and rounded efficiency use the backend’s native formulas. Factor attribution uses recorded operands and explicit residuals; missing sources are not reconstructed from present content.
- Values, output, organisations and variant definitions have independent coverage/read dates. Gaps, incomplete tables and replacement countries stop carries; missing final output can make totals unknown.
- Missing equipment and organisation pictures use subject icons. Line choices use equipment names and line numbers.
- Names/art follow the read’s variant and country context. Native icon/organisation fallback logic belongs to Rust, not a frontend catalogue.
- Hover, moment parts and factor history share one part model. Signed corrections remain exact rows; only positive bands draw. A picked day holds, Follow resumes playback and Pin preserves the line/factor and picked date. Saved line keys and original choice dates bind campaign, branch and native producer. Breakdown histories follow its renames, exclude reused tags and date the header to local Pick/Follow. Missing saved origins keep a neutral header and unavailable body.

## Files

| File | Responsibility |
|---|---|
| [ProductionLinesWidget.tsx](ProductionLinesWidget.tsx), [useProductionLines.ts](useProductionLines.ts) | Table/history, window requests and choices. |
| [productionData.ts](productionData.ts), [ProductionDrill.tsx](ProductionDrill.tsx) | Validation, exact rows/factors and drills; adjacent tests. |
| [definition.ts](definition.ts), [index.ts](index.ts), [productionLines.css](productionLines.css) | Options, registration and layout. |
