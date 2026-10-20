# Division template

## What it shows

A country's current division templates at the shown date, in the game's roster order: icon and name, how many divisions field each, battalions and support companies, base combat width and priority. Obsolete templates sit in a closed **Obsolete templates (N)** group under the current ones; unused templates are listed with none fielded.

## Products it reads

[`division-templates`](../../../../crates/dashboard-army/src/products/division_templates.rs), one country at the shown hour (a reached-hour request through [useReachedProduct](../../dashboard/reachedProduct.ts)).

## Settings

One country; Campaign, Follow (default) or Pin; Expeditionary forces count for (Owner/Host, as Army size); the shared table options and sort; drill placement.

## Notable behaviour

- Fielded counts follow Army size's attribution and leave out intelligence copies; an unread census shows "—", never 0.
- The country is the one identity the choice named at the date it was made, kept with the widget and followed through that identity's renames only, so a later holder of the tag never takes it; a tag that named no country then and several over the campaign asks which one.
- Only "Base combat width" is shown: the recorded sum before country and commander factors. The designer's final width belongs to the stats lane.
- Priority shows the designer's own heading for the stored stage, resolved through the content stack; a stage with no proven heading shows "—" with its reason on hover, never the number.
- A row opens the game-window renderer's division designer through [designerSnapshot.ts](designerSnapshot.ts). Until the renderer registers its snapshot, rows have nothing to open and their tooltip says *Game view not available yet*.
- Narrow widths hide Priority, Base combat width and Battalions, in that order, into the row tooltip.

## Files

| File | Responsibility |
|---|---|
| [DivisionTemplateWidget.tsx](DivisionTemplateWidget.tsx) | Table, obsolete group, header pill and designer drill. |
| [divisionTemplateData.ts](divisionTemplateData.ts) | Validation, rows, battalion summary and tooltips; adjacent tests. |
| [designerSnapshot.ts](designerSnapshot.ts) | The seam the renderer lane fills. |
