# recorder-app

Battle Plan Recorder (`battle-plan-recorder.exe`): a tray program that records campaigns while HOI4 runs. It drives the [recorder](../recorder/README.md)'s capture in-process through `recorder::controller`; there is no helper process or control channel. Closing its panel never stops a recording, and the replay app is never needed to record.

## Where to work

| File | Responsibility |
|---|---|
| [main.rs](src/main.rs) | Single instance, Tauri setup, commands and keeping the app alive with no window. |
| [worker.rs](src/worker.rs) | The record loop, the published snapshot, Stop/Discard/Record/Quit and compaction after HOI4 closes. |
| [targets.rs](src/targets.rs) | Each campaign's recording folder and New file. |
| [snapshot.rs](src/snapshot.rs) | The snapshot the panel and tray show, and the reading of collector status into it. |
| [tray.rs](src/tray.rs) | The runtime-drawn icon, tooltip and right-click menu. |
| [panel.rs](src/panel.rs), [commands.rs](src/commands.rs) | The panel window and the commands it invokes. |
| [compaction.rs](src/compaction.rs) | Automatic compaction of finished recordings. |
| [flags.rs](src/flags.rs) | The viewed country's flag through the recorded content stack. |
| [startup.rs](src/startup.rs) | One instance per user session, Start with Windows and the taskbar's light/dark mode. |

