# Agency comparison

## What it shows

Country intelligence-agency names, defence, acquired upgrades and active/maximum operatives in a comparison table.

## Products it reads

[`agency-comparison`](../../../../crates/dashboard-intelligence/src/products/agency_comparison.rs) supplies screen values, upgrade entries and identities; shared dated country facts supply names/flags.

## Settings

All created agencies by default, manual/groups/Top N; campaign/follow/pin, include-uncreated, Top countries by only for Top N selections, and shared table controls. Defence descending is the default sort.

## Notable behaviour

- Active/maximum are the recorded agency-screen values, not recruitment ceilings or inferred roster counts. Defence uses backend native formatting; sort exact raw values.
- Created names preserve literal text, including empty names. Uncreated agencies use content’s default name and muted rows.
- Upgrade entries retain native positions, duplicate definitions and signed levels; complete lists sum nonnegative levels. Missing/incomplete is Unknown, absent older capture is Not recorded. Definition pictures do not prove acquired-entry icons.
- Manual tags follow native holder aliases; gaps and replacements never inherit values. Pending/empty choices never request all countries. Names for alphabetic sorting can load all candidates, while flags load only mounted rows. Jump uses the supplying read.
- Prepared day windows preserve separate agency and operative dates. The parity row model is "Agency comparison values".

## Files

| File | Responsibility |
|---|---|
| [AgencyComparisonWidget.tsx](AgencyComparisonWidget.tsx) | Shared table and visible-row country facts. |
| [agencyData.ts](agencyData.ts) | Validation, exact ranking/rows and tooltip text; adjacent tests. |
| [definition.ts](definition.ts), [index.ts](index.ts), [agencyComparison.css](agencyComparison.css) | Options, registration and layout. |

Beyond prepared history the all-country day table reads exact agency screen
values and acquired upgrades from local frames, with the preceding supplying
operative read. Manual selections that need earlier holder aliases retain their
joint prefix and show Still being prepared for its pending hours. Capture
observation spans and preparation coverage remain separate.

The Country intel section uses the same table and scrolling tooltip components. Country and viewing-country choices are independent of agency creation. Reserved slot-zero sources and raw-zero propaganda holders retain their slot labels. The family filter exposes source pools/references, derived player estimates/ranges, network summaries/topology/subnetwork operands and propaganda history. One transient request shares the backend frame cache for the selected hour and any fuel supplying hour; source/content and viewer/target changes cancel obsolete replies. Tooltips show content-derived caps, decay and unlock reasons. Estimates derive from recorded game state and content; no recording-player panel observations are used. Fuel ranges retain an explicit As of date and hour in the table and tooltip; every operand uses that supplying hour. The game threshold's unknown marker is `?`; a missing reached input is named separately and counted.
