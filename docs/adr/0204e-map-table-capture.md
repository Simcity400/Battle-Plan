# 0204-E. Keep map table ownership and row order through projection

The host dispatches, loans and publishes the ordinary map table captures. The battle-sound list retains the native per-frame visit order and its member list. Worker buffers grow from the acquired list and member counts. A vacant placeholder needs no allocated storage while a worker owns those buffers. Naval-base and saved-target capture likewise grow from the bounded map, each live country list and the acquired name lengths. Owned naval-base decoding performs sorting and publication without a second read of the game.

The special-facility projection can see the same province in more than one program. Its flag is a single value and the first captured occurrence wins. The rows already occupy the owned vector in capture order, so using their positions as the sort tie-breaker avoids a second sorting allocation.

[Battle sounds](../../crates/collector-map-tables/src/battle_sounds.rs), [Map facilities](../../crates/collector-map-tables/src/map_facilities.rs), [Naval bases](../../crates/collector-map-tables/src/naval_bases.rs), [Saved event targets](../../crates/collector-map-tables/src/saved_event_targets.rs)
