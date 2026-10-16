# collector-ship-models

Ship model capture: each ship's ShipModels operands (fixed fields, pool entries, sprites, experience and medal modifiers), the held read that serves a ship until a hooked writer touches it, and the ship model writer observer. The host owns dispatch, loans and table assembly.

Read the [shared rules](../../AGENTS.md) and the [collector guide](../collector/README.md).

Run `cargo test -p collector-ship-models` and `cargo clippy -p collector-ship-models --all-targets -- -D warnings` for its synthetic checks.
