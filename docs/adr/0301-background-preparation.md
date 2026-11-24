# 0301. Read blocks ahead in order

Read blocks ahead to overlap decode and scan work without reordering what the callback receives. All concurrent scans share one core budget. A decode walk joins consecutive blocks in a segment and includes the campaign table. If the walk meets damage, preserve the error for the reached block and the segment’s lost remainder, then begin a new walk at the next segment. A scan that stops reading one block can move on without incorporating its remaining hours; reading that block afterwards, or after the scan has ended, is an error rather than a finished block. Cancellation or a finished scan ends the walk, even while the callback still holds a block. Charge decode timing to the scan that requested it.

[Block read-ahead](../../crates/dashboard-core/src/blocks.rs)
