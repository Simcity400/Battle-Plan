//! Observed hash-map header shared by the operation-token database and manager.
//!
//! The old source does not recover a C++ container type name. This file describes
//! that observed header only. The scan covers mask + overflow + 1 slots, including
//! unoccupied entries; this is not a live-row count.

/// Operation-token map header bytes inspected; old diplomacy_intel_operatives_live/src/extractor.rs:10686,10709 reads 0x20 bytes without proving the full object size.
pub const HEADER_READ_SIZE: usize = 0x20;
/// Operation-token map slot-array pointer, u64; old diplomacy_intel_operatives_live/src/extractor.rs:10689 decodes bytes +8..+16.
pub const TABLE: usize = 0x08;
/// Operation-token map bucket mask, u32; old diplomacy_intel_operatives_live/src/extractor.rs:10690 includes it in the allocated-slot scan bound.
pub const MASK: usize = 0x14;
/// Operation-token map overflow extent, u8; old diplomacy_intel_operatives_live/src/extractor.rs:10691-10692 computes mask + this value + 1 slots.
pub const OVERFLOW: usize = 0x18;
