# 1301. Check viewer actor preparation against the eager walk

World rendering belongs in a crate that can operate independently of the host and sibling viewer features. Its tests use the allocator shipped by the viewer. Campaign fixtures keep one table allocation across frames so tests exercise fold-state retention as production does.

`BATTLEPLAN_VIEWER_CHECK_ACTORS` schedules each actor walk independently. The check runs the eager path with the same inputs, using a copy of the state that it discards afterwards. A numeric setting is a frame interval for long runs. A nonnumeric setting checks every frame. A frame record confirms the check ran and stderr identifies differences by element and field. Sharing the walk keeps the comparison focused on the lazy preparation path.

Actor motion measures the displayed result. It compares the root on the visible map copy and the current animation samples, with zero samples reserved for actors that play no animation. A frame record can therefore distinguish a publication-driven pose from motion on the viewer's clock. Recent root motion keeps an actor a mover between those updates, with one second of history at the 240 FPS target. Off-projection movement counts as movement even when its screen distance cannot be measured. The record reports projected distances in thousandths of a pixel, and forgets unseen actors to bound state over a campaign.

[Actor parity check](../../crates/viewer-scene-base/src/actor_check.rs) · [Motion counters](../../crates/viewer-scene-base/src/actor_motion.rs)
