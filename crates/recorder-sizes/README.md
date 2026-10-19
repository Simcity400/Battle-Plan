# recorder-sizes

Offline recording size attribution: block sections, columns, compression levels and round-trip hour verification. Uses the recording format codecs directly. Command routing is owned by recorder; no live game is opened.

- [sizes.rs](src/sizes.rs)

Shared development and content rules live in [AGENTS.md](../../AGENTS.md). Capture and fresh-save checks are described in the [recorder guide](../recorder/README.md).

The size report's `refusals` array lists each non-retired unavailable table by table, branch and reason, with first/last hour and count. Text output prints the same refusals, including spool validation details and failed hook families. Rewriting and `--verify` preserve these table refusals.
