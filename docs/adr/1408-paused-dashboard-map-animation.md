# 1408. Freeze ambient animation on paused dashboard map widgets

Status: Accepted · Date: 2026-11-27

## Context

When playback is paused in the game, ambient motion continues: water and rivers ripple, supply trains, convoys and aircraft move along their routes, unit and building models play idle/combat animations, and time-driven map shaders tick. Pull request #1177 let a paused, untouched map stop requesting frames when nothing visible moves, while keeping that ambient animation alive whenever animated surfaces, actors or arrivals are visible.

Dashboard pages embed map widgets beside tables, charts and timelines. Most widgets show a coast, river, border or rail corridor, so keeping ambient animation alive while paused keeps the desktop app drawing frames continuously on a paused dashboard page—around 25 frames per second on a weak machine—even when the user is simply reading numbers on the page.

## Decision

When playback is paused and a surface is a **dashboard map widget** (`Viewer.dashboard_map`), treat ambient animation as still:

1. **One scheduling decision, keyed on surface kind.**
   - `Viewer::paused_motion` classifies a paused non-export surface as live on full-map surfaces (`PausedMotion::Live`) and held on dashboard map widgets (`PausedMotion::Held`).
   - While `PausedMotion::Held`:
     - The surface holds its ambient presentation clock at the phase of the moment it paused, so water, rivers, supply trains, convoys, aircraft, animated models, particles, strike effects, animated cursors and clock-driven shaders stay at that exact frame across any redraws triggered by hover, selection, seeking, or an untimed picture export of that paused widget.
     - `Scene::track_paused_motion` disables paused ambient-motion tracking (`surface_motion` queries, time-reading shader motion checks, and animated-actor checks), `schedule_after_render` schedules no delayed arrival wakeup for trains or convoys, and the audio owner suppresses ambient repaint wakes.
   - When playback resumes (`playing == true`), the ambient clock resumes continuously from its held phase without skipping across the paused duration.

2. **Finite transitions and interactivity still wake immediately.**
   - A paused dashboard map widget still wakes at once on play, seek, camera input, hover/selection, visible data or settings changes, and every finite transition that must finish (loading, pipeline builds, snow/mud transitions, railway and country-name fades, day/night light blends, selected movement arrows, floating map text, and counter/placement settling).
   - Finite transitions inside `Scene` advance on the live presentation clock (`Request.transition_time`) rather than the held ambient clock, so they converge and return the widget to zero frames per second.

3. **Non-widget surfaces and live export paths stay unchanged.**
   - The full map page, pop-out map window, standalone viewer, timed and video exports (`capture.time == Some(_)`), full-map exports, and untimed exports of playing widgets are unaffected and keep advancing live exactly as in #1177.
   - An untimed picture export (`capture.time == None`) of a paused dashboard map widget exports the widget's held frame so the saved image matches what is on screen and exporting never jumps the paused widget or resets its settled scene history.
   - Recorded facts and numbers are unaffected.

4. **Debug visibility.**
   - When diagnostics are enabled on a paused dashboard map widget, `Viewer::diagnostics_ui` notes `Ambient animation: held while paused`.

## Alternatives rejected

- **Keep ambient animation running on paused dashboard widgets.** Costs continuous GPU and CPU work on an idle dashboard page.
- **Throttle paused dashboard widgets to a lower frame rate (e.g. 5 fps).** Still wakes the GPU and CPU indefinitely on an untouched page and adds a second scheduling path with timer state.
- **Freeze ambient animation on the full map too.** The full map page is meant to feel alive like the paused game when inspecting a campaign.

## Consequences

- A paused, untouched dashboard page draws zero map frames once loading and finite transitions settle.
- On a paused dashboard map widget, water, rivers, trains, convoys and animated models stand still as a deliberate visual departure from the game until the user plays again; the full map page keeps them moving.
