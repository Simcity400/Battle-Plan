# Occupation trend

**What it shows:** Mean resistance/compliance across each occupier's readable held states, as country lines, a race or one country's measure lines.

**Product(s) it reads:** [`occupation-trend`](../../../../crates/dashboard-occupation/src/products/occupation_trend.rs) reuses the [`occupied-states`](../../../../crates/dashboard-occupation/src/products/occupied_states.rs) preparation observer; optional collaboration uses its recorded country context.

**Settings:** All occupiers, Lines and Resistance by default; manual countries/smart groups/top N, Compliance/Both and ranking measure, optional collaboration, label decimals and shared race/line controls; Campaign, Follow, Pin and Range.

**Notable behaviour:**

- The mean uses unweighted state scalars and native signed fixed-point arithmetic at their supplying read. It is not the game's unrecorded cached occupied-country-group mean, nor an average of rounded UI values.
- Membership uses active occupation/garrison rules, not owner/controller inequality. Covered/held counts remain explicit; incomplete inventory cannot establish the total. No readable held states means a gap, not zero.
- A reached hour beyond preparation reuses the occupied-state window?s proved look-back, preserving the supplying resistance read?s controller and native join. Unprepared lines sample each block at its first fresh resistance read and draw those spans approximate. Unsampled hours stay pending. Branch-wide identity/segment metadata is labelled **Still being prepared**; exact reached holder evidence replaces sampled aliases.
- Country collaboration is separate from state compliance, off by default and requested only when enabled. Each context value keeps its own read date and unknown reason.
- Manual roots follow native holders/aliases; smart groups retain counted dates and unknown-roster gaps. One-country choices fix a native holder; absence does not silently select another. Looks never use a later holder to name old values.
- Display decimals and smooth geometry do not alter exact tooltip means. Resistance/compliance remain independent measures; Both uses different line styles. The shared race view owns keyboard, zoom and Jump.

**Files:** [OccupationTrendWidget.tsx](OccupationTrendWidget.tsx) configures [CountryRaceView](../../charts/race/CountryRaceView.tsx); [occupationTrendData.ts](occupationTrendData.ts) validates and projects means/context, with adjacent tests. `definition.ts` and `index.ts` declare settings and registration; membership details are in [Occupied states](../occupied-states/README.md).

Shared framework conventions: [widgets](../README.md).

The occupied-state preparation observer feeds both fixed5 mean channels, supplying-read boundaries, exact context events and rank-position runs. The widget queries reached exact hours and width summaries; Both draws separate summaries by its existing line IDs. Exact windows use the product reply builder to retain supplying-read and optional collaboration boundaries. A one-country view bounds summary membership to its chosen native holder, and line labels read exact endpoint facts. No chart envelope supplies tooltip state counts or collaboration. The registered product remains the parity reference. Parity builds expose "Occupation trend values".
