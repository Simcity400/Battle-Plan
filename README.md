# Battle Plan

Record your Hearts of Iron IV campaigns, then replay them in a Windows desktop app. Follow the war on an interactive map, rewind to any hour, compare countries and factions, and dig into the military, economic and political history in an editable statistics dashboard.

The app has two views:

- **Campaign map**: the game's map, redrawn from the recording, hour by hour.
- **Overview**: a dashboard of charts, tables, timelines and detail views.

A separate tray app, **Battle Plan Recorder**, records while you play. Watching a replay doesn't need the game running.

## How it works

1. **Attach.** The recorder finds the running game, checks it is the supported version, and loads `collector.dll` into it.
2. **Observe.** Every game hour, the collector reads the game's state. Hooks catch events between hours. It only reads: it never pauses, saves or controls the game.
3. **Write.** Readings are copied out of the game straight away, then encoded in the background into a recoverable spool file.
4. **Publish.** When recording stops, the spool becomes an indexed, compressed `.battleplan` file. A load, a new session or an unexplained jump in time starts a new branch instead of inventing continuous history.
5. **Replay.** The desktop app opens recordings, runs the dashboard and shares one playback clock with the map viewer. In the background it prepares histories and map checkpoints so playback and charts are fast. That prepared data is disposable and separate from the recording.

Anything the recording didn't capture is shown as missing, never as zero.

## Requirements

- Windows, with Microsoft Edge WebView2.
- Hearts of Iron IV, at the exact supported version. The recorder checks the executable's size and hash and refuses anything else.
- A graphics card for the map. Software rendering works, with a notice.

No installed content are included; the app reads them from your own installation.

The installer puts five files in one folder, with the licence files: `replay-desktop.exe` (the app, which draws the map itself), `battle-plan-recorder.exe` (the tray recorder), `collector.dll` (which the recorder loads into the game), and Microsoft's DirectX shader compiler, `dxcompiler.dll` and `dxil.dll`. The command-line recorder, `recorder.exe`, is built from source.

## Record a campaign

### With Battle Plan Recorder (tray app)

The replay app starts it automatically. You can also run `battle-plan-recorder.exe` on its own, or have it start with Windows.

1. Start the game. The recorder waits until exactly one game is running.
2. Click the tray icon. Press **Record**, or turn on automatic recording for this campaign or for every campaign.
3. Optionally pick where to save. The default is `%LOCALAPPDATA%\HOI4Replay\captures`, which is also the replay app's library.
4. Play normally. Closing the panel or the replay app doesn't stop recording.
5. Press **Stop**, then **Save** to publish the replay, or **Discard** to throw the session away. Quitting the game saves automatically.

Later sessions of the same campaign continue the same file. Choose **Next time it loads: New file** to start a fresh one.

### With the command line

```powershell
recorder.exe record --once --out <capture folder> --user-dir <Paradox user folder>
```

With `--once`, quitting the game finishes the recording. Without it, the recorder waits for the next game. Press Ctrl+C to stop, or run `recorder.exe detach --out <capture folder>` from another terminal. Use either the command line or the tray app, not both. Running `recorder.exe` with no arguments attaches to a game; it isn't a status check.

## Watch a replay

- Under **Campaigns** in the sidebar, press **+** to open a `.battleplan` file. **Archive** lists other recordings in your replay folder.
- In **Settings > Folders**, choose your game and Paradox user folders.
- Use the playback clock to play, pause, seek and switch branches. The map and the dashboard follow it. You can line up several recordings at once.
- Press **Edit** to add, move, resize or remove widgets, then **Save** the layout.
- **Preparation and storage** shows background preparation and lets you prepare, pause or clear it. Preparation pauses while the game is running. Clearing it never deletes a recording.

Partly damaged recordings still open: you get the readable part, with a notice about what's missing.

## Campaign map

- Watch from the recording player's point of view or any other country, with fog of war and day and night.
- See units, orders and routes, battle plans, land and naval battles, raids, buildings, resources and weather. Hover for game-style detail windows.
- Map modes include political, terrain, states, factions, players, ideology, resistance, compliance, manpower, infrastructure, diplomacy, operatives, resources, air, navy and raids, plus scripted modes from the recorded mods. Supply mode isn't available: recordings don't hold the data it needs.
- Use the normal camera or free **Fly** and **Walk** cameras (F12 switches), with controller support. **Pop out map** opens a second map window.
- Game sounds, ambience and music play, with volume controls.
- Export map images from the app, or from the command line:

```powershell
replay-desktop.exe --window-dump <campaign.battleplan> --hour <n> --branch <id> --out <frame.png> --camera <x,z,height>
```

The map steps hour by hour; it doesn't interpolate between hours.

## Overview dashboard

An editable dashboard of widgets: ranked races, line charts, exact tables, timelines, heatmaps, breakdowns, sunbursts and country-to-country flows. Tooltips and drill-downs show exact values and dates.

- **Countries and factions:** scoreboard and game score, army, air and navy strength, factories, territory, fuel, nukes, experience.
- **Economy:** production, equipment designs and stockpiles, losses, construction, trade, resources, lend-lease, market contracts, licences, naval repairs.
- **War:** wars and war goals, casualties, land and naval battles, ships sunk, air losses, bombing, raids, peace deals, territory changes.
- **Politics:** political power, stability and war support, parties and elections, national spirits and focuses, subjects, civil wars, factions, world tension, diplomacy.
- **Research and intelligence:** research, doctrines, special projects, intelligence agencies, operatives, decryption.
- **Units and leaders:** division templates and designers, equipment and training, unit histories, generals, admirals, air aces.

## Mods

- The recording remembers the exact base game, DLC and mod list it was made with, and playback uses that list, not whatever your launcher has selected. You need the same mods installed for full detail.
- You can pick visual mods for artwork, interface, fonts and models. They change how things look, never the recorded facts.
- Shared recordings hold no installed content or file paths, only content identities and hashes. A mismatch is reported, not ignored.
- Recordings from other people are treated as untrusted. Readers check every size and count, and damaged parts become named gaps.

## Diagnostics and recovery

- **Settings > Diagnostics** (or Ctrl+F3) shows map diagnostics.
- The recorder has offline tools:

```powershell
recorder.exe inspect <replay>                # identity, branches, hours, gaps, coverage
recorder.exe metrics <replay> <branch> <first hour> <last hour>   # recorded capture timings
recorder.exe sizes <replay> --verify --columns                    # size by table and column
recorder.exe derived <replay> --game-dir <game> --user-dir <user> # check playback calculations
```

- If a recording didn't finish cleanly, its session folder is kept. Once the game has exited, `recorder.exe finalize <session folder> --out <capture folder>` recovers the readable part.

## License

Battle Plan is free software under the [MIT License](LICENSE).

This software is based in part on the work of the Independent JPEG Group. It uses the Rust `image` library for several texture formats and includes a modified Rust adaptation of PhysicsFS in the archive reader. The refresh icon is from Lucide. Third-party notices are in [LICENSES](LICENSES/).

Battle Plan is an unofficial project and is not affiliated with or endorsed by Paradox Interactive. Hearts of Iron IV is a trademark of Paradox Interactive AB. The repository contains no installed content, assets, or original game code from the game. The app reads the content it needs from your own installation.
