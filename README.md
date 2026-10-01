# Battle Plan

Record a Hearts of Iron IV campaign, replay it on an interactive map, and explore its statistics over time.

The recorder reads the game's state without controlling the game. The desktop app has a native **Campaign map** tab and an **Overview** [dashboard](crates/dashboard/README.md). Recordings use `.battleplan` files; playback does not need access to live game memory.

## Start here

This is a Windows project for a specific, hash-verified HOI4 build. Unsupported game executables are refused. Rust is pinned in [rust-toolchain.toml](rust-toolchain.toml); build settings are in [.cargo/config.toml](.cargo/config.toml). Development rules and validation commands live in [AGENTS.md](AGENTS.md).

Desktop development needs the pinned Rust toolchain, Windows MSVC build tools, `lld-link`, `sccache`, Node.js 22.12 or newer, and the Microsoft Edge WebView2 Runtime. Follow the [desktop guide](crates/desktop/README.md#develop) to start the app.

Choose **+** beside **Campaigns** in the sidebar, set your replay folder when prompted (or in Settings), and load a campaign from the list. The map also needs the matching game and user folders; Overview widgets can read the recording without map assets.

To make a recording, follow the [recorder guide](crates/recorder/README.md). For builds, launch arguments and desktop packaging, see the [desktop guide](crates/desktop/README.md).

The [dashboard architecture](docs/dashboard.md) describes the crate groups, the data-product flow and the identity and time rules widgets follow.

## Find your way around

Each crate's README explains its responsibility, important rules and development entry points.

| Component | What belongs here |
|---|---|
| [shader-pipeline](crates/shader-pipeline/README.md) | Executable native shader compiler, persistent cache and GPU pass binding. |
| [engine](crates/engine/README.md) | Supported game build, native layouts and guarded memory reads. |
| [collector](crates/collector/README.md) | Injected DLL, hourly acquisition and recording pipeline. |
| [recorder](crates/recorder/README.md) | Attach/detach, replay finalization, inspection and save comparison. |
| [replay](crates/replay/README.md) | Shared recording format, tables, branches, cadence and sample history. |
| [installed-content](crates/installed-content/README.md) | Game assets, text saves, content layers and identity verification. |
| [viewer](crates/viewer/README.md) | The native map process: rendering, playback, game-derived visuals and image export. |
| [dashboard](crates/dashboard/README.md) | Widget data products, the native data service, the host/viewer protocol and the generated frontend types. |
| [desktop](crates/desktop/README.md) | Tauri host and the native viewer's lifecycle. |
| [ui](ui/README.md) | React interface: shell, shared components, the widget framework and one folder per widget. |

## Working on the project

Read [AGENTS.md](AGENTS.md) for development rules and validation commands. Detailed references stay beside their subject: [engine readers](crates/engine/README.md#source-map), [map rendering](crates/viewer/README.md#rendering-rules), [reverse-engineering tools](tools/re/README.md), and [reference captures](reference/).

Preserve exact recorded values and distinguish missing data from zero. Show gaps, uncertainty and carried samples honestly. A passing build is not proof of capture correctness, map fidelity or game-speed performance.

These documents describe how the project works. Old plans and decisions remain in Git history; experiment logs and captures do not belong in the documentation or source control.
