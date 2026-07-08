# One-session capture runbook (map rounds + trees + UI sweep)

Goal: ONE live session that extracts everything the replay layer still needs:
per-moment runtime textures, the light-rig law vs camera height, the
GradientBorder zoom-bake law, LOD-by-height thresholds, the trees second
sub-draw, and a directed full-UI atlas. Requires harness jobs H1 (map rounds)
and H2 (UI hardening) merged; the DLL side is flag-gated so defaults match
the proven-live behavior.

## Preconditions

- VANILLA the game (no Kaiserreich — captured colors/regions must match shipped
  files). `--continuelastsave` loads the MOST RECENT save, whatever that is —
  so make sure the most recent save is a vanilla one (save a fresh vanilla
  1936 game right before the session if unsure). Clear weather over Europe
  preferred, game speed paused.
- Build: `cargo build --manifest-path replay_interface/native/Cargo.toml -p
  terrain_capture` (debug DLL, matches the launch recipe).
- Output dir: `C:\Users\Example\AppData\Local\hoi4_captures\multi_zoom_A`
  (non-OneDrive, per standing rule).

## Launch (user)

Standard attach recipe (handoff "Operating rules" item 7) with:

    --capture-moments terrain_active_pass,water_pass,rivers_pass,trees_pass,ui_pass
    --enable-map-frame-rounds                (H1: unlocks capture_map_frames_round)
    --trees-close-mode frame-repeat          (H1 deliverable 4 — LIVE TEST)
    --capture-out-dir C:\Users\Example\AppData\Local\hoi4_captures\multi_zoom_A
    --dwell-ms 900000                        (session stays open; rounds are manual)

## Phase 1 — map rounds at zoom stops (user + orchestrator)

The camera stays over the SAME location (suggest: center on Munich/Alps —
land + sea within view at wide zooms). For each stop the USER sets the zoom,
then the ORCHESTRATOR writes the round command to
`<out-dir>\terrain_capture_control.json`:

    {"type":"capture_map_frames_round","label":"h<height>"}

Zoom stops (camera heights; the game clamps 50..3000 per 00_graphics.lua):
close ~100, ~200, ~400, ~800 (matches existing captures), ~1600, max 3000.
Six rounds, ~1 min each. Watch `terrain_capture_events.jsonl` for the round
finish event before asking the user to change zoom. Each round lands in
`rounds/<label>/` fully self-contained.

What this pins down:
- rig law: CB0 c9-c12 (vVirtualSunPos family) vs camera height across rounds;
- GB zoom bake: per-round GradientBorderChannel payloads -> thickness/fill
  falloff vs height;
- LOD thresholds: which chunk scale the game draws per height;
- per-moment SnowMudData etc. -> byte-exact parity for every round.

## Phase 2 — trees second sub-draw (passive)

With `--trees-close-mode frame-repeat` on, the trees pass in each round
should record BOTH sub-draws (pixel shaders 277b7b5e... AND
6b77b6df9ba8f03d). Verify in events/bundle before ending the session; if the
close misbehaves (pass never closes / grows unbounded), remove the flag and
re-run one round — defaults are unchanged-proven-live.

## Phase 3 — directed UI sweep (user + orchestrator)

The user opens each screen; the orchestrator forces a labelled state capture
via the control file (H2 deliverable 3):

    {"type":"ui_capture_state_now","label":"<name>"}

Screen list (each ~10s): map_default, map_naval (naval mapmode — also lights
sea-zone borders for R6 ground truth), map_supply, production, construction,
research, trade, logistics, diplomacy_major, focus_tree, ledger_army,
army_panel_selected (select an army), division_designer, tooltip_state
(hover a state), tooltip_unit, main_menu_pause.

UI hazard note: `--ui-cb-skip hazard` is the default (browser-overlay draw's
constant buffers recorded Unobtained; D3DKMTOpenResource interop crash —
see the crash dossier). If the game still dies on a UI state, note WHICH
label, kill the game, relaunch, and continue from the next label — bundles
already written are durable.

