# recorder-derived

Offline derivation checks against recorded clocks, temperature passes and attack/power-balance/naval tree witnesses, using the recording's verified content stack. Differences and incomplete comparisons fail. The end-to-end command-dispatch fixture stays with recorder.

- [derived.rs](src/derived.rs)
- [derived](src/derived)

Shared development and content rules live in [AGENTS.md](../../AGENTS.md). Capture and fresh-save checks are described in the [recorder guide](../recorder/README.md).
