# Resource balance

One country's strategic resources: where each comes from and where it goes, per day (mock-up economy-1).

- **Balance bars (default view):** one row per resource with its game icon and name, in the game's order.
  - The supply bar shows produced, then received from subjects, then imported, in lighter shades of one colour.
  - The demand bar below shows needed (by special projects and production lines, met or not), then sent to the overlord, then the export reserve, in lighter shades of a second colour.
  - The game's surplus (green) or deficit (red) is at the right.
  - The legend lists only the parts that occur. Bars glide over the app's motion time within one unbroken stretch of recording, and jump across a gap, a missing table or a change of country, as a line breaks there; numbers are exact at once.
- **Over time (setting):** supply solid and demand dashed for the chosen resources.
  - Each chosen resource takes its palette colour by the game's order when that colour is free, else the first free one.
  - Resource chips above the chart choose the recorded resource tokens (a token declared twice draws a line for each declaration); the choice is saved like any setting and also appears in the settings panel. Lines past the palette repeat the colours with dash patterns of their own, one set for every further eight lines, so no two lines look alike however many there are.
  - A chip changes only its own resource. Saved resources this campaign lacks (another campaign's modded resource) are kept, as in the settings panel; the last resource this campaign shows stays chosen.
  - Until a resource is chosen, the resource with the largest supply or demand in the branch is shown.
  - With exactly one resource, the gap between the lines is shaded green (surplus) or red (shortfall), which can be switched off. The shading is bounded by the very curves the lines are drawn with and changes colour where they cross.
  - A chip that cannot act says why on hover: a resource whose content name cannot be stored, 16 tokens already chosen, or the last resource shown. With nothing chosen yet, choosing a resource never requires storing the automatic one.
- **Hover** (or keyboard focus on a row) shows the game-style breakdown:
  - produced, from subjects, imported with each import deal (flag, partner, the civilian factories paid and convoys);
  - to the overlord, exported with each export deal, what the deals do not account for, and the reserve set aside but not bought;
  - available, needed by special projects, needed by production lines, then the surplus or deficit.
  - Each direction lists its largest deals, as many as fit the window (at most eight), and sums the rest in one "N more deals" row. Where even one deal each way does not fit, the tooltip shows supply, demand and the net instead, so the surplus or deficit always shows at any window height (the chart kit's `fitTooltip`).
  - The deals are captured apart from the pools, and the product records the hours their deals are known. Where they are not (an hour whose deal table was not recorded), the pools still show and the breakdown says the trade partners were not recorded at that hour, with no "not from a listed deal" part.
  - Partners are named and flagged as the recording showed them at the hour the tooltip shows, never a later identity. Until those arrive, or where the recording has none, a partner shows by its tag and without a flag.
  - Over time lists this for up to two resources when it fits the window; otherwise each resource's supply, demand and net, or, in a short window, one row per resource with its net, so every net stays visible (a window too short even for that lists what fits and counts the rest).
- **Value axis** labels are the chart kit's: exact, with the precision their ticks need, so neighbouring ticks never read the same (0.25 beside 0.50; 10,000,000.00002 beside 10,000,000.00004); thousands are shortened to k only when that keeps them apart.
- **A date range** (Time settings) shows only its hours over time: nothing before or after it is drawn, shaded, fitted into the value axis or shown on hover, even where the axis is dragged past it.
- **Screen readers:** each row's label gives supply, demand and the net; its description is the full breakdown, trade partners included, even where the window trims the tooltip.
- **Keyboard over time:** with the chart focused, the arrow keys step through the hours where a chosen resource's pools or deals change, and the start of a date range or zoom that begins between two of them (Home and End go to either end, Escape leaves); a key takes over from the pointer until it moves again, showing the tooltip at that hour and reading it out in full: each resource's breakdown for up to two resources, else each one's supply, demand and net. The context-menu key jumps there.
- A widget scrolled out of view (or on a hidden tab) drops its tooltip and forgets the pointer.
- **Right-click** (or the context-menu key on a row) jumps to that moment: the hour under the pointer, or the hour the bars show.
- **Settings:** view; resources over time; gap shading; row order (game order, biggest shortfall first, largest first); bar length (same scale all campaign, or fit the moment shown); hiding resources with no supply and no demand; the time-chart playback mode, window and axes.

## Data

**Product `resource-balance`** (`crates/dashboard/src/products/resource_balance.rs`) supplies, for one country over the branch:

- runs of the recorded `CountryResources` pools per resource (fixed point, five decimals);
- runs of each `ResourceExchanges` trade deal naming the country, and the hours the deals were recorded;
- each recorded resource's name and icon from content.

**The resources** are the recorded `ResourceDefinitions`, in the game's own order, so a mod's resources appear by themselves. They are told apart by their native index: the game keeps a token declared twice as two resources, so both show, the second numbered ("Steel (2)"), and a chip for the token chooses both. A resource is named as the game's resource texts name it (`GetLocalizedResourceName` 140BBC360): `PRODUCTION_MATERIALS_` and the token in upper case, through the plain localiser, so brackets show as written. A key the content lacks shows as the key, as the game shows it; a name the localiser refuses is shown as the token, with a warning. Its icon is its `icon_frame` from `common/resources` in the sprite that the trade view's `resources_info_entry` gives its `resources_icon` (vanilla declares the row in `interface/countrytradeview.gui`; a mod may declare it in any GUI file). The trade view's control lookup is not yet traced, so both button and icon declarations remain supported. Names and icons come from the shared `products/resources.rs`, which Trade flows uses too.

**What each pool means** was checked against a fresh text save at a full-refresh hour (W2 part-G, hour 60827807). All 3,073 country resources match the save exactly; the product module lists the mapping and evidence. In short:

- supply = `produced` + `adjustment` when positive (from subjects) + `imported`;
- demand = special projects' need (`used` − `special_total`) + production lines' need (−`production_demand`) + `adjustment` when negative (to the overlord) + the trade-law export reserve (`to_export`);
- the net is the game's own: `special_total` + `production_demand`.

The reserve, not what buyers took (`exported`), is what the game takes off what the country has for use.

**Built from the kit:** tooltips are fitted, lines styled, runs cut and dated, values glided and labelled, and partners named by the shared chart and dashboard kit (see [Reuse first](../README.md#reuse-first)); this folder keeps only what the resource pools and deals mean and how their breakdown reads.

**Speed:** the deals are looked up through the kit's interval index, built once per reply, and each hour's breakdowns (tooltips and screen-reader text) are built once and reused while the pointer moves, so a long history of deals costs nothing per hover.

**Freshness:** the pools and the deals are both read every hour, so every value shown is as of its hour. A gap, an incomplete table or a change of country identity ends it; the pools and the deals end apart, so an hour whose deal table is missing still shows the pools.

## Not yet

- **Comparing countries** side by side is left out: the widget shows one country.
- **Special projects' share** is named from the game's breakdown ("Needed by Special Projects", between what the country has for use and its production lines' need) and special projects' `resource_cost`. That no other consumer, such as a `country_resource_cost` modifier, lands in the same step is not checked against a save.
- **Deals and pools** do not always agree: at the checked hour a country's import deals add up to `imported` in 3,055 of 3,073 cases and export deals to `exported` in 3,065. The difference is shown as "Not from a listed deal"; why they differ is not identified.
- **Market purchases** of equipment are not resources and are not shown here.
