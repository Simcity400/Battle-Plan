//! Tree-instance source row consumed by FUN141265c10; exact C++ class name is unconfirmed.

/// Tree source-row stride used by s1_tree_caster.rs:170.
pub const STRIDE: usize = 0x10;
/// Tree source-row native instance count u16, read by s1_tree_caster.rs:192; old wire widened it to u32.
pub const INSTANCE_COUNT: usize = 8;
/// Tree source-row category and primary sort key u8, read by s1_tree_caster.rs:171.
pub const CATEGORY: usize = 0x0a;
/// Tree source-row state index u8, read by s1_tree_caster.rs:173.
pub const STATE_INDEX: usize = 0x0b;
