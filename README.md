# Battle Plan

Record a Hearts of Iron IV campaign, replay it on an interactive map, and explore its statistics over time. The recorder reads game state without controlling the game; the desktop app provides Campaign map and Overview views. Playback reads `.battleplan` files without live game memory.

This is a Windows project for a hash-verified game executable. Start with the [desktop guide](crates/desktop/README.md) to run the app or the [recorder guide](crates/recorder/README.md) to record a campaign. Shared development, validation and merge rules live in [AGENTS.md](AGENTS.md).

## Components

Each component's README describes its layout, concepts and local rules.

| Component | Responsibility |
|---|---|
| [engine](crates/engine/README.md) | Supported executable, native layouts, guarded memory readers and hook site descriptors. |
| [collector](crates/collector/README.md) | Injected DLL, native observation hooks, acquisition and recording pipeline. |
| [recorder](crates/recorder/README.md) | Attach/detach, replay finalization, inspection and save comparison. |
| [replay](crates/replay/README.md) | Recording format, tables, branches and sample history. |
| [installed-content](crates/installed-content/README.md) | Recorded content stack, game assets, text saves and identity verification. |
| [shader-pipeline](crates/shader-pipeline/README.md) | Native shader compiler, persistent cache and GPU pass binding. |
| [viewer](crates/viewer/README.md) | Native map rendering, playback, game-derived visuals and image export. |
| [dashboard](crates/dashboard/README.md) | Widget data products, data service, host/viewer protocol and frontend types. |
| [wire](crates/wire/README.md) | Binary query replies, validating readers, shape contracts and synthetic cross-language fixtures. |
| [desktop](crates/desktop/README.md) | Tauri host, application setup, packaging and native viewer lifecycle. |
| [UI](ui/README.md) | React interface, navigation, layouts, playback and widgets. |

The [dashboard architecture](docs/dashboard.md) explains how recordings become data products and widgets.

## UI areas

| Area | Responsibility |
|---|---|
| [Shell](ui/src/shell/) | Navigation, Settings, campaign views, native viewer glue and Tauri API wrappers. |
| [Appearance](ui/src/shell/appearance/) | Themes, backdrops, fonts and appearance preferences. |
| [Shared components](ui/src/components/) | Reusable controls, popovers, tooltips, country choices and flags. |
| [Dashboard framework](ui/src/dashboard/) | Layout storage, shared playback clock, campaign sources, data client and product cache. |
| [Charts](ui/src/charts/README.md) | Canvas charts, tables, axes, formatting and viewport windowing. |
| [Widgets](ui/src/widgets/README.md) | Widget registration and individual widget guides, including the map widget. |
| [Generated types](ui/src/generated/) | TypeScript contracts generated from Rust. |

## Diagnostics

| Area | Responsibility |
|---|---|
| [Foreign-unwind fixture](tools/tap-foreign-unwind/README.md) | Out-of-game C++ and SEH checks for the production Tap. |

## Recording and dependency checks

Shared recordings are untrusted input. The [replay reader](crates/replay/README.md)
validates container and compressed-block framing and uses checked growth; the
[dashboard](crates/dashboard/README.md) isolates failed requests. Synthetic
[cargo-fuzz targets](fuzz/README.md) exercise opening, partial trailers, blocks,
frames and recording metadata without a game installation.

Run dependency checks after lockfile/dependency changes and before releases:

```powershell
cargo install cargo-deny --locked
cargo deny check advisories
npm.cmd run audit:ui
```

[deny.toml](deny.toml) uses the RustSec database with no ignored advisories.
Vulnerabilities fail the check; review other advisory notices. The npm check audits
the root lockfile's UI runtime and development dependencies against the registry.
Both require network access for current advisory data. Review fixes and run the
affected checks before updating dependencies; audit results do not validate the
recording reader or replace fuzzing.

## License

Battle Plan is free software under the [MIT License](LICENSE).

The image decoders and archive reader in `crates/installed-content` include modified Rust adaptations of the Independent JPEG Group's libjpeg 9a, libpng 1.6.16 and PhysicsFS. This software is based in part on the work of the Independent JPEG Group. The refresh icon is from Lucide. Their notices are in [LICENSES](LICENSES/).

Battle Plan is an unofficial project and is not affiliated with or endorsed by Paradox Interactive. Hearts of Iron IV is a trademark of Paradox Interactive AB. No installed content are included; the app reads them from your own installation.
