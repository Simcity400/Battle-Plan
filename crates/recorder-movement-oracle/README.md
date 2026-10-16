# recorder-movement-oracle

Read-only native movement census and comparison against the verified content stack's MovementMap. Preserves province/neighbour order and terrain checks; unreadable data and differences fail. Its command opens a game only when explicitly invoked; tests use synthetic maps.

- [movement_oracle.rs](src/movement_oracle.rs)

Shared development and content rules live in [AGENTS.md](../../AGENTS.md). Capture and fresh-save checks are described in the [recorder guide](../recorder/README.md).
