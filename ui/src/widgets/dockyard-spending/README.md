# Dockyard spending

## What it shows

Where one country's dockyard IC goes: new ships, refits and repairs, each by equipment type and design. Breakdown is the shared Sunburst; By month draws one stacked column per calendar month on the time axis.

## Products it reads

[`dockyard-spending`](../../../../crates/dashboard-base/src/products/dockyard_spending.rs) answers both views through prepared queries, through the hours the production and dockyard repair families have both prepared.

- New ships and Refits are Production spending's exact applied input of naval designs, split by purpose: one design can appear under both. Refit input belongs to the target design. The By month view reads the same input from production's shared spent series: a month's amount is its end's cumulative input less the hour before its start, inside one uninterrupted observation.
- Repairs are the dockyards the country uses for repairs times the output one of its dockyards applies to naval production at that hour (the recorded base rate with its power fraction and dockyard output modifiers), over the hours they were used ([dockyard_repairs.rs](../../../../crates/dashboard-base/src/products/dockyard_repairs.rs)). This is dockyard output used for repairs, which the game does not record as spending. Repair amounts stay exact fractions (daily output times hours, over the hours of a day), so designs and their total add exactly; only the written IC rounds. Hovering Repairs says so.
- Repairs are split by repaired design at each read. A naval base repairs the first eligible ships in its queue, one dockyard each, up to the dockyards assigned to it; at each recorded hour every ship that selection holds counts one of its holder's dockyards toward the ship's current design, by equipment type and design. Allied ships repaired at a country's bases count toward that country's spending. Dockyards the selection does not place (an assignment with fewer eligible ships, a ship or design that cannot be read, a selection whose inputs are missing) stay in Unattributed, never spread over the queue. This split is the selection as read at each hour, not a record of which ships the game repaired in the hour before it.

## Settings

View (Breakdown, By month; also switched in the header and listed as its own catalogue card), the three ring levels, IC decimals and the shared sunburst options. Campaign/follow/pin/range time; one country.

## Notable behaviour

- A design whose type content cannot resolve could be naval. It is left out, and the purpose it belongs to is marked incompletely attributed (hatched); By month hatches that month's new and refit bands.
- A line no census listed has no recorded purpose: its input counts as new. Its evidence is the summed magnitude of that input, so corrections between lines never cancel it. Where such input joined the dates shown (or a month), New ships and Refits keep a present Purpose not recorded part and are marked incompletely attributed, however small the net amount; the whole stays known.
- Equipment spending the source cannot read is not zero: New ships and Refits each hold an unknown part, so known repairs never read as the whole. Without production reads, known repairs still show.
- A month whose designs content cannot resolve could be naval has no known total; its new and refit bands are hatched, and an uncertain band too small to draw keeps a hatched marker.
- A whole campaign's repairs start at the first hour the country's repair output is known; when that is after the recording's start, hovering Repairs names the date ("since …"). Hours before it, and months before it, are not known rather than zero.
- The split of repairs by design needs the holder's bases' assigned dockyards to sum to its dockyards in use. Where they do not (or the bases cannot be read), the repairs total stays known, the designs whose shares are not known are left unmeasured, Unattributed holds the rest and the split is marked incomplete. Mismatches are counted in the reply's warnings.
- A month counts only where the equipment source was read and untainted: before its first read, after a recording gap or missed publication taints it, or after its observation restarts, new ships and refits are not known.
- Spending only on designs whose type content cannot resolve keeps a present Type not resolved part under its purpose, so the chart never reads as nothing spent.
- Missing is unknown, never zero: a recording gap, an unread repair rate, dockyards in use without a known output, or a country change leaves that amount unknown with its reason, and a month with an unknown part shows a hatched cap.
- Before preparation reaches the dates shown, the reply answers its prepared prefix as Production spending's does; By month bands the pending months.

## Files

| File | Responsibility |
|---|---|
| [DockyardSpendingWidget.tsx](DockyardSpendingWidget.tsx) | Product, view switch, Sunburst and month columns. |
| [dockyardData.ts](dockyardData.ts) | Validation, sunburst items, the repairs part and month columns; adjacent tests. |
| [definition.ts](definition.ts), [index.ts](index.ts) | Views, levels, shared options and registration. |
