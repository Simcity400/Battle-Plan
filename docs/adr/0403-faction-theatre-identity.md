# 0403. Keep native faction theatre identity and ordering for playback

The political map uses the first strategic region for the theatre's pin and averages all listed regions for its anchor. A faction member can see the pin unless its tag is hidden. The count is the theatre's stored country count, while the tooltip names the theatre, owner, commander and countries. Preserve native list order and signed region id bits. The commander reference stores its type before its id, and a zero pair represents absence. The name keeps its original bytes.

Authenticate the embedded manager against its faction before trusting the theatre array. Size all arenas from the original live headers before copying. The shared faction icon is acquired and counted once for a faction with theatres. Demand includes rows omitted for space so insufficient capacity cannot reduce the recorded requirement. A faction with zero theatres needs no icon copy.

Walk the theatre's native stride and keep hidden tags sorted by canonical slot. Inspect every header during sizing, and check list contents when copying that row. Test fixtures cover inline and heap names including unreadable heap terminators, an empty theatre list, an empty set of regions or countries and a split commander reference whose id word alone is zero.

[Faction theatre reader](../../crates/engine-land-inputs/src/cfaction_theater_manager.rs)
