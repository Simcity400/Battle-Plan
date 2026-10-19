# collector-primitives

Shared typed capture rows and arenas, observed country sizing, observer publication lanes, address-table hashing, raw-memory capacity accounting and session clock reads. Buffer growth remains off the game thread and keeps exact observed demand and refusal outcomes.

Read the [shared rules](../../AGENTS.md) and the [collector guide](../collector/README.md).

Run `cargo test -p collector-primitives` and `cargo clippy -p collector-primitives --all-targets -- -D warnings` for its synthetic checks.

SharedLane exposes drained ownership so an event family can replace its empty storage off-thread while excluding producers.
