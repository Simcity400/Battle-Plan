# desktop

Tauri host for the replay app. **Campaign map** embeds the native viewer; **Overview** hosts the dashboard. The host owns folder selection, replay library, native-process lifecycle and an asynchronous data proxy. Replay decoding/products remain in the viewer process, shared with map consumers.

## Develop

Use Node.js 22.12+ and Microsoft Edge WebView2; see the [project index](../../README.md) for component guides. Run from the worktree root using the [shared wrapper](../../AGENTS.md#builds-and-merging):

```powershell
npm.cmd ci
bash tools/build-slot.sh "dev/desktop" "npm.cmd run dev"
```

This builds the viewer, then starts Tauri/Vite. Frontend changes hot reload; desktop Rust rebuilds. Restart development after viewer changes to rebuild its sibling executable. Ctrl+C must stop both host and child. For UI-only layout work use `npm.cmd run dev:ui`; native features need the host.

Viewer arguments pass through both separators:

```powershell
bash tools/build-slot.sh "dev/desktop" "npm.cmd run dev -- -- -- C:/recordings/campaign.battleplan"
```

## Build and launch

```powershell
bash tools/build-slot.sh "dev/desktop" "npm.cmd run build:desktop"
.\.cargo-target\release\replay-desktop.exe C:\recordings\campaign.battleplan
```

Keep `replay-desktop.exe` and `viewer.exe` together. Production embeds the frontend, needs WebView2 and forwards viewer arguments such as `--game-dir`/`--user-dir`. This produces an executable directory, not an installer/updater. Retain [third-party notices](../../THIRD_PARTY_NOTICES.md) and the full shader compiler directory; see [distribution](#shader-compiler-distribution) and [publication guidance](../../docs/publication.md).

## Where to work

| File | Responsibility |
|---|---|
| [main.rs](src/main.rs) | Tauri commands, app setup and native folder/export dialogs. |
| [native.rs](src/native.rs), [native/](src/native/) | Geometry owner, bounded commands, pipe/process/job lifecycle. |
| [dashboard_service.rs](src/dashboard_service.rs) | Asynchronous requests, cancellation and fragmented reply assembly. |
| [replay_library.rs](src/replay_library.rs) | Replay folder preferences, recursive listing and validated trailer metadata. |
| [video.rs](src/video.rs) | UI/map scale, VSync and diagnostics preferences and application. |

[dashboard](../dashboard/README.md) owns protocol/products/generated types; [ui](../../ui/README.md) owns shell and widgets; [viewer](../viewer/README.md) owns rendering and battle interaction. Rebuild both host and viewer after shell-control/protocol changes.

## Ownership and lifecycle

The sibling viewer belongs to a Windows job object. Shutdown requests graceful exit before bounded forced reap; never leave an owned child after app/dev closure. Host and child require per-monitor-v2 DPI awareness.

Select the active geometry owner before frontend tabs mount/unmount. Late cleanup from Overview cannot hide Campaign map or vice versa. The standalone map retains camera across tab switches and follows the shared clock while active. Clock coalescing must retain playing state and the seek counter; an unstamped seek is explicit. Preserve the [viewer continuity rules](../viewer/README.md#playback-threading-and-frame-budgets).

Unloading retires the cached map and clears matching open/restart targets before releasing the frontend lease. A running load finishes within its worker slot, then discards its obsolete result; it cannot reopen an unloaded source. Unloading the default selects another loaded campaign or hides the map when none remain. Queries may run before map assets load.

Sources share canonical path/revision but return independently releasable bindings. Closing one widget must not cancel another consumer's query. Process failure terminates pending requests. Restart requires reopening sources; lease IDs include a fresh service namespace so delayed releases cannot close new sources. See [dashboard architecture](../../docs/dashboard.md).

The host assembles bounded reply fragments and forwards raw JSON bytes without parsing recording/product data. Cancellation/admission ordering must prevent a cancelled open from later creating a leaked binding. Map layouts replace snapshots; data commands have a separate bounded count/byte queue. Preserve canonical retryable refusals from [transport.rs](../dashboard/src/transport.rs) and the shared [message budget](../dashboard/src/protocol.rs); never truncate requests naming many countries.

## Folders, settings and exports

Library listing runs off the UI thread, excludes hidden folders/symlinks and obeys its traversal budget. It reads validated trailers to identify partial recordings without decoding frame blocks. A manual recording pick does not change the replay folder.

Native dialogs run off the UI thread, belong to the app window, identify their purpose and start in the current valid directory. Cancellation preserves the field. Game/user folder choices stay drafts until Apply; replay-folder choices save immediately.

`video.rs` stores preferences in APPDATA/battle-plan/video.json. UI scale, map scale and VSync initially inherit HOI4; Revert restores inheritance. *Match game* is the inherited map scale. Apply webview zoom/native presentation with rollback on failure and send applied values to existing/new viewers. These settings do not resize windows/widgets or change fullscreen/monitor modes; dashboard Edit handles own widget size.

The host owns embedded controls and hides the viewer's egui header/timeline. Map actions update the existing native controls and rendering path. Map diagnostics default off; Settings > Diagnostics or F3 toggles/saves `mapDiagnostics`. Native F3 requests the host toggle; `--diagnostics` enables one run without saving. `MapDetails` is requested on demand.

Export carries an optional map-view ID through the native action. Widget exports target that view; standalone exports omit the ID. Global map-settings requests need no source; campaign/widget targets retain source/revision validation. Appearance inheritance and content reload rules belong in the [viewer guide](../viewer/README.md#campaign-appearance).

## Test without taking over the screen

Follow [AGENTS.md](../../AGENTS.md) for scoped Rust/UI checks. Lifecycle changes must exercise open, tab switch, seek, cancellation, unload, close and development shutdown. Native integration requires matched built host/viewer executables.

[desktop_ui.py](../../tools/desktop_ui.py) launches the real host/viewer on an inactive Windows desktop without switching desktops or moving the cursor. It copies executables and isolates APPDATA, LOCALAPPDATA and WebView2 storage under the session directory. It needs a logged-in Windows session and normal graphics/WebView2 support. Build the frontend, viewer and host with `custom-protocol` before launch so the host embeds UI instead of needing Vite. Keep the launcher alive; use distinct sessions/ports for concurrent runs.

```powershell
python tools/desktop_ui.py --session captures\desktop-ui launch .\.cargo-target\debug\replay-desktop.exe
node tools/desktop_cdp.mjs captures\desktop-ui inspect
python tools/desktop_ui.py --session captures\desktop-ui list
```

[desktop_cdp.mjs](../../tools/desktop_cdp.mjs) targets the real Tauri WebView2 using the session's local debugging port. Use mouse/key commands to exercise controls and expressions to inspect state. WebView screenshots exclude native dialogs/map windows; use the Python helper's `capture`, `children`, `resize`, `choose`, `cancel-dialog` and `close` commands for those. `capture` requires Pillow. Native input commands are confined to the isolated desktop.

Settings checks cover Apply, Cancel, Revert, restart persistence, all three folder dialogs and a loaded map; inspect saved session preferences too. Closing the host must reap its viewer: check launcher exit/processes. Ctrl+C requests closure with a bounded fallback. Use a matched smoke replay; this does not replace the [recorder fresh-save oracle](../recorder/README.md). Inactive-desktop checks cannot establish visible-monitor VSync/tearing.

## Shader compiler distribution

`build:desktop` stages the pinned official DXC under `.cargo-target/release/shader-tools/dxc/`. Ship the whole directory with both executables, including runtime DLLs and all three unmodified licences. [The installer](../../tools/install-shader-compiler.ps1) verifies the archive hash and accepts a downloaded archive. Development can stage the same sibling directory or set `BATTLE_PLAN_SHADER_COMPILER`; runtime downloads nothing and never guesses an SDK compiler. Future packaging must preserve these files/terms. [shader-pipeline](../shader-pipeline/README.md#compiler-and-packaging) owns discovery, cache and refusal details.
