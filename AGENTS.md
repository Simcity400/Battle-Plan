# Working on Battle Plan

Read the [README](README.md) and the README of each component you change.

- Port native behaviour exactly with hoi4re (`hoi4re fn|asm|data|callers <addr>`) and cite the addresses beside the code. Never capture or compute more harshly than the game.
- Mods are first-class: resolve every content fact through the recorded content stack ([installed-content](crates/installed-content/README.md)). Hard-code nothing from vanilla. Support every form an installed mod (`<local-path>`) reaches; refuse anything else by name and count it.
- Size from live data (design for 1,500 countries, test 5,000); no caps we chose.
- Change format readers, writers and consumers together. A branch's format is provisional (main+1) and renumbered at merge; old recordings keep opening, refusing only changed tables.
- Tests must fail on the old behaviour. READMEs describe what exists now (no history or status).
- Ask with the question tool when a decision has no clear answer.
- Work in your own worktree and branch; merge commits only; never push to main. After merging, delete the branch, worktree and `.cargo-target`.
- Run cargo (and npm `dev`/`build:desktop`) through `bash tools/build-slot.sh "<task>" "<command>"`.
- One the game at a time, only while holding the game lock; close it when done; never stop another agent's game, recorder or build. Drive the game and apps with computer use.
- Recorder changes need the fresh-save oracle ([recorder guide](crates/recorder/README.md)); never waive a save difference.
- Installed content are read-only. Captures and scratch go under `captures\`, never in Git. Never delete `C:\sccache_cache`.
- Before sharing source, run `python tools/check-publication.py` ([publication guidance](docs/publication.md)).
