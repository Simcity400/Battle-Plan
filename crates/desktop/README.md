# desktop

Tauri host for the replay app. **Campaign map** embeds the native viewer; **Overview** hosts the dashboard. The host owns folder selection, replay library, native-process lifecycle and an asynchronous data proxy. Replay decoding/products remain in the viewer process, shared with map consumers.

Game-derived display preferences initialize executable metadata in the host from the selected game folder. The viewer's dashboard service validates that folder and initializes its own process-wide tables before exposing content-dependent products. Missing or incompatible installation metadata reports a folder/content error; recording metadata remains accessible.

## Develop

Use Windows, Rust 1.96+ with the x64 MSVC toolchain, Visual Studio C++ Build Tools and a Windows SDK, Node.js 22.12+ and Microsoft Edge WebView2. See the [project index](../../README.md) for component guides. Run from the repository root:

```powershell
npm.cmd ci
npm.cmd run dev
```

This builds the viewer, then starts Tauri/Vite. Frontend changes hot reload; desktop Rust rebuilds. Restart development after viewer changes to rebuild its sibling executable. Ctrl+C must stop both host and child. For UI-only layout work use `npm.cmd run dev:ui`; native features need the host.

Viewer arguments pass through both separators:

```powershell
npm.cmd run dev -- -- -- "<campaign.battleplan>"
```

## Build and launch

```powershell
npm.cmd run build:desktop
.\.cargo-target\release\replay-desktop.exe "<campaign.battleplan>"
```

