# Preserve native power-balance identity and selection

The collector takes one census of the manager under the caller's consistency boundary. The reader preserves native record order and keeps member, range and override order within each record. Row capacity omits whole rows. The census reports exact row and byte demand, while a failed dependency withholds the census. Required text includes the bytes belonging to omitted rows.

These identities are byte strings. The caller-owned arena stores them without UTF-8 conversion or a fixed token-length limit. A recorded member retains its original country reference after canonical authentication. The record ordinal is the position in the manager. It carries no persistent identity across snapshots.

The stored trending side is independent of the sign of the balance. The selected range is the one the manager holds, which resolves overlapping bounds. Sprite overrides retain duplicates and empty values in order. Lookup takes the first match and falls back to the side sprite when its value is empty. The embedded balance is a signed fixed-point value. Members retain the raw daily and weekly accumulator values separately from the flags that control their presentation. The census reloads the flags on every call because the definitions can change on load.

[Power-balance reader](../../crates/engine-power-balance/src/power_balance_manager.rs) · [Recorded operands](../../crates/engine-power-balance/src/power_balance_record.rs)
