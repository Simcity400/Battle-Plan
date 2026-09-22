# Widgets

Each folder is one widget: everything it owns lives there, and nothing elsewhere switches on its kind.

```
widgets/<name>/
  definition.ts   pure metadata: kind, version, title, icon, time modes, sizes
  index.ts        the React renderer (and header controls if any)
  <Name>Widget.tsx
  <name>Data.ts   payload validation and local playback lookups, with tests
  <name>.css
  README.md       what it shows, which product it reads, what remains
```

`definitions.ts` (this folder) lists every widget's metadata and registers it, for node tests and the layout model. `index.ts` pairs each kind with its renderer and registers the full widgets; TypeScript rejects a kind missing from either list.

## Contract

- The dashboard gives a widget `WidgetProps`: its spec, the shared runtime (tracks, clock, client, maps), whether it is on screen, the map options, a header slot and a diagnostics callback. See `dashboard/definitions.ts`.
- Data comes from one product per request through `useProduct` in `dashboard/products.ts`. A product reply covers the whole branch, so following playback is a local lookup with no further requests.
- Playback: subscribe with `usePlaybackHour` only when the widget follows the clock, and resolve the hour with `resolveWindow`.
- A widget never opens files, starts a clock or reads another widget's state. Header controls, icon and country choice come from its definition.
- Validate the product payload shape in `<name>Data.ts`; the transport already checks source identity.
- Register the Rust product in `crates/dashboard/src/products/`; regenerate `generated/dashboard.ts` when its types change.

## Adding a widget

1. Create the folder above and implement `Product` in `crates/dashboard/src/products/<name>.rs`.
2. Add its metadata to `widgets/definitions.ts` and its renderer to `widgets/index.ts`.
3. Run `npm run check:ui`, `npm run test:ui` and the checks in `AGENTS.md`.
