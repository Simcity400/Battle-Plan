# 1115. Mix battle ambience from ordered recording updates

Battle ambience aggregates combat fronts by province using BattleSounds row membership and, when recorded, the sub-units in each division. Animated model sound events belong to the presented frame. They are independent of the hourly battle mix. The battle fold marks changed combats, then fronts, then battles. A template change can invalidate a front without changing or rereading a combat's member list. Its type ids are interned before the counts are named. Missing inputs surface the same limitations as the direct reader, and content without battle sound definitions declares no inputs.

The game records the battle list by ordinal. The battle object identifies the same battle after an earlier entry ends and shifts its ordinal. Preserve a moved battle's existing mix unless a member province's front generation has advanced. A carried slot can otherwise miss the interval in which the battle has no slot tracking that front. Preserve recorded order in the final mix handed to audio.

The window hands its applied steps and changed declarations to a worker that maintains its own table copies and folds queued work in order. Ordinary playback takes available products without a wait and publishes the hour of the mix it actually holds. Compose and whole settle wait for the product of the applied hour. A reset discards results from an older epoch; a panic stops the battle layer. The tests compare the resulting mix against the direct fold throughout membership changes, ordinal shifts, and a recording gap.

[Battle ambience](../../crates/viewer-layers-infrastructure/src/sounds.rs) · [Worker](../../crates/viewer-layers-infrastructure/src/sounds/fed.rs) · [Tests](../../crates/viewer-layers-infrastructure/src/sounds/tests.rs)
