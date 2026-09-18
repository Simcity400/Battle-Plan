# desktop

The Tauri host for the replay application. **Map** embeds the existing native viewer; **Analytics** hosts the React interface. The host owns file selection, analytics work and the viewer process, not a second renderer.

## Develop

Use the repository's Rust/Windows prerequisites, Node.js 22.12 or newer and the Microsoft Edge WebView2 Runtime. Run from the repository root:

```powershell
npm.cmd ci
npm.cmd run dev
```

The development command builds the native viewer, then starts Tauri and Vite. Frontend edits hot reload; desktop Rust changes rebuild. Restart the command after changing native viewer code so its sibling executable is rebuilt. Ctrl+C stops development.

Viewer arguments pass through both separators:

```powershell
npm.cmd run dev -- -- -- C:\recordings\campaign.battleplan
```

Use `npm.cmd run dev:ui` for frontend-only layout work. Native features require the desktop host.

## Build and launch

```powershell
npm.cmd run build:desktop
.\.cargo-target\release\replay-desktop.exe C:\recordings\campaign.battleplan
```

Keep `replay-desktop.exe` and `viewer.exe` together. Production embeds the frontend and does not need Node or a development server. WebView2 remains a runtime prerequisite. Viewer arguments, including `--game-dir` and `--user-dir`, are forwarded. This workflow produces executables, not an installer or updater.

## Ownership and lifecycle

Switching tabs retains the loaded map, camera and hour. Hidden playback pauses and resumes where it left off. Opening a replay while Analytics is selected defers the map load until Map is shown.

The host launches its sibling viewer, contains it in a Windows job object and requests graceful shutdown before the bounded forced-reap fallback. Closing the app or stopping development must not leave an owned viewer process behind. Both host and child require per-monitor-v2 DPI awareness.

Analytics uses one running job and one replaceable pending job. New requests cancel obsolete scans; one decoded replay and one completed response are cached. Do not replace this with an unbounded queue. Check a response's file revision before seeking the map.

## Where to work

Host implementation is under [src](src/). The [protocol crate](../desktop-protocol/README.md) owns shared messages; [analytics](../analytics/README.md) owns queries and types; [ui](../../ui/README.md) owns layout and charts. See [AGENTS.md](../../AGENTS.md) for checks. Exercise open, tab switching, seek, cancellation, close and development shutdown when lifecycle behavior changes.
