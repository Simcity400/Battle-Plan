# 0906. Reuse table readers across hour queries

The recording can read each selected table's lane independently and assemble the result as a frame for the playhead. Requests select table, columns and row keys. A typed partial row keeps defaults in fields the query did not decode. Withheld or damaged tables retain their reasons and original sample hours. A table whose row type cannot represent a missing recorded cell is refused as a table. Other selected tables remain available. Unselected tables retain that status, and the resulting frame names its recording branch.

Successive hour requests reuse the last table state. Read identity includes the run, table and selected columns. Requested row keys filter a view after reading and need no independent reader. `Cursor` serves a fixed set of reads, and `Seeker` retains table readers for an evolving set. The reader advances from its current state if that is cheaper than restoring. Earlier hours or nearer restarts use the recovery state instead. A cursor can hand its readers to a subsequent request, subject to matching tables, columns and an unchanged run extent. A frame failure invalidates the exported set. A damaged table is opened afresh for a later move, and a failed seeker move discards its participating readers because they may be between hours.

Differential tests compare independent seeks to direct frames and reused cursors across run and block boundaries, restoring and reversing the hour order. Partial key and column reads must equal the relevant subset without creating missing keys or decoding a withheld opaque field. Decoder counters check that advancing across hours uses restarts only when needed.

[Hour reads](../../crates/replay-lanes/src/at.rs) · [Reader parity tests](../../crates/replay-lanes/src/at/tests.rs)
