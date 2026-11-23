# Reuse the encoder’s baselines without changing spool bytes

The spool compares only complete hourly tables with a baseline for the exact branch and hour. A keyframe, restart, gap, mistyped table, or missing baseline leaves validation and encoding to the ordinary path. Clock and MapEvents are excluded because their rows describe the enclosing hour. Other cadences keep their existing carried or sliced inputs. The comparison order prioritizes expensive tables so helper work finishes sooner.

Daily slot-12 training progress uses the ordinary carried path according to the recorded cadence decision. [Cadence audit](../../crates/replay-data/cadence-audit.md)

An equal table retains the baseline. A sparse change can release unchanged owned rows early only when membership and key positions match and the baseline permits row updates. Membership changes, unavailable storage, and refused row updates keep the complete input. Dense changes keep the full path for 24 hours because replacing most rows costs more than normal encoding. The compared forms carry the encoder’s baseline token, and the encoder verifies it again. Equality in encoded fields produces the same spool bytes even if cached rows differ in fields the codec never writes. Temporary comparison slots use an allocation-free empty reason and are restored before encoding.

The spool and its persistent helpers encode large table segments against shared baselines and assemble them in table order. Segmentation adds an allocation and byte copy, so small tables encode in place. If one segment fails, in-place encoding reports its error. Synthetic campaigns lower the threshold to exercise the same assembly path with small inputs.

File writes have a separate ordered owner and keep capture and encoder storage on the spool side. Pending receipts may occupy up to half the admission budget so writes can complete and release those receipts. Idle sync failures create a completion without a frame. Allocation-failure tests can hand the writer one turn at a time to keep allocator call order reproducible.

[Comparison](../../crates/collector-spool/src/spool_compare.rs) · [Segmentation](../../crates/collector-spool/src/spool_segments.rs) · [File writer](../../crates/collector-spool/src/spool_writer.rs)
