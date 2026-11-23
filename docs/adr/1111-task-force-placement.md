# Keep task-force drawing and carrier launch evidence distinct

Task forces use the replay's recorded motion and the mod stack's movement map. A missing or incomplete motion census withholds placement. Class-1 task-force routes share division control and settling rules. Combat task forces advance their source state but use stand-off stack positions. A docked task force belongs to the naval-base pass, which admits only a task force with an empty ordinary route. An unplaceable route can leave the visible task force on its province with a reason. It cannot provide a province-centroid substitute for a carrier launch origin.

Use one route sample for admission, placement, and scale. Animation and selection are viewer state. A route with too few valid controls can lose its movement samples while still retaining the recorded ordinary route and destination.

The counter and ship-model passes share the mission skip decision. A skipped task force never reaches representative selection, so neither its counter nor its model appears. The decision depends on the recorded skip flag, active mission, region state, absence of combat, and the mission-specific region requirements. A strike force is inactive for this rule. Invasion support needs a bombardment region, and a patrol needs the current matching patrol region. Absence of required operands leaves the result unavailable.

[Task-force visual state](../../crates/viewer-unit-reads/src/task_force_visual.rs) · [Shared unit rules](../../crates/viewer-foundation/src/unit_rules.rs)
