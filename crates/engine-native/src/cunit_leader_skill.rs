//! CUnitLeaderSkill definition, shared by army, navy and operative leaders.

use engine_sites::Rva;
/// CUnitLeaderSkill vtable.
pub const VTABLE_RVA: Rva = Rva(0x029c_2910);
/// CUnitLeaderSkill allocation extent.
pub const SIZE: usize = 0x1c8;
/// Valid byte; true for real definitions and false for the null object.
pub const VALID: usize = 0x08;
/// CUnitLeaderSkill level i32.
pub const LEVEL: usize = 0x1b8;
/// Required whole experience, i32; scaled to fixed5 for comparison with leader experience.
pub const REQUIRED_EXPERIENCE: usize = 0x1bc;
/// Category, i32; four categories with a separate default value.
pub const CATEGORY: usize = 0x1c0;
/// CUnitLeaderSkill construction/parser.
pub const CONSTRUCT_RVA: Rva = Rva(0x00b3_fff0);
