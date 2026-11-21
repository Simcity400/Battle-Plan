# Verify content identity before recording facts

The recorder packs game facts beside capture at low priority on half the available cores. Dropping the packing job cancels it and waits for it to stop. A recorder taking over a session preserves facts already packed for its content identity. For unpacked facts it verifies the current files against the recorded layer identities and names any withheld layer. Cached compressed chunks can be reused across packs.

The standalone facts attachment tool keeps readable fact packs already carried by a recording. It can add missing facts as an in-place revision only for layers proven by available installations. A pack proving nothing would prevent a later successful attachment, so that case leaves the recording alone. A folder naming lock excludes concurrent publication. The tool also refuses hours awaiting a save decision because discarding them would remove any later revision.

Scripted map demand is resolved before attachment, outside hourly capture. The resolver follows authored properties and localisation substitutions, including parameterized trigger keys and bracket properties assembled from multiple keys. A failed substitution still leaves authored alternatives to explore. A visited set prevents cycles from repeating work. An unused key alone does not demand a state read.

[Game fact packing](../../crates/recorder/src/recording/game_facts.rs) · [Facts attachment](../../crates/recording-upgrade/src/lib.rs) · [Scripted map demand](../../crates/recorder/src/recording/scripted_map_demand.rs)