## Phase 4 — full-game census play (user; requires job H3 merged)

With `--census` added to the launch flags, the user simply PLAYS the game
normally for as long as they like. The DLL captures every distinct rendering
pass the first time the game issues it — 3D map objects (buildings, cities,
ports, units via pdxmesh), battle-plan arrows and front lines, map text
labels, particles — with complete evidence, deduplicated payloads, and a
census manifest that also lists anything it could NOT capture and why.
Camera-height bucket changes trigger automatic map-frame rounds in the
background (one per bucket).

To maximize coverage in one playthrough, try to make these happen at least
once while playing (the census only captures what the game actually renders):
- zoom fully in over a big city and a port; watch buildings/urban sprawl
- construct something (civ factory + a railway/supply hub) and view it
- draw a front line + offensive arrows with an army selected
- start a land battle and view the battle bubble on the map
- move a fleet (convoy/ship models at sea) and view a naval battle if any
- a paradrop/naval-invasion order arrow if convenient
- open each map mode once (the census sees their distinct passes)
- let at least one in-game day pass at each of several zoom levels

Watch `terrain_capture_events.jsonl` heartbeats (every 30s: signatures seen/
captured, bytes, rounds). Budget: keep --max-total-payload-bytes-per-session
at 8 GiB; census stops itself first and the manifest records the tail.

Census-session flags & known limits (from the H3 fix round, fb7b25f1e):
- ADD `--max-capture-work-ms-per-frame 5000` to the attach flags for the
  census session. The default 1000 ms/frame work budget PERMANENTLY stops
  census payloads if a single frame's first-occurrence capture work
  exceeds it (a giant pass could trip it); the manifest records the stop
  explicitly if it happens — check `payload_stopped` in the heartbeats.
- Expect a small FPS cost while census is armed (~3.7 µs per draw call);
  repeats do no D3D work but still take a lock.
- Instanced draws (buildings/units) are wired and unit-tested but first
  exercised live in this session — if the heartbeat shows an instanced
  pass uncaptured with an unexpected reason, note it and keep playing;
  evidence of the failure is itself captured.

## Solo restore-pass gate (orchestrator; no user needed)

Goal: one `map_frame_restore_pass_bundle.json` carrying the composed
ColorCube LUT (1024x32 RGBA8), the PS slot-1 PostEffectBase CB (80 bytes:
HSV@0, ColorBalance@16, EmissiveBloomStrength@28, MiddleGrey@60,
LumWhite2@64) and the 1x1 AverageLuminanceTexture — the color-grading
ground truth for `replay_app --postfx-restore-full`.

- MUST be a fresh `terrain_capture_attach --exe` launch: the restorescene
  PS is identified at Create* time by its reflected SRV signature
  (MainScene t0 / RestoreBloom t1 / ColorCube t2 /
  AverageLuminanceTexture t3); attaching to an already-running game never
  sees the creation and the moment can never arm.
- Put `restore_pass` FIRST in `--capture-moments` (tiny payload, fires
  every frame once trusted; it completes seconds after the map loads,
  then the sequence advances to the heavy map moments).
- `restore_pass` is exempt from the per-frame work pause and is kept out
  of map-frame rounds by design. It requires the census trust latch
  (post map-renderer-live), so it cannot fire on the load screen —
  `restore_pass_seen_untrusted` events before the map are expected noise.
- Prerequisite in settings.txt: `draw_postfx=yes` for this session (the
  usual capture recipe disables postfx, which would remove the very pass
  being captured). Revert after.

## Aftercare (orchestrator)

- Validate every round with the bundle validator; run parity per round
  (each round is its own byte-exact reference).
- Fit the rig law + GB zoom law from round CB0s/payloads; file results in
  the handoff.
- Feed the naval-mapmode GB payloads to the R6/sea-border work as encoding
  ground truth.
