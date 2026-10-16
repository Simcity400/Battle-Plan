# desktop

Tauri host for the replay app. **Campaign map** embeds the native viewer; **Overview** hosts the dashboard. The host owns folder selection, replay library, native-process lifecycle and the dashboard data runtime. Dashboard queries use the shared data library directly in the host, independently of the map process.

Game-derived display preferences and content products initialize executable metadata in their process from the selected game folder. The host owns the game/user folders and the background-preparation choice in `APPDATA/battle-plan/content.json` ([config.rs](../dashboard-core/src/config.rs)); this run's `--game-dir`/`--user-dir` override them without saving. Folder Apply, the preparation choice and the development bridge update that file under an exclusive `content.lock`, so overlapping saves keep each other's choices. The host data runtime, the video settings and the viewer all use those effective folders: the viewer starts with them on its command line (or with the last folders Apply sent, if a launch races it), and an applied change reaches a running viewer as a `Folders` map action. The viewer saves no settings. Missing or incompatible installation metadata refuses dependent content; recording metadata remains accessible.

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
| [preparation.rs](src/preparation.rs) | Startup and idle library preparation, with independent game-presence polling and cancellation. |
| [game_process.rs](src/game_process.rs) | Passive game-process presence check for automatic preparation. |
| [recorder.rs](src/recorder.rs) | Starting Battle Plan Recorder with the app, reading its settings, and sharing recording names and campaign flags. |
| [video.rs](src/video.rs) | UI/map scale, VSync and diagnostics preferences and application. |

[dashboard](../dashboard/README.md) owns protocol/products/generated types; [ui](../../ui/README.md) owns shell and widgets; [viewer](../viewer/README.md) owns rendering and battle interaction. Rebuild both host and viewer after shell-control/protocol changes.

## Ownership and lifecycle

The sibling viewer belongs to a Windows job object. Shutdown requests graceful exit before bounded forced reap; never leave an owned child after app/dev closure. Host and child require per-monitor-v2 DPI awareness.

Select the active geometry owner before frontend tabs mount/unmount. Late cleanup from Overview cannot hide Campaign map or vice versa. The Campaign map retains its camera across tab switches and follows the shared clock while active. Clock coalescing must retain playing state and the seek counter; an unstamped seek is explicit. Preserve the [viewer continuity rules](../viewer/README.md#playback-threading-and-frame-budgets).

Unloading retires the cached map and clears matching open/restart targets before releasing the frontend lease. A running load finishes within its worker slot, then discards its obsolete result; it cannot reopen an unloaded source. Unloading the default selects another loaded campaign or hides the map when none remain. Queries may run before map assets load.

Sources share canonical path/revision but return independently releasable bindings. Closing one widget must not cancel another consumer's query. Host data-runtime failure terminates pending requests. Reopening uses a fresh service namespace so delayed releases cannot close new sources; a viewer restart leaves host dashboard bindings available. See [dashboard architecture](../../docs/dashboard.md).

The host assembles bounded library reply fragments and returns raw JSON bytes to the WebView. Dashboard replies do not cross the native map pipe. Cancellation/admission ordering prevents a cancelled open from later creating a leaked binding. Map layouts replace snapshots and retain their bounded control queue. Preserve canonical retryable refusals from [transport.rs](../dashboard-core/src/transport.rs); never truncate requests naming many countries.

Preparation and prepared-page queries also run host-direct. Opening a recording
admits background preparation; the library and preparation controls query real
progress, start or cancel work, and Clear its stored data. The commands validate
the current source binding and revision. Progress counts and stored byte totals
are decimal strings, preserving values beyond JavaScript's exact-number range.

At startup, an idle monitor scans the shared library without opening a campaign
in the UI. It observes newly published recording revisions and admits unfinished
recordings in most-recently-modified order through the dashboard's existing queue.
The library scan retries busy admission and checks cancellation between entries
and trailer metadata reads. Game presence is polled independently every 250 ms,
so a slow scan or source open cannot delay observing a game start. While any
`hoi4.exe` process runs, automatic admission stops and automatic jobs yield at
their next block boundary. The idle monitor retries after game exit; manual Prepare
stays available. Library scans watch for changed recordings once per idle second.
Unchanged or busy replies stop admission retries until a recording revision,
worker completion, preference/content binding or game state changes. No additional
scheduler or persisted queue is used. Shutdown signals both monitor threads before closing the
shared dashboard service. Library admission uses the dashboard's shared 2 MiB
worker-thread constructor, so its source opening and preparation restoration have
the same stack budget as the production preparation workers and service loaders.

## Folders, settings and exports

The replay folder is the recorder's save folder in the shared `RecorderSettings` (by default `HOI4Replay\captures` under local app data, created when first listed), so the folder this app lists is the one Battle Plan Recorder saves into; choosing a replay folder here changes it there. Library listing runs off the UI thread, excludes hidden folders/symlinks and obeys its traversal budget. It reads validated trailers to identify partial recordings without decoding frame blocks. A manual recording pick does not change the replay folder.

When the app opens and the recorder's **With Battle Plan** setting is on (the default), it starts the sibling `battle-plan-recorder.exe` detached, with no console or inherited pipes and outside any job this app belongs to, so closing the app never stops a recording; the recorder's one-instance rule makes a second start a no-op. Recording display names and sidebar campaign flags live in the recorder's shared settings, so the recorder shows the same ones: `recording_preferences` reads them (flags by the frontend's campaign key), `set_recording_names` sets or clears only the names a change touches, by normalized path, and `set_campaign_flag` sets or clears one campaign's flag, each inside one `RecorderSettings::update`, so neither program's names or flags are lost to the other's save.

Native dialogs run off the UI thread, belong to the app window, identify their purpose and start in the current valid directory. Cancellation preserves the field. Game/user folder choices stay drafts until Apply, which `set_content_folders` checks against the installation before saving; replay-folder choices save immediately.

`video.rs` stores preferences in APPDATA/battle-plan/video.json. UI scale, map scale and VSync initially inherit HOI4; Revert restores inheritance. *Match game* is the inherited map scale. Apply webview zoom/native presentation with rollback on failure and send applied values to existing/new viewers. These settings do not resize windows/widgets or change fullscreen/monitor modes; dashboard Edit handles own widget size.

The viewer has no interface of its own: the host owns every control, setting and dialog, and map actions change the map's state and rendering. `viewer.exe` started outside the host runs only its headless tooling ([viewer guide](../viewer/README.md#use-and-export)). Map diagnostics default off; Settings > Diagnostics or F3 toggles/saves `mapDiagnostics`. Native F3 requests the host toggle; `--diagnostics` enables one run without saving. `MapDetails` is requested on demand.

The host saves map appearance preferences; after each save request, even one whose reply failed, it sends the running viewer `MapSettingsSaved`, once however many are queued, and the viewer reads them again and reloads any map they change. Export carries an optional map-view ID through the native action. Widget exports target that view; Campaign map exports omit the ID. Global map-settings requests need no source; campaign/widget targets retain source/revision validation. Appearance inheritance and content reload rules belong in the [viewer guide](../viewer/README.md#campaign-appearance).

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
