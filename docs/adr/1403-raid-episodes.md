# 1403. A raid episode is a run of one key's observations that never contradict

A raid's row key can be reused by a later raid, a load can carry a raid into a new stretch, and hours between reads can be unread. The raid log decides where each raid begins and ends by one rule, not by the key's lifetime.

An episode is a maximal run of observations of one key in which the identity evidence (raider slot, target, raid type) never contradicts and the phase never goes backwards. An observation is a stretch of hours over which the raid's row and its target row hold; a change of either starts the next one. A complete read without the key, a contradiction of identity, or a phase regression splits, whether or not the hours between were read. Within a stretch, unknown evidence is no contradiction. Across stretches (a load, a break or another run) two observations join only when raider, target and type are all known at the join hour and equal; evidence that becomes readable later never decides a join. A type is its spelling read at the observation's own hour.

The phase order is the game's phase number. An unknown outcome may become known; a known outcome never changes within an episode.

Every fact carries the hour it was witnessed and belongs to the episode holding that hour: an end, a success, a controller, a completion country. A deletion logged at an hour belongs to the episode holding the hour before it. An end-log row that contradicts the episode it falls in is not its fact. The victim is the target's controller at each observation until the first ended one. A completion country read later is a victim change at the hour it was read. A raid disappears at the next complete read only where its key was not read again; a key read again under another episode proves no absence.

Every decision uses evidence up to its own hour, so a reply through any hour gives the same episodes, ids, victims and success hours as the full history cut there; a reply from an hour clips that history. Artwork, icon warnings and the availability answer come from the episodes and lifetimes up to the requested hour. A tally takes a chosen slot first and otherwise the tag at the event: the slot's tag at launch, and at the actual end for a success, with no fallback to the raid's opening tag.

[Raid log](../../crates/dashboard-intelligence/src/products/raids.rs) · [Raid tally](../../ui/src/dashboard/raids.ts)
