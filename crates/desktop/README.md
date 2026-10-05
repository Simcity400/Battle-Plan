# desktop

The Tauri host for the replay application. **Map** embeds the existing native viewer; **Overview** hosts the widget dashboard. The host owns folder selection and the replay library, the native process and an asynchronous data proxy. Recording data and widget products live in the native viewer process, shared with its map consumers.

## Develop

Use the repository's Rust/Windows prerequisites, Node.js 22.12 or newer and the Microsoft Edge WebView2 Runtime. Follow the shared [build commands](../../AGENTS.md#build-commands):

```powershell
npm.cmd ci
bash tools/build-slot.sh "dev/desktop" "npm.cmd run dev"
```

The development command builds the native viewer, then starts Tauri and Vite. Frontend edits hot reload; desktop Rust changes rebuild. Restart the command after changing native viewer code so its sibling executable is rebuilt. Ctrl+C stops development.

Viewer arguments pass through both separators:

```powershell
bash tools/build-slot.sh "dev/desktop" "npm.cmd run dev -- -- -- C:/recordings/campaign.battleplan"
```

Use `npm.cmd run dev:ui` for frontend-only layout work. Native features require the desktop host.

## Build and launch

```powershell
bash tools/build-slot.sh "dev/desktop" "npm.cmd run build:desktop"
.\.cargo-target\release\replay-desktop.exe C:\recordings\campaign.battleplan
```

Keep `replay-desktop.exe` and `viewer.exe` together. Production embeds the frontend and does not need Node or a development server. WebView2 remains a runtime prerequisite. Viewer arguments, including `--game-dir` and `--user-dir`, are forwarded. This workflow produces executables, not an installer or updater. The build copies [third-party notices](../../THIRD_PARTY_NOTICES.md) beside the executables; retain that file and the compiler directory with its upstream licences when distributing the app. Other bundled dependency notices still require the [publication review](../../docs/publication.md).

## Test without taking over the screen

[desktop_ui.py](../../tools/desktop_ui.py) launches the real app and its native viewer on an inactive Windows desktop. It never switches the visible desktop or moves the cursor. It copies the executables and isolates APPDATA, LOCALAPPDATA and WebView2 storage under the supplied session directory, so test settings stay separate. This requires a logged-in Windows session and the normal graphics/WebView2 prerequisites; it is not a service-session or GPU-free renderer.

Build before testing, then keep the launcher running in a background terminal:

```powershell
npm.cmd run build:ui
bash tools/build-slot.sh "dev/desktop" "cargo build -p viewer --bin viewer"
bash tools/build-slot.sh "dev/desktop" "cargo build -p replay-desktop --features custom-protocol"
python tools/desktop_ui.py --session captures\desktop-ui launch .\.cargo-target\debug\replay-desktop.exe
```

The custom-protocol feature embeds the built frontend instead of depending on Vite. Python 3 is required; `capture` also needs Pillow. The launcher keeps the desktop handle alive until the app exits. Use a separate session directory and `--port` for concurrent runs. Its local WebView2 debugging port defaults to 9227 and exists only for this test launch.

From another terminal, [desktop_cdp.mjs](../../tools/desktop_cdp.mjs) inspects and sends mouse/keyboard input to the actual desktop WebView2, rejecting pages without the Tauri host. It uses the port recorded in the session directory and Node's built-in WebSocket support (Node 22.12+). Native dialogs and the embedded map are separate Win32 windows; inspect or capture those with the Python helper:

```powershell
node tools/desktop_cdp.mjs captures\desktop-ui inspect
node tools/desktop_cdp.mjs captures\desktop-ui click-selector '#tab-settings'
python tools/desktop_ui.py --session captures\desktop-ui list
# Replace 12345 with the app HWND returned by list.
python tools/desktop_ui.py --session captures\desktop-ui capture 12345 captures\desktop-ui\settings.png
python tools/desktop_ui.py --session captures\desktop-ui close 12345
```

`children HWND` enumerates native controls; `resize HWND WIDTH HEIGHT` changes the window's physical dimensions without activating it. With exactly one folder dialog open, `choose PATH` selects a folder and `cancel-dialog` cancels it. `message HWND MESSAGE WPARAM LPARAM` and `text HWND VALUE` target native controls on this isolated desktop only. CDP also supports `click X Y`, `key Home|End|Enter|Escape|ArrowDown|ArrowUp|Tab`, `screenshot PATH` (WebView only), and `file PATH` for a JavaScript expression saved outside source. Prefer input commands to exercise controls; use expressions to inspect state. Closing the host must also reap its viewer; check the launcher's exit and process list. Ctrl+C requests closure and provides a bounded fallback for an unresponsive test host.

For settings checks, exercise Apply, Cancel, Revert, restart persistence, all three folder dialogs and a loaded native map. Check saved preferences under the session directory as well as screenshots. Use an existing matched smoke replay for map checks; this does not replace the recorder's fresh-save oracle. Inactive-desktop tests cannot establish visible-monitor VSync/tearing behavior.

## Ownership and lifecycle

Unloading a campaign retires its cached standalone map and clears matching restart/open targets before releasing the frontend lease. A running asset load finishes within its existing worker slot and its obsolete result is discarded; it cannot reopen an unloaded recording. The standalone Map view retains its camera across tab switches and follows the shared top-bar clock through the existing seek command while active. Clock updates include their playing state and a counter that changes only on seeks; coalescing keeps both, so skipped playback hours preserve train/weather animation and an explicit seek resets it. A seek command without clock state is always explicit. Overview and Campaign map share country selection and playback controls; layout editing remains in Overview. Widgets may follow the shared transport, retain a selected period, show the whole campaign or pin a reference moment. Queries can run before map assets are loaded.

