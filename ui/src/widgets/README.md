# Widgets

Each folder owns one widget's metadata, rendering, payload validation and wording.

```text
widgets/<name>/
  definition.ts     metadata/settings
  index.ts          React exports
  <Name>Widget.tsx   body/header slots
  <name>Data.ts      validation/lookups/tests
  <name>.css
  README.md         purpose, products, notable behaviour
```

[definitions.ts](definitions.ts) registers pure metadata; [index.ts](index.ts) pairs every kind with its renderer, checked by TypeScript. Types live in the [dashboard contract](../dashboard/definitions.ts).

## Contract

- `WidgetProps` supplies spec, runtime, `active`, header slots and `onDiagnostics`. Report each note behind what the widget cannot show, named and counted, through `onDiagnostics` for tests; the dashboard does not display them, so never render product warnings in the body. Bodies mount after their frame, in turn (those near the viewport first, a few each animation frame), and `active` starts from the widget's visibility. Report warnings through [useDiagnostics](../dashboard/useDiagnostics.ts). Widgets never open files, start clocks or read another widget's state.
- Read validated histories through [useProduct](../dashboard/products.ts), then look up playback locally. Windows/transient facts and client/request ownership follow the [data guide](../../README.md#data-and-identity).
- Follow with `usePlaybackHour` and `resolveWindow`. Declare time modes/defaults; `history` controls the whole-campaign Live switch, while moment views can offer Pin. Store branch-relative offsets; missing is not zero.
- The frame owns title, country chip, Live and gear. Portal summaries into `headerSlot`; semantic drills use `countryChip`/`countrySlot` at their displayed hour. Custom controls use `headerControls`.
- Read declared options with `resolveOptions`, save through `setOption`, and load content choices with `useOptionChoices`. Settings save immediately; layout edits wait for Save. Version changes need `migrate`; unsupported settings stay stored.
- Declare `factionColours` when the widget uses shared campaign faction colours. Its settings then offer the campaign-wide colour/name editor for its resolved campaign.
- The [catalogue](../dashboard/WidgetPicker.tsx) uses metadata; gated entries require `available: true`. It groups widgets by `category`, else by the first subject page that lists the kind ([widgetCategories.ts](../dashboard/widgetCategories.ts)), with a category rail (plus an On this page view of what the page already has), a search over title, description and group, and a card per widget. The selected card is drawn live from the loaded campaign by [WidgetPreview](../dashboard/WidgetPreview.tsx) at the details pane's width and the widget's default height, unscaled and scrollable (clicks stop at its edge), at the recording's end (`recordingEndTime`: the whole campaign, else a pin on the last hour), since opening hours are often empty; set `preview: false` when a second copy cannot be drawn (the native map). Adding keeps the catalogue open. A widget whose View, Show or similar setting switches between genuinely different views lists the others as `variants`: each is its own card, right after the widget's own, adding the same kind with an initial title naming the view and its options; the setting stays switchable in every copy. Shared families build theirs from their option builders: `raceViewVariants` (*Army size over time*, *Occupation trend race*), `breakdownVariants` (one card per Show breakdown, titled as Pin as widget titles it, `breakdownTitle`; Production lines' breakdowns need a chosen line, so they stay reachable through its widget) and `battleVariants` (*Biggest land battles*); `choiceVariants` covers any other choice. Measures, perspectives and display choices stay settings only. A card counts the page's copies that show its view, reading options as the widget does; the own card counts those in the default of every option its views change. A widget whose content splits into kinds declares `usePresets`: each preset (one idea category) is its own card storing the same kind with initial title/options, and replaces the widget's own card while there are any; its views still follow.
- Use shared [country/groups](../dashboard/groups.ts), recorded identity and dated labels, with no chosen cap on unrestricted lists. Only drawn flags load; missing flags retain grey placeholders/reasons.
- Compose [DrillHost](../charts/drill/DrillHost.tsx), `DrillParts`, `DRILL_OPTION` and `breakdownOption` for Follow/picked moments. Pin uses `pinnedBreakdown`/`runtime.addWidgetBelow`, preserving kind/campaign, branch and native slot/ID at the original choice hour. Histories follow that holder's aliases, exclude tag reuse and refuse missing/invalid origins; country headers use the body's displayed date or stay neutral. An unpinned breakdown, such as a catalogue card, reads *Choose a country* until its body has a holder: a body that lets you choose one (Army, Supply, Naval losses) keeps the header and names the holder it opened or was given; the others use [breakdownCountryChip](../dashboard/CountryChip.tsx), which gives the header to the body only once Pin as widget saved a country. Follow uses the track-aligned clock independently of the parent's time mode. Menus use [jumpItem](../charts/jump.ts); states use `WidgetState`.

## Reuse first

Share drawing/interaction in the [chart kit](../charts/README.md) and data/layout helpers in [dashboard/](../dashboard/).

[Event feed](event-feed/README.md) keeps native roles, event dates and supplying reads separate. [Equipment produced](equipment-produced/README.md) and [Production spending](production-spending/README.md) combine independently covered equipment/rocket sources; missing native world/load lifetime leaves the affected source unknown. Hook lifetime belongs to [collector](../../../crates/collector/README.md#how-it-works).

## Adding a widget

1. Create the folder and reuse/add a [Rust product](../../../crates/dashboard/README.md); regenerate TypeScript when its contract changes.
2. Register metadata in `definitions.ts` and React exports in `index.ts`.
3. Keep synthetic data tests beside the module; follow [AGENTS.md](../../../AGENTS.md) and [frontend checks](../../README.md#development).
