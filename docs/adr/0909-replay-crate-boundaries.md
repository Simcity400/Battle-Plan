# Replay crate boundaries

`replay-recording` owns the columnar reader, the shared schema and the recording metadata codecs.
`replay-journal` builds on that schema and owns the journal, file writer and native row visitors.
`replay-capture-spool` owns spool recovery, spool writing and command analysis, using the metadata codecs from the reader.
`replay` builds campaign playback on those layers and enables live writing through its `recording` feature, with derivation writers supplied by its callers.
`replay-lanes` exposes reads over recorded columns and summaries to dashboard consumers.
`replay-settings` owns recorder settings and packing, `replay-files` owns Windows file management and recompression, and `replay-tools` owns the measurement binaries.
The crate boundaries leave the recording format and encoding paths intact.

[Reader](../../crates/replay-recording/src/lib.rs) ? [Journal](../../crates/replay-journal/src/lib.rs) ? [Spool](../../crates/replay-capture-spool/src/lib.rs) ? [Campaign](../../crates/replay/src/campaign.rs)
