# Working on Battle Plan

Read the [README](README.md) and the README of each component you change.

- Match the game's behaviour exactly, and never capture or compute more harshly than the game. Describe behaviour in the project's own terms: source, comments and docs carry no executable addresses, decompiler names, or copied game code, shaders, scripts or localisation. Offsets, RVAs and hashes the code needs are constants named for their purpose.
- Mods are first-class: resolve every content fact through the recorded content stack ([installed-content](crates/installed-content/README.md)). Hard-code nothing from vanilla. Support every form an installed mod reaches; refuse anything else by name and count it.
- Nothing from the game is checked in. Game data is read from the user's installation at run time; tests use synthetic fixtures, and tests that need the real installation read it from `BATTLE_PLAN_GAME_DIR`.
- Size from live data (design for 1,500 countries, test 5,000); no caps we chose.
- Change format readers, writers and consumers together. Old recordings keep opening, refusing only changed tables.
- Tests must fail on the old behaviour. READMEs describe what exists now (no history or status).
- Recorder changes need the fresh-save oracle ([recorder guide](crates/recorder/README.md)); never waive a save difference.
- Installed content are read-only. Keep captures, recordings and scratch files out of Git.
- Sub-agents are not allowed to run any cargo commands unless specifically requested.
