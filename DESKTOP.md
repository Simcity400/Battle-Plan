# Replay desktop

Two side tabs. **Map** hosts the existing native viewer, with its own controls and wgpu rendering, as a child window. **Analytics** is a React page. Switching tabs keeps the map loaded with its camera and hour; hidden playback pauses and resumes where it left off.

## Develop

Needs the repository's Rust toolchain and Windows build prerequisites, Node.js 22.12 or newer, and the Microsoft Edge WebView2 Runtime.

```powershell
npm.cmd ci
npm.cmd run dev
```

`dev` builds the native viewer into `.cargo-target`, then runs Tauri and Vite together. Frontend edits hot reload and desktop Rust edits rebuild automatically; native viewer edits need a restart of `npm.cmd run dev` so the sibling `viewer.exe` is rebuilt. Stop with Ctrl+C. Viewer arguments pass through both separators: `npm.cmd run dev -- -- -- C:\recordings\campaign.battleplan`.

**Open recording** picks a `.battleplan`. The native viewer's **Folders** control still selects the game and user directories the map needs. Analytics reads the recording without map assets. Opening a recording while Analytics is selected defers the native map load until Map is shown.

`npm.cmd run dev:ui` serves the frontend alone at `http://127.0.0.1:1420` for layout work; native features report that they need the desktop app.

## Build

```powershell
npm.cmd run build:desktop
```

Produces `.cargo-target\release\replay-desktop.exe` and `viewer.exe`; keep them together. The frontend is embedded, so a production launch needs neither Node nor a dev server. Viewer arguments are forwarded:

```powershell
.\.cargo-target\release\replay-desktop.exe C:\recordings\campaign.battleplan `
  --game-dir 'C:\Games\Hearts of Iron IV' --user-dir 'C:\Users\Example\Documents\Paradox Interactive\Hearts of Iron IV'
```

No installer or updater; WebView2 is a runtime prerequisite.

## Analytics

Country history by branch over an inclusive hour range: political power, command power, stability, war support and convoys. A selected sample can be opened on the map. Values keep their native fixed-point integers; the UI receives decimal strings plus a scale and formats them with integer arithmetic. Floating point is used only for chart coordinates.

A query scans the branch's blocks once and keeps at most 1,200 intervals. A multi-hour interval keeps its first and last samples and exact per-metric minimum and maximum; the path between them is not retained, so query a smaller range for detail. Missing, truncated, country-absent and recorded-zero values stay distinct, and the chart breaks across non-recorded coverage. The country list is built from observed snapshots, not a census of every hour. Branches are never joined through probable parent links.

## Components

| Location | Responsibility |
|---|---|
| `ui/` | React/TypeScript layout, navigation, exact-value display, lazy ECharts history. |
| `crates/analytics/` | Replay queries and generated data types. No map assets. |
| `crates/desktop/` | Tauri host, file picker, one analytics worker, native viewer lifecycle. |
| `crates/desktop-protocol/` | Control messages and the file revision marker shared by host and viewer. |
| `crates/viewer/src/embedded.rs` | Windows child-window adapter and control pipe. |

The host launches its sibling `viewer.exe` and sends coalesced geometry, visibility, open and seek commands over a line-oriented pipe; no map images or replay tables cross it. A Windows job object contains the child, and shutdown asks it to close before reaping it after two seconds. The adapter requires per-monitor-v2 DPI awareness on both sides.

Analytics runs one job at a time with one replaceable pending job; newer requests cancel older scans between blocks and frames. One decoded replay and one completed response stay cached.

The revision marker is file length plus modification time. Analytics results carry it, and a map seek is refused when the file has changed since the analysis.

## Checks

```powershell
npm.cmd run check:ui
npm.cmd run build:ui
cargo run -p analytics --bin export-types -- --check
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

After changing analytics DTOs, regenerate `ui/src/generated/analytics.ts` with `cargo run -p analytics --bin export-types`.
