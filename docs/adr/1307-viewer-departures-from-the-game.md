# 1307. Record where the map deliberately differs from the game

Status: Accepted · Date: 2026-11-21

## Context

The map aims to look like the game. Some differences are chosen on purpose: a recording lacks the input the game used, matching would cost frame time for little visible gain, or a stand-in gives a better picture than an honest blank. Without a record, agents read these as shortcuts and file issues to undo them. Recorded facts and the numbers the app reports still match the game exactly. This register covers only how the map looks.

## Decision

Each entry below is a deliberate difference. Change one only with a maintainer's agreement, and update its entry in the same change. A difference already decided in its own ADR, such as zoomed-out border and river simplification ([1302](1302-frame-workers-and-line-detail.md)), needn't be repeated here.

| Difference | What the game does | Why we differ | Revisit if |
|---|---|---|---|
| Bloom blurs at a quarter of the scene, below the seed size ([post.rs](../../crates/viewer-scene-base/src/post.rs), #1473, #1566) | Blurs at seed size | The two blur passes went from 0.59 to 0.18 ms per frame on an integrated GPU, part of #1473's 2.1 ms of approved cuts. The glow keeps its width, and the only noticeable change is a slightly softer halo | GPU time stops mattering here, or a comparison shows a visible artifact |
| Screen-space shadows and fog default to half resolution ([shadow.rs](../../crates/viewer-scene-base/src/shadow.rs), #1571) | Full resolution | Draws a quarter of the pixels to save fill cost. Edges are slightly softer | A full-resolution quality setting is wanted |
| A standing land figure faces a direction hashed from its province and owner ([unit_models.rs](../../crates/viewer-unit-models/src/unit_models.rs), #1575) | Uses a facing the recording doesn't contain | Neighbouring figures face different ways instead of all one way. Route headings still come from the recorded path | The collector can record standing facing cheaply |
| A root idle or defend animation with no phase of its own starts within a fixed five-second spread ([entity_animation.rs](../../crates/viewer-models/src/entity_animation.rs), #1570) | Each figure's phase follows when it entered the state, which isn't recorded | Neighbouring figures show different poses, as in the game. Matching needs new recorded data for a close-zoom detail | State-entry times become available |
| When a nuke and a critical shore bombardment hit one province in the same hour, the bombardment's explosion is shown, because it starts after the hour's nuclear impacts ([strike_effects.rs](../../crates/viewer-models/src/strike_effects.rs), #1569) | Shows whichever came last; one effect per province matches the game | The order within an hour isn't recorded, and this case is rare | Event order within an hour is recorded, or the nuke should take priority |
| Winter grading visits regions in map order ([winter.rs](../../crates/map-snow/src/winter.rs), [post.rs](../../crates/viewer-scene-base/src/post.rs), #1572) | Uses its own database order | No visible difference has been found. Matching needs game research | A map shows a grading difference |
| A train at a closed route end stays for the summed authored length of its cars ([trains.rs](../../crates/viewer-rail/src/trains.rs), #1568) | Uses the length measured from the train's mesh | Only the end of the train's lifetime differs, and clipping often hides it. A train kept active across a departure tick can hold the next one back under the fleet cap | A visible early or late disappearance, or a missing departure, is reported |
| When an hour's province visibility can't be rebuilt, the fog overlay is cleared and land stacks are shown in full; naval stacks keep their recorded detection while unit sight is available. Only the diagnostics' data limitations note it ([1111-map-texture-contracts.md](1111-map-texture-contracts.md)). A Map-mode recording, which never records fog, instead shows every stack and keeps its capture-mode notice | Not applicable | A clear map reads better than a broken-looking one, and the user doesn't need a message for a single hour. The capture-mode notice describes a recording mode the user chose | Users mistake this for real visibility |

## Alternatives rejected

- **Match every one exactly.** Several need data the recording doesn't hold, or cost frame time with no visible gain.
- **Show missing inputs as a blank or a warning.** That makes the map look broken for details the user isn't relying on as facts.

## Consequences

Issues asking to undo an entry are closed with a link here. A difference that is neither listed here nor decided in its own ADR is treated as a bug until a maintainer records it.
