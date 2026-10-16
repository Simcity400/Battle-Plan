# recorder-metrics

Exact inclusive recording-interval metrics, recorded failure accounting and native-nanosecond percentiles. The command parser and JSON schema live in metrics.rs. Command routing is owned by recorder; synthetic fixtures are dev-dependencies.

- [metrics.rs](src/metrics.rs)

Shared development and content rules live in [AGENTS.md](../../AGENTS.md). Capture and fresh-save checks are described in the [recorder guide](../recorder/README.md).
