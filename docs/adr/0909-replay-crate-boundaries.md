# Replay crate boundaries

`replay-recording` owns the columnar reader, the shared schema and the recording metadata codecs.
`replay-journal` builds on that schema and owns the journal, file writer and native row visitors.
`replay-capture-spool` owns spool recovery, spool writing and command analysis, using the metadata codecs from the reader.
`replay` builds campaign playback on those layers and enables live writing through its `recording` feature, with derivation writers supplied by its callers.
`replay-lanes` exposes reads over recorded columns and summaries to dashboard consumers.
`replay-settings` owns recorder settings and packing, `replay-files` owns Windows file management and recompression, and `replay-tools` owns the measurement binaries.
The crate boundaries leave the recording format and encoding paths intact.

## Rows and table operations

`replay-rows` owns the codecs and row traits. A domain value implements `OpaqueValue` to use the core's opaque encoding for its wrappers, including nested wrappers. Types with distinct native encodings keep their explicit implementations in the core. The core also implements codecs for values owned by `game-model`.

World, events, air, naval, wars, economy, intelligence and projects build on the core. Weather builds on world, land builds on world and weather, and presentation builds on land and world. Politics uses presentation and world. Derived rows use the event-feed scope type.

Table operations live in `replay-tables-<domain>`, or in the existing `ops` module for politics, intelligence and projects. World owns the modifier breakdown rows and their table operations. Their shared values and tree helpers stay in the core. `replay-data` re-exports the rows and operations and owns the closed `TableRows` enum. Letting its dependents build against a subset of tables would require a type-erased table store.

[Reader](../../crates/replay-recording/src/lib.rs) ? [Journal](../../crates/replay-journal/src/lib.rs) ? [Spool](../../crates/replay-capture-spool/src/lib.rs) ? [Campaign](../../crates/replay/src/campaign.rs)