The host launches its sibling viewer, contains it in a Windows job object and requests graceful shutdown before the bounded forced-reap fallback. Closing the app or stopping development must not leave an owned viewer process behind. Both host and child require per-monitor-v2 DPI awareness.

`dashboard_service.rs` tracks independent requests and reassembles bounded fragmented replies. Cancellation and admission are ordered so a cancelled source-open cannot start later and leak a binding. The host never parses a reply: the assembled JSON goes to the frontend as raw bytes. One `dashboard_product` command carries every widget data request by product key; `dashboard_open_source`, `dashboard_map_settings`, `dashboard_cancel` and `dashboard_release_source` are the rest of the data surface.

`native.rs` selects the active geometry owner before tabs mount or unmount. Late cleanup from the dashboard cannot hide the standalone map, and vice versa. Map layouts are replacement snapshots; data commands have their own bounded queue. Its count and byte refusals use `NATIVE_QUEUE_FULL`, and busy map controls use `MAP_CONTROLS_BUSY`, from the dashboard’s [retryable refusal catalogue](../dashboard/src/transport.rs), so country facts are retried once the queue drains. Native process failure terminates pending requests rather than leaving widgets loading indefinitely. A restarted process requires sources to be reopened; old source handles are not valid in it. Lease IDs include a fresh service namespace so delayed releases cannot close sources in a restarted process.

The native data service opens one shared source for a canonical path and revision and returns independently releasable client bindings. Several maps and charts may reference it. Closing one widget cannot cancel another consumer's shared query. See the [dashboard architecture](../../docs/dashboard.md) for the ownership model and limits.

## Where to work

The Campaign map settings pop-down sends `map_action` commands to the existing viewer controls. `export_map` and `choose_content_folder` use native system dialogs off the host UI thread. Settings requests `MapDetails` on demand through the normal viewer-state event. The embedded viewer omits its old egui header, timeline and diagnostic strips; its map and native battle interactions retain the existing rendering path. Map diagnostics (technical stats and data limitations over the Campaign map and map widgets) are off by default. **Show diagnostics on maps** in Settings > Diagnostics, or F3 in the app or the native map, toggles them. `video.rs` saves the choice as `mapDiagnostics` in video.json and sends it to the viewer as `MapAction::Diagnostics`; the native map's F3 asks the host through `NativeEvent::ToggleDiagnostics`. `--diagnostics` shows them for one run without saving. These shell-control additions require rebuilding both desktop and viewer before native integration testing.

All three Settings folder fields have Browse buttons. The native dialogs belong to the app window, identify the requested folder in their titles and start in the field's current directory when valid. Cancelling preserves the field. Game/user selections remain drafts until Apply game folders; replay-folder selection saves immediately.

Host implementation is under [src](src/). The [dashboard crate](../dashboard/README.md) owns the protocol, products and generated types; [ui](../../ui/README.md) owns layout and widgets. See [AGENTS.md](../../AGENTS.md) for checks. Exercise open, tab switching, seek, cancellation, close and development shutdown when lifecycle behavior changes.

`replay_library.rs` lists replay filenames recursively without decoding recording data. The Settings folder is shared with browser development through the user configuration file described in the frontend guide. Folder selection and the recording library's manual file picker use native system dialogs; Load opens through the existing source service. Choosing an individual recording does not change the replay folder. Directory listing runs off the UI thread, excludes hidden folders and symbolic links, and has a traversal budget. Unloading the current default selects another loaded campaign, or clears the default and hides the native map when none remain.

`video.rs` owns application UI scale, native-map VSync and the map UI scale in APPDATA/battle-plan/video.json. They inherit the configured HOI4 preferences initially; Revert to HOI4 settings restores inheritance. The map UI scale's inherited value is *Match game*; the host sends it, and VSync, to each viewer it launches and whenever they are applied. The host applies webview zoom and the native surface presentation command, with rollback on failure. Removed resolution/window-mode fields in older preference files are ignored. There is no settings path that resizes windows or widgets, changes fullscreen mode, or queries monitor refresh rates. Widgets use the existing dashboard Edit handles.

The native export dialog accepts an optional dashboard view ID and carries it through the map action. The frontend uses this from the shared map menu in widget headers; standalone export omits the ID.

Map-settings requests forward a global, campaign or widget target through the existing bounded service. Global requests require no recording; campaign/widget requests retain source/revision validation.

## Shader compiler distribution

`build:desktop` stages the pinned official DXC distribution in `.cargo-target/release/shader-tools/dxc/`. Ship that whole directory beside `viewer.exe` and `replay-desktop.exe`: the executable, dxcompiler.dll, dxil.dll and all three upstream licence files. [The installer](../../tools/install-shader-compiler.ps1) verifies the archive hash and supports an already downloaded archive. This is part of the executable-directory build; a future installer/updater must retain these files and the distribution terms. Development can stage the same directory beside its viewer or explicitly set `BATTLE_PLAN_SHADER_COMPILER`. Runtime performs no download and never guesses an SDK compiler. See [shader-pipeline](../shader-pipeline/README.md) for the exact cache location, invalidation, runtime refusal and licence contract.
