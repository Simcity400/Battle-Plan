# Battle Plan / Replay

Battle Plan passively records supported Hearts of Iron IV campaigns into portable
`.battleplan` files and plays them back as an interactive reconstruction of the
game.

The recorder automatically finds running games, captures changing simulation
state without pausing or controlling HOI4, preserves reload and relaunch branches,
and accounts for unreadable periods as explicit gaps instead of discarding the
recording. The viewer authenticates the recording and reconstructs the map and
observer UI from the exact game, DLC, mod, language, and settings content the
campaign used. Unsupported builds or incompatible content are refused rather than
guessed.
