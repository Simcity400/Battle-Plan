# collector-map-events

Map event capture: floating map text, the critical shore bombardment explosion and forced scripted-map refresh calls (`hooks`), the per-thread publication lanes they copy into (`lanes`) and the hourly interval that becomes the `MapEvents` table (`interval`). The host installs the hooks in the shared attach round, seals each hourly cut, assembles and commits the interval on the spool thread and grows the lanes on the sizing thread.

A forced refresh publishes only the selected scripted declaration ordinal, at the
native call after the mode-name and limit gates. Quiet hours poll no map-mode object.
The direct-CALL hook forwards both native arguments and stays in flight until the
original returns; an event read refusal never skips the game's refresh.

Each site sits where the game decides the event, before its local-viewpoint filter, so one recording can show what any country would have seen. The sites, their operands and why each is where it is are in the engine's [map_event_sites.rs](../engine-events/src/map_event_sites.rs).

- **Callbacks** copy their event in bounded synchronous steps through guarded reads: no allocation, lock, wait, game call or native pointer kept past the call; a refusal's reason is one atomic store. Entry and call-site callbacks count themselves in flight until their native original returns; detach waits for them, and one a native exception unwinds stays counted and keeps the collector loaded. A refused event (unreadable operands, no room) is counted and marks its family's interval incomplete; nothing is overwritten or retried.
- **Lanes**: each registered thread writes framed events into its own byte ring; threads that register after every ring is taken share the [shared lane](../collector-primitives/src/shared_lane.rs). Every published event takes the next number of one global sequence. The sizing thread installs a larger generation from recorded demand (refused bytes, threads beyond the rings, refused shared payloads); callbacks move to it and the older one drains until empty.
- **Interval**: the hourly cut reads two counters and each family's fault bits, so a quiet hour costs a few atomic operations. When every sequence number taken has been published, the interval is exactly the events numbered up to the last one taken; otherwise it is marked in flight and the late event is listed by a later interval. Each event carries the native world occurrence it happened in, read before its operands and checked again before it is published (an event read across a world change is refused and counted), and each cut the occurrence its world boundary admitted: attach, a world reset, a load or an unadmitted cut start a new epoch whose first interval is incomplete, and an event an ended occurrence published after its last cut is counted and listed by no interval. A lost interval marks the next one. The spool thread copies and decodes the interval and frees its space only after the frame is written.

Read the [shared rules](../../AGENTS.md) and the [collector guide](../collector/README.md).

Run `cargo test -p collector-map-events` and `cargo clippy -p collector-map-events --all-targets -- -D warnings` for its synthetic checks; `cargo test -p collector-map-events --release --lib hook_cost_ -- --ignored --nocapture --test-threads=1` times the callbacks.
