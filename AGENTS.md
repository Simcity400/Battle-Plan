# Working on Battle Plan

Read the [README](README.md) and the README of each component you change.

- Match the game's behaviour exactly, and never capture or compute more harshly than the game. Describe behaviour in the project's own terms: source, comments and docs carry no executable addresses, decompiler names, or copied game code, shaders, scripts or localisation. Offsets, RVAs and hashes the code needs are constants named for their purpose; comments and messages name the field or constant, never where it sits in memory.
- Mods are first-class: resolve every content fact through the recorded content stack ([installed-content](crates/installed-content/README.md)). Hard-code nothing from vanilla. Support every form an installed mod reaches; refuse anything else by name and count it.
- Nothing from the game is checked in. Game data is read from the user's installation at run time; tests use synthetic fixtures, and tests that need the real installation read it from `BATTLE_PLAN_GAME_DIR`.
- Size from live data (design for 1,500 countries, test 5,000 in the full suite); no caps we chose.
- One build ([builds](docs/builds.md)), and no test is behind a feature: every test compiles in every build and all but `#[ignore]`d ones run; `#[ignore]` is only for benchmarks and tests that need hardware or inputs a run may not have. Run the tests for what you changed (`cargo nextest run -p <crate> [filter]`). Each test runs in its own process, so it sets up everything it reads, such as executable metadata, and never relies on another test having run first or on timing.
- Change format readers, writers and consumers together. The app isn't in production, so there's no backwards compatibility: don't add code, migrations or tests to keep old recordings, formats or saved settings working, and don't go out of your way to reject them either.
- Recorder changes need the fresh-save oracle ([recorder guide](crates/recorder/README.md)); never waive a save difference.
- Installed content are read-only. Keep captures, recordings and scratch files out of Git.
