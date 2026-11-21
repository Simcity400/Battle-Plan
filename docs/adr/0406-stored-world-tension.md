# Preserve stored world tension and its contribution operands separately

The world's ordered contribution vector, dense per-country cache and resident tension scalar are independent recorded inputs. Rebuild may remove exhausted sources, sum current values by canonical country slot, then clamp and divide the total for storage. Capture copies each input as it is stored. The dense cache includes slot zero and zero totals. Each source keeps both raw country references, stored hour, literal reason and three signed fixed-point values. The second country's role is unverified. Zero and negative source values and empty reasons are valid. Vector position supplies occurrence order within an observation and can change after compaction.

An unread slot inside the counted vector withholds the remaining rows. String sizing preserves the original header and terminator observation, including a reason too long for the caller's remaining arena. Count that row and its reason demand without publishing a partial value. A source that could not be read supplies no reason length. Sizing has no bearing on availability.

[Manager and dense cache](../../crates/engine-stability/src/cworld_threat.rs) · [Contribution capture](../../crates/engine-stability/src/cthreat_source.rs)
