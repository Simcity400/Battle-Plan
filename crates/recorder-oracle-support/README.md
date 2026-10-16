# recorder-oracle-support

Shared save-comparison inputs, exact token/string/country joins and coverage accounting. The equality helpers distinguish comparisons that count each scalar from comparisons that only report differences. Convoy arithmetic and authenticated country-emission projection share the same owner.

- [support.rs](src/support.rs): exact joins and projections.
- [report.rs](src/report.rs): comparison attempts, differences, carried reads, limitations and verdicts.
- [fixtures.rs](src/fixtures.rs): shared synthetic fixtures, available only with `test-support`.

The command/comparison order belongs to [recorder-oracle](../recorder-oracle/README.md). Content resolves through the recorded stack. Shared rules live in [AGENTS.md](../../AGENTS.md); required live validation is in the [recorder guide](../recorder/README.md).
