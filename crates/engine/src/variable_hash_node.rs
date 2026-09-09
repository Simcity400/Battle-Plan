//! Candidate hash-node field shape used by the old CVariables reader.
//!
//! The C++ node type and exact first-node address are unconfirmed. The old
//! country_variables_extractor::scan_variables_snapshot tries six eight-byte
//! phases over (bucket_count + 4) strides and accepts only a unique key/value
//! set matching the declared count. This is retained layout knowledge, not a
//! proven occupancy walk. The new walker must establish actual node boundaries
//! and live-entry markers instead of adopting that phase search.

/// CVariables candidate node stride; old country_variables_extractor::scan_variables_snapshot advances by 0x30 at each candidate phase.
pub const STRIDE: usize = 0x30;
/// CVariables candidate node key MSVC string; old read_msvc_string_from_node_snapshot treats the candidate address as the string object.
pub const KEY: usize = 0;
/// CVariables candidate node native i64 value; old scan_variables_snapshot reads eight bytes at candidate+0x20.
pub const VALUE: usize = 0x20;
/// CVariables value divisor; old country_variables_extractor::VALUE_SCALE_RAW records native signed fixed-point units of 1/100000.
pub const VALUE_SCALE: i64 = 100_000;
