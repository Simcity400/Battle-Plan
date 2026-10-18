# weather-period

Strategic-region weather periods, their default bounds and temperature ranges, native date setters and first-matching-period lookup. This leaf crate has no dependencies. Both the content reader in [installed-content-regions](../installed-content-regions/README.md) and the temperature simulation in [replay-data](../replay-data/README.md) use the same types and arithmetic.

Preserve wrapping integer arithmetic, truncating division, zero-based month/day inputs, inclusive bounds and authored period order. Defaults describe the game loader; authored values come from the recorded content stack. The simulation and recorded rows remain in replay-data, which re-exports these public functions and types through `replay_data::temperature`.

Synthetic date and period tests run with `cargo test -p weather-period`. Shared rules live in [AGENTS.md](../../AGENTS.md).