The panel itself is the `recorder.html` entry of the [frontend](../../ui/README.md); settings live in the shared [RecorderSettings](../replay/README.md#finished-recording-compression) file both programs read. Shared development, validation and storage rules live in [AGENTS.md](../../AGENTS.md); capture behaviour and its checks in the [recorder guide](../recorder/README.md).

## Lifecycle

A named mutex (`Local\HOI4Replay.Recorder`) allows one recorder per user session; a second start exits at once. The recorder starts from its own executable, with Windows when **Start: With Windows** is on (a value in the user's `Run` key, written or removed with the setting and refreshed at each start), and when Battle Plan opens with **With Battle Plan** on. No window exists at rest: the tray icon and one worker thread do.

The worker looks for one supported game about once a second, woken early by commands and setting changes:

- **Waiting**: HOI4 isn't running. More than one game is a problem; none is recorded.
- A read-only probe identifies the loaded campaign while not recording: after the same build check capture uses, it opens the game without write or thread rights and reads the engine's clock (campaign id, hour and the load signal, the session start stamp that changes with each load of a save) twice, accepting only an agreeing campaign UUID and load signal. Nothing it reads is recorded.
- The recorder attaches when the loaded campaign records automatically (its own choice, else **Record every campaign**), or at the game's menu when **Record every campaign** is on, or when **Record** is pressed. Otherwise it shows **Not recording** with the campaign and its date.
- After Stop (Save or Discard, even while a refused load was already ending the session), or a capture that failed, the same game process (identified by its start time, not its PID) is not attached again until it exits or Record is pressed.
- HOI4 closing saves automatically. A session that reached no spool, such as a game closed at its menu, ends quietly.

## Stop, Discard and Quit

Stop opens a one-row prompt asking whether to keep this session: **Save** finalizes as the command line does; **Discard** detaches, then (after collector absence or game exit is proved) finalizes nothing, keeps only the session's `completed.json` (marked `discarded`) and removes its spool, copied collector and evidence. A recording the session would have continued keeps its earlier contents. Pressing Stop again, Esc or clicking away cancels. The right-click menu's **Stop recording** (enabled only while recording) and **Quit** while recording open the panel on the same prompt; Quit exits once Save or Discard completes, and cancelling the prompt cancels Quit. Quit while saving exits when saving ends; Quit while compacting cancels compaction, leaving that recording as recorded.

## Campaigns and files

The save folder (**Save to**; by default `HOI4Replay\captures` under local app data) is the one recordings folder Battle Plan lists too: choosing a replay folder there changes it here, and the reverse. Each session records into a target folder: the save folder, or the folder a campaign's last **New file** started. The target is the collector's `replay_dir` (prior history and the finalization lock) and the finalization destination; session folders and `active.json` stay in the save folder. **Next time it loads: New file** is used at the campaign's next load or attachment: when the campaign already has a recording (one in its current folder, or hours of an earlier load in the running session's spool, which become that recording when the session saves), a new folder `<campaign id> (n)` beside the earlier recording becomes its folder and later sessions continue there; either way the choice returns to Continue. File names never change.

A session publishes only the loads it admitted. A load is a campaign and the game's load signal, the same identity the collector writes before every spooled frame. The load the worker attaches for is admitted before the collector starts. While attached, capture asks every two seconds about each load the probe reads, usually while it is still paused. When the session saves (Save, or HOI4 closing), once the game is no longer captured, capture reads the spool's clock witnesses once and asks about every load in it that was never asked about, in the order they were loaded across campaigns (by clock witness time), so a load that records and ends within one poll is not lost. The collector's status naming a campaign is never asked about, so it never uses up a Continue or New file choice. Each load is asked once. The campaign must record automatically, or be the one Record was pressed for (the campaign loaded when it was pressed, else the first one loaded: the campaign whose hours reached the spool first, by clock witness time, so one that recorded and ended before any probe still counts, or failing that the first one the probe names); either way it must belong to this session's folder, a pending New file included. Each decision is written to the session's `admitted.json` the moment it is made: admitted loads with the folder the session publishes into, and refused loads with why. A refusal while recording ends the session and the worker looks again at once, either skipping the campaign or attaching for its own folder; Record pressed for a campaign that belongs in another folder carries over to the session there.

Finalization, by this session or by any later `finalize`, publishes each campaign's branches whose load is admitted and nothing else. A load that failed in the collector before its first clock witness takes its load signal from the failure witness, so an admitted one keeps its gap and partial notice. Hours of a load that was refused, or not yet admitted when the recorder crashed, stay in the session folder, unpublished. So a reload after New file, even one unpaused before it is asked about, never reaches the earlier recording; the next session records it into its own folder from when it attaches. A session whose admission record is missing or unreadable publishes nothing. If the probe cannot then see a loaded campaign, the game is held rather than attached at its menu again. Choosing Save or Discard while a refusal is already ending the session holds the game like any Stop.

Loads a session recorded but left out are reported, never presented as nothing recorded. Each one (campaign, load signal, hours, and why: `off`, `elsewhere`, `settings` or `not_admitted`) is listed under `omitted` in `completed.json`. When any of them recorded hours, the panel replaces its usual status text with one line until it has been shown, for example **Saved · 1 load not recorded (Record automatically was off)**. If nothing was saved, it shows the same line in place of Not recording. Loads that belong in another recording (a new file, or the campaign's own folder) say "it belongs in another file"; loads decided while the recorder settings couldn't be read or updated say "the recorder's settings couldn't be read" (whether the campaign records and its folder are then unknown, so the load is never guessed into a recording); loads whose load signal was never identified say "it was not identified"; mixed reasons give only the count. Loads left out before they recorded an hour are not mentioned.

## Panel and tray

Left-clicking the tray icon creates the 360 px panel above the icon within the monitor's work area (or closes it); it is undecorated with the native shadow, always on top and out of the taskbar, and is destroyed when it loses focus (native folder dialogs excepted) or on Esc. A click arriving as the panel closes does not reopen it. Tray and menu callbacks run on the event loop, where creating a webview window would wait on that same loop, so the panel is created on its own thread; clicks while it is being created open no second panel. The panel uses the replay app's WebView data folder (`local.hoi4.replay` under local app data), so it opens with the user's appearance; should that WebView refuse the panel's options, the panel opens with its own.

The worker publishes a snapshot (phase, campaign, recording path, whether that file held an earlier recording, hour, size, progress, session start, problem, flag) as the `recorder-status` event and through `recorder_status`. Recording size is the earlier recording plus this session's spool bytes; the saved line reads the file's hour span from its trailer and its size. Saving progress counts actual finalization work against about four steps per recorded hour and stops short of complete until saving ends. Saved and Session discarded stay until the panel has shown them; a capture failure stays until shown or the next capture.

The flag is the recording client's viewed country as the game draws it. The collector publishes that country's flag operands in `status.json` (`player`, from committed rows; see the [collector](../collector/README.md#how-it-works)); `installed_content::country_names::flag_keys` turns them into the game's key order (cosmetic tag with and without the ruling group, then the original-or-own tag likewise). A background thread resolves the session's recorded content stack once (the identity in `bin/collector.json`, the game's folder and the session's Paradox user folder, mods first-class) and takes the first key whose `gfx/flags/<key>.tga` exists and passes the game's checked decoder at the content's large flag cell; a missing or refused file falls through, and a key that would leave the flag folder ends the search. The image, bounded to 128 pixels like the replay app's, goes to the panel's flag canvas. No flag is drawn when none resolves, and the panel title stays the campaign's display name. A campaign shown while not recording, saving, compacting or saved keeps the last flag this recorder resolved for it.

The 32-pixel icon is drawn at run time: a light ring (dark on a light taskbar) around a dot that is grey at rest, red while recording, amber for a problem and blue while saving or compacting. The tooltip names the state or problem. Problems use plain words; Details opens the session folder with the collector's status and completion records.

## Compaction

With **File size: Small**, recordings written or appended while a game ran are compacted after that game closes, one at a time in Windows thread background mode, through `replay::container::compact_with`: streamed, verified frame by frame and published only when strictly smaller. A game starting (checked every quarter second) or Quit cancels the current file, leaving it as recorded; the rest wait for the next close. Manual Compact stays in Battle Plan.

## Build and run

`npm.cmd run build:desktop` builds this app with the embedded frontend (`--features custom-protocol`) beside the replay app, viewer and `collector.dll`; keep them together, since the recorder injects the sibling `collector.dll` and **Open Battle Plan** starts the sibling `replay-desktop.exe` (with the saved recording's path for **Open**). This produces an executable directory, not an installer.

For development, run `npm.cmd run dev:ui` (or `npm.cmd run dev`), build `collector` and run `cargo run -p recorder-app`: a debug build loads the panel from the Vite server.
