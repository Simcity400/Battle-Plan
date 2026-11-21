# Match naval modifier trees by source identity

The recording identifies a naval modifier source by owner kind, owner identity, and a sub-key within that owner. Country sources use a raw country reference, while static fleet pride uses zero. Ship experience and medals use a ship handle. Critical effects also use a ship handle, with the critical index as sub-key. Task-force doctrine uses the doctrine map key, and terrain uses the ship class. Regional weather, province night effects, and the task-force modifier use the task-force handle. Commander sources use the leader handle, with a subunit key for class modifiers. Each support lane uses the influence's character reference. Side terrain and weather use the combat handle.

Source order stays the same as the reader's own visit. The opposing commander source uses that commander instead of the own commander. No primary country means there is no side source, and a commander source needs the commander it reads. A source without a retained tree contributes no identity. Carried trees rely on coverage of every writer and a valid receiver certificate.

[Naval modifier source model](../../crates/game-model/src/naval_modifier_trees.rs)
