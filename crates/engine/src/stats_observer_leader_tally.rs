//! Leader-tally entry in NCombatLog::CStatsObserver's array at +0x20.
//!
//! This module uses a descriptive name; the engine's exact C++ entry type is
//! unproved. The old reader returned only the first cached, last-nonnull entry
//! and explicitly did not establish current-leader freshness.

/// Observer leader-tally entry stride; old passive_modifier_carrier.rs:20,384 reads 0x0c bytes.
pub const STRIDE: usize = 0x0c;
/// Observer leader-tally handle, read u64 by old passive_modifier_carrier.rs:387; pointer identity is not implied.
pub const HANDLE: usize = 0;
/// Observer leader-tally count, read u32 by old passive_modifier_carrier.rs:388.
pub const TALLY: usize = 8;
