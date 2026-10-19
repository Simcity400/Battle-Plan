# recorder-derived

Offline derivation checks against recorded clocks, temperature passes and attack/power-balance/naval tree witnesses, using the recording's verified content stack. Differences and incomplete comparisons fail except for the diagnostic cross-hour movement comparison. That comparison reports each unchanged edge's equal, later-only, differing, stalled and refused progress steps without failing the command, because operands change inside the hourly update. The end-to-end command-dispatch fixture stays with recorder.

- [derived.rs](src/derived.rs)
- [derived](src/derived)

Shared development and content rules live in [AGENTS.md](../../AGENTS.md). Capture and fresh-save checks are described in the [recorder guide](../recorder/README.md).

`recorder derived <replay> --movement-witness` selects the diagnostic native-speed
comparison: every mover must have a same-boundary sample, and every playback
speed must equal it exactly. Missing, refused or differing samples fail, as does
an empty comparison. The default command also checks these samples when present
and retains the report-only cross-hour progress comparison. Hourly snapshots can
differ from an intervening progress step because attrition, supply, weather, stats and
combat writers run within that interval. The cross-hour counts are diagnostic;
the opt-in same-boundary witness is the strict, exact movement-speed gate.
