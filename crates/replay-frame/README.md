# replay-frame

Frame encoding and reconstruction, branch continuity and CPU measurement over
[replay-data](../replay-data/README.md). This library has no compressed container,
filesystem writer, playback derivation or content-reader dependency.

`frame` owns baselines, selected decoding, sample provenance, immutable update
columns and prepared checkpoints. `branches` owns branch tracking. `envelope`
owns the existing spool record framing and its size bound, shared by the frame
encoder and recording writer so an encoded frame keeps its original allocation.
`rows::with_demand_parts` is the frame-aware fuel-demand join; row definitions
and codecs belong to replay-data.

Container implementations consume the frame wire and immutable-column APIs;
they do not mutate reconstruction internals. Test-only counters and historical
fixtures are exposed through `test-support`, enabled from dev dependencies.
Run `cargo nextest run -p replay-frame`.
Shared rules live in [AGENTS.md](../../AGENTS.md).
