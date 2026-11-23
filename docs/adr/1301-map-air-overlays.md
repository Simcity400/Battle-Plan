# 1301. Derive air overlays from the selected wings and recorded positions

Assignment stripes, wing ranges and relocation arrows follow selection in the viewer, with their positions, eligibility and ordering drawn from the recording and content stack. Assignments choose the longest-range representative by resolved pool base before checking its eligibility, so an ineligible training wing can displace a shorter-range wing there. Without a resolved selected base, the mode returns to country stripes. The viewing country's diplomacy vectors decide the war gate and the draw order of enemy, allied and own colors.

Mission arrows retain their prepared geometry and vary hover tint independently. A segment wraps only when its controls are strictly more than half a map width apart. Relocation computes progress from floating-point seam-adjusted controls, projects the symbol back onto the fixed grid without clamping progress, and carries the adjusted destination between incoming registrations at the same base. A nonpositive route length reports zero progress.

Range preparation visits task-force members in recorded order and uses the first carrier on each. Grouping uses the game's fixed scratch capacity and combines equal origins and ranges. An empty wing selects the group's empty color. Hovered ranges replace selected ranges only if at least one group is admitted. The reached base list used for tinting is separate from the retained base references used for gold icon frames.

[Assignments](../../crates/viewer-map-modes/src/air_assignment.rs) · [Mission arrows](../../crates/viewer-aircraft/src/air_orders.rs) · [Ranges](../../crates/viewer-aircraft/src/air_ranges.rs) · [Relocations](../../crates/viewer-aircraft/src/air_transfers.rs)
