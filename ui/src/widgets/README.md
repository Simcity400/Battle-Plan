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

- `WidgetProps` supplies spec, runtime, `active`, header slots and diagnostics. Bodies mount after their frame, in turn (those near the viewport first, a few each animation frame), and `active` starts from the widget's visibility. Report warnings through [useDiagnostics](../dashboard/useDiagnostics.ts). Widgets never open files, start clocks or read another widget's state.
- Read validated histories through [useProduct](../dashboard/products.ts), then look up playback locally. Windows/transient facts and client/request ownership follow the [data guide](../../README.md#data-and-identity).
- Follow with `usePlaybackHour` and `resolveWindow`. Declare time modes/defaults; `history` controls the whole-campaign Live switch, while moment views can offer Pin. Store branch-relative offsets; missing is not zero.
- The frame owns title, country chip, Live and gear. Portal summaries into `headerSlot`; semantic drills use `countryChip`/`countrySlot` at their displayed hour. Custom controls use `headerControls`.
- Read declared options with `resolveOptions`, save through `setOption`, and load content choices with `useOptionChoices`. Settings save immediately; layout edits wait for Save. Version changes need `migrate`; unsupported settings stay stored.
- The [picker](../dashboard/WidgetPicker.tsx) uses metadata; gated entries require `available: true`. `pickerItems` offers content presets through the shared button, storing the same kind with initial title/options.
- Use shared [country/groups](../dashboard/groups.ts), recorded identity and dated labels, with no chosen cap on unrestricted lists. Only drawn flags load; missing flags retain grey placeholders/reasons.
- Compose [DrillHost](../charts/drill/DrillHost.tsx), `DrillParts`, `DRILL_OPTION` and `breakdownOption` for Follow/picked moments. Pin uses `pinnedBreakdown`/`runtime.addWidgetBelow`, preserving kind/campaign, branch and native slot/ID at the original choice hour. Histories follow that holder's aliases, exclude tag reuse and refuse missing/invalid origins; country headers use the body's displayed date or stay neutral. Follow uses the track-aligned clock independently of the parent's time mode. Menus use [jumpItem](../charts/jump.ts); states use `WidgetState`.

## Reuse first

Share drawing/interaction in the [chart kit](../charts/README.md) and data/layout helpers in [dashboard/](../dashboard/).

[Event feed](event-feed/README.md) keeps native roles, event dates and supplying reads separate. [Equipment produced](equipment-produced/README.md) and [Production spending](production-spending/README.md) combine independently covered equipment/rocket sources; missing native world/load lifetime leaves the affected source unknown. Hook lifetime belongs to [collector](../../../crates/collector/README.md#how-it-works).

## Adding a widget

1. Create the folder and reuse/add a [Rust product](../../../crates/dashboard/README.md); regenerate TypeScript when its contract changes.
2. Register metadata in `definitions.ts` and React exports in `index.ts`.
3. Keep synthetic data tests beside the module; follow [AGENTS.md](../../../AGENTS.md) and [frontend checks](../../README.md#development).
