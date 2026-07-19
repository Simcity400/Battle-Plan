# XUI23 — active command abilities

Status: **REPLAY INTEGRATION READY — EXACT A36 PRODUCER INPUTS EXTERNALIZED**

XUI23 closes every replay-owned command-ability seam without evaluating game
triggers or inventing timers, costs, targets, ordering, text, or disabled
reasons. The remaining input is the exact extractor-owned A36 carrier, not a
rendering approximation or an unsearched capture request.

## Proven authored law

The effective ability definition set comes from `common/abilities`, joined
through the same UI14/UI15/XUI24 VFS receipt used by the production UI catalog.
The vanilla 1.19.2 baseline contains 22 ability definitions in six files. Every
definition has a name key, description key, `army_leader` target type, cost and
duration; two define an explicit 504-hour cooldown and three are indefinite
cancelable abilities (`duration=-1`). Every resolved icon exists in
`interface/abilitylist.gfx`.

`abilitylist.gui` fixes the button container at `(37,11)`, size `36x80`, and
places the active layer, cooldown layer, and button at `(-8,-3)`. The window is
`165x80`, center-down, unclipped, with a 300 ms linear fade. The active and
cooldown backgrounds are separate vertical `33x33` progress bars using
`gfx/FX/progress.lua`. Static `allowed` blocks prove only authored trigger
presence; they cannot be promoted as a runtime availability result.

## Production replay law

`countries.<TAG>.active_abilities` is deliberately separate from the existing
save-side `abilities` object, which contains aces, leader reassignment/capture
cooldowns and officer-corps state—not named active command abilities.

Each exact row carries identity, availability, activation hour, total and
remaining cooldown, effective command-power cost, target kind/id, total and
remaining duration, source order, resolved icon, resolved tooltip and exact
disabled reason. A `complete=true` empty list is a proven-empty bar. All absent,
null, wrong-typed, incomplete, duplicate, out-of-order, timer-inconsistent,
unknown-definition and icon-mismatch states reject.

`replay_core::PoliticsDetailView` preserves every row leaf as
`Absent`/`Unobtained`/`Measured`. `game_ui::active_abilities` admits only complete
rows and performs no trigger, localization, cost or timer reconstruction. The
Python snapshot schema enforces the same types, identities, timer bounds,
completeness and ordering.

`load_active_ability_definitions` consumes the authenticated effective VFS,
strictly parses every effective `common/abilities` file, resolves icons against
the effective sprite registry, and rejects parse recoveries, unknown top-level
constructs, duplicate ability identities, invalid authored bounds and missing
sprites. Mod-added abilities therefore enter through the same production path;
there is no frozen vanilla-only runtime table.

## Exact extractor dependency

A36 is frozen separately in
`extractor/documents/dossiers/A36_ACTIVE_ABILITY_INPUTS.json` and listed once in
the extractor-only `RE_REMAINING.md`. It requires a same-tick selected-context
row set plus affected-division identities for the existing on-counter
`active_ability_kinds` seam. Static definitions, save commander cooldowns and
RenderDoc draws are explicitly forbidden substitutes.

The retained XN16P receipt binds 214 RDCs totaling 69,298,844,274 bytes. Those
captures remain useful for pixels and authored-layer corroboration, but a draw
stream cannot supply the complete identity/availability/target/timer/text row.
No additional capture is requested for the replay-owned implementation.

## Mechanical freeze

`build_xui23.py` deterministically rebuilds the static catalog and retained
source census. `verify_xui23.py` hashes every production source and dossier,
runs the focused Rust/Python gates, and writes the verification and manifest.
The verifier must run twice byte-identically before promotion.

## Incidental findings

- `extra_suplies` is the shipped script identifier (misspelling preserved),
  while its texture path is `ability_extra_supplies.dds`.
- The shipped GUI comment still names `basic_abilities.txt`, but the effective
  vanilla definition is `generic_leader_abilities.txt`; the comment is not used
  for resolution.
- Save-side `leader_modifier_enable_date` is a reassignment/capture gate and
  must not be displayed as an ability cooldown.
- The original static-catalog builder serialized eight shipped-source paths as
  machine-local `F:/...` paths. The frozen builder now records
  `root=installed_game` plus canonical game-relative paths; all eight file
  hashes and sizes validate without retaining a workstation-specific path.

## Ghidra provenance

| Function address | Job ID | Artifact path | Manifest hash |
|---|---|---|---|
| N/A | N/A | N/A | N/A |

No new binary claim is made. `PROGRAM_STATUS`, unrelated packets and adversarial
review surfaces were not edited.