The build also produces [Battle Plan Recorder](../recorder-app/README.md) (`battle-plan-recorder.exe`) and `collector.dll`. Keep `replay-desktop.exe`, `viewer.exe`, `battle-plan-recorder.exe` and `collector.dll` together. Production embeds the frontend, needs WebView2 and forwards viewer arguments such as `--game-dir`/`--user-dir`. This produces an executable directory, not an installer/updater. Retain `LICENSE`, `LICENSES/` and the full shader compiler directory; see [distribution](#shader-compiler-distribution).

## Where to work

| File | Responsibility |
|---|---|
| [main.rs](src/main.rs) | Tauri commands, app setup and native folder/export dialogs. |
| [native.rs](src/native.rs), [native/](src/native/) | Geometry owner, bounded commands, pipe/process/job lifecycle. |
| [dashboard_service.rs](src/dashboard_service.rs) | Asynchronous requests, cancellation and fragmented reply assembly. |
| [replay_library.rs](src/replay_library.rs) | The replay folder shared with Battle Plan Recorder, recursive listing and validated trailer metadata. |
| [compaction.rs](src/compaction.rs) | Recorder compression preference, revision tracking and manual compaction jobs/progress. |
| [recorder.rs](src/recorder.rs) | Starting Battle Plan Recorder with the app, and recording names and campaign flags in the shared recorder settings. |
| [video.rs](src/video.rs) | UI/map scale, VSync and diagnostics preferences and application. |

[dashboard](../dashboard/README.md) owns protocol/products/generated types; [ui](../../ui/README.md) owns shell and widgets; [viewer](../viewer/README.md) owns rendering and battle interaction. Rebuild both host and viewer after shell-control/protocol changes.

## Ownership and lifecycle

The sibling viewer belongs to a Windows job object. Shutdown requests graceful exit before bounded forced reap; never leave an owned child after app/dev closure. Host and child require per-monitor-v2 DPI awareness.

Select the active geometry owner before frontend tabs mount/unmount. Late cleanup from Overview cannot hide Campaign map or vice versa. The standalone map retains camera across tab switches and follows the shared clock while active. Clock coalescing must retain playing state and the seek counter; an unstamped seek is explicit. Preserve the [viewer continuity rules](../viewer/README.md#playback-threading-and-frame-budgets).

Unloading retires the cached map and clears matching open/restart targets before releasing the frontend lease. A running load finishes within its worker slot, then discards its obsolete result; it cannot reopen an unloaded source. Unloading the default selects another loaded campaign or hides the map when none remain. Queries may run before map assets load.

Sources share canonical path/revision but return independently releasable bindings. Closing one widget must not cancel another consumer's query. Process failure terminates pending requests. Restart requires reopening sources; lease IDs include a fresh service namespace so delayed releases cannot close new sources. See [dashboard architecture](../../docs/dashboard.md).

The host assembles bounded reply fragments and forwards raw JSON bytes without parsing recording/product data. Cancellation/admission ordering must prevent a cancelled open from later creating a leaked binding. Map layouts replace snapshots; data commands have a separate bounded count/byte queue. Preserve canonical retryable refusals from [transport.rs](../dashboard/src/transport.rs) and the shared [message budget](../dashboard/src/protocol.rs); never truncate requests naming many countries.

## Folders, settings and exports

The replay folder is the recorder's save folder in the shared `RecorderSettings` (by default `HOI4Replay\captures` under local app data, created when first listed), so the folder this app lists is the one Battle Plan Recorder saves into; choosing a replay folder here changes it there. A folder this app chose in its own `battle-plan\replay-folder.json` (under the roaming app data folder) becomes the shared folder once, when the recorder has none chosen, and that file is then removed, so what the app listed stays listed and no recording moves. Library listing runs off the UI thread, excludes hidden folders/symlinks and obeys its traversal budget. It reads validated trailers to identify partial recordings without decoding frame blocks. A manual recording pick does not change the replay folder.

Settings > Folders > Recordings edits the recorder's **Fast (larger files)** / **Small (slower)** preference through `RecorderSettings::update` at `%LOCALAPPDATA%\HOI4Replay\recorder\settings.json`. Old or missing settings default to Fast. Under Small, [Battle Plan Recorder](../recorder-app/README.md) compacts each recording after HOI4 closes; this app never compacts on its own.

**Compact** in loaded and archived campaigns' sidebar context menus and **Compact all** in Settings work independently of Fast/Small, game presence, recorder presence and loaded status. Settings displays manual recording/block progress, saved space and plain reasons for unchanged files. The game-running note explains that manual compaction may slow it briefly; a minimal ToolHelp snapshot compares each `PROCESSENTRY32W.szExeFile` basename case-insensitively with `hoi4.exe` and reads no process memory or executable path. When a manual attempt changes the file revision, both entry points notify the frontend to use the existing campaign reconnect path: retire its Campaign map, release the old source, reopen the recording in the same campaign slot and bind the new revision. Unchanged recordings do not reload; archived recordings remain closed. Pending opens finish before reconnect, and a campaign unloaded before the notification is handled stays closed.

Work runs one file at a time with Windows thread background mode for CPU and I/O priority; one manual job runs at a time. Cancellation is checked between directory entries, around each library metadata read, and between 64 KiB chunks and frames during compaction. A single filesystem, metadata decode, frame decode or codec call can take longer, so there is no fixed wall-clock cancellation bound. Shutdown signals cancellation and drops owned thread handles without waiting for a long worker step; workers clean up when they next yield, while process exit can leave an excluded temporary candidate or a recoverable backup.

[compaction/store.rs](src/compaction/store.rs) keeps normalized-path/revision/outcome hints at `%LOCALAPPDATA%\HOI4Replay\compaction.json`, which tell whether a recording's revision is already compacted. The app permits multiple instances. A short OS file lock serializes merging pending updates with current disk contents and atomically replacing the file with synced JSON. Changed/appended files lose their hint; missing/unreadable hints only mean a harmless verified retry. Per-recording `compact_recording`, `recording_compaction_status`, folder `compact_replay_folder` and manual `compaction_progress` commands share the same host.

When the app opens and the recorder's **With Battle Plan** setting is on (the default), it starts the sibling `battle-plan-recorder.exe` detached, with no console or inherited pipes and outside any job this app belongs to, so closing the app never stops a recording; the recorder's one-instance rule makes a second start a no-op. Recording display names and sidebar campaign flags live in the recorder's shared settings, so the recorder shows the same ones: `recording_preferences` reads them (flags by the frontend's campaign key), `set_recording_names` sets or clears only the names a change touches, by normalized path, and `set_campaign_flag` sets or clears one campaign's flag, each inside one `RecorderSettings::update`, so neither program's names or flags are lost to the other's save.

Compaction reads with normal sharing, verifies every frame and publishes only a strictly smaller candidate through a backup-validated swap. Concurrent publications are restored instead of discarded; uncertain backups survive errors with their paths reported. The [replay contract](../replay/README.md#finished-recording-compression) describes streaming, block retention, historical compatibility and recovery files.

Native dialogs run off the UI thread, belong to the app window, identify their purpose and start in the current valid directory. Cancellation preserves the field. Game/user folder choices stay drafts until Apply; replay-folder choices save immediately.

`video.rs` stores preferences in APPDATA/battle-plan/video.json. UI scale, map scale and VSync initially inherit HOI4; Revert restores inheritance. *Match game* is the inherited map scale. Apply webview zoom/native presentation with rollback on failure and send applied values to existing/new viewers. These settings do not resize windows/widgets or change fullscreen/monitor modes; dashboard Edit handles own widget size.

The host owns embedded controls and hides the viewer's egui header/timeline. Map actions update the existing native controls and rendering path. Map diagnostics default off; Settings > Diagnostics or F3 toggles/saves `mapDiagnostics`. Native F3 requests the host toggle; `--diagnostics` enables one run without saving. `MapDetails` is requested on demand.

Export carries an optional map-view ID through the native action. Widget exports target that view; standalone exports omit the ID. Global map-settings requests need no source; campaign/widget targets retain source/revision validation. Appearance inheritance and content reload rules belong in the [viewer guide](../viewer/README.md#campaign-appearance).

## Test without taking over the screen

Follow [AGENTS.md](../../AGENTS.md) for scoped Rust/UI checks. Lifecycle changes must exercise open, tab switch, seek, cancellation, unload, close and development shutdown. Native integration requires matched built host/viewer executables.

[desktop_ui.py](../../tools/desktop_ui.py) launches the real host/viewer on an inactive Windows desktop without switching desktops or moving the cursor. It copies executables and isolates APPDATA, LOCALAPPDATA and WebView2 storage under the session directory. It needs Python, a logged-in Windows session and normal graphics/WebView2 support. Build the frontend, viewer and host with `custom-protocol` before launch so the host embeds UI instead of needing Vite. Keep the launcher alive; choose a session directory outside the checkout and use distinct sessions/ports for concurrent runs.

```powershell
npm.cmd run build:ui
cargo build -p viewer
cargo build -p replay-desktop --features custom-protocol
$sessionDir = Join-Path $env:TEMP 'battle-plan-ui'
python tools/desktop_ui.py --session $sessionDir launch .\.cargo-target\debug\replay-desktop.exe
node tools/desktop_cdp.mjs $sessionDir inspect
python tools/desktop_ui.py --session $sessionDir list
```

[desktop_cdp.mjs](../../tools/desktop_cdp.mjs) targets the real Tauri WebView2 using the session's local debugging port. Use mouse/key commands to exercise controls and expressions to inspect state. WebView screenshots exclude native dialogs/map windows; use the Python helper's `capture`, `children`, `resize`, `choose`, `cancel-dialog` and `close` commands for those. `capture` requires Pillow. Native input commands are confined to the isolated desktop.

Settings checks cover Apply, Cancel, Revert, restart persistence, all three folder dialogs and a loaded map; inspect saved session preferences too. Closing the host must reap its viewer: check launcher exit/processes. Ctrl+C requests closure with a bounded fallback. Use a matched smoke replay; this does not replace the [recorder fresh-save oracle](../recorder/README.md). Inactive-desktop checks cannot establish visible-monitor VSync/tearing.

## Shader compiler distribution

`build:desktop` stages the pinned official DXC under `.cargo-target/release/shader-tools/dxc/`. Ship the whole directory with both executables, including runtime DLLs and all three unmodified licences. [The installer](../../tools/install-shader-compiler.ps1) verifies the archive hash and accepts a downloaded archive. Development can stage the same sibling directory or set `BATTLE_PLAN_SHADER_COMPILER`; runtime downloads nothing and never guesses an SDK compiler. Future packaging must preserve these files/terms. [shader-pipeline](../shader-pipeline/README.md#compiler-and-packaging) owns discovery, cache and refusal details.
