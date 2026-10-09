# Agency comparison

## What it shows

Country intelligence-agency names, defence, acquired upgrades and active/maximum operatives in a comparison table.

## Products it reads

[`agency-comparison`](../../../../crates/dashboard/src/products/agency_comparison.rs) supplies screen values, upgrade entries and identities; shared dated country facts supply names/flags.

## Settings

All created agencies by default, manual/groups/Top N; campaign/follow/pin, include-uncreated, separate ranking measure and shared table controls. Defence descending is the default sort.

## Notable behaviour

- Active/maximum are the recorded agency-screen values, not recruitment ceilings or inferred roster counts. Defence uses backend native formatting; sort exact raw values.
- Created names preserve literal text, including empty names. Uncreated agencies use content’s default name and muted rows.
- Upgrade entries retain native positions, duplicate definitions and signed levels; complete lists sum nonnegative levels. Missing/incomplete is Unknown, absent older capture is Not recorded. Definition pictures do not prove acquired-entry icons.
- Manual tags follow native holder aliases; gaps and replacements never inherit values. Pending/empty choices never request all countries. Names for alphabetic sorting can load all candidates, while flags load only mounted rows. Jump uses the supplying read.

## Files

| File | Responsibility |
|---|---|
| [AgencyComparisonWidget.tsx](AgencyComparisonWidget.tsx) | Shared table and visible-row country facts. |
| [agencyData.ts](agencyData.ts) | Validation, exact ranking/rows and tooltip text; adjacent tests. |
| [definition.ts](definition.ts), [index.ts](index.ts), [agencyComparison.css](agencyComparison.css) | Options, registration and layout. |
