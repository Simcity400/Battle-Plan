//! Standard-grid presentation geometry; no hourly walker is provided.

/// Primary vtable.
pub const VTABLE_RVA: usize = 0x02b2_f458;
/// Secondary vtable.
pub const SECONDARY_VTABLE_RVA: usize = 0x02b2_f6e0;
/// Secondary-base displacement; authenticate its vtable before use.
pub const SECONDARY_SUBOBJECT: usize = 0x1f8;
/// GUI pointer used for geometry queries.
pub const GUI: usize = 0x140;
/// Materialized row-pointer array.
pub const CHILDREN: usize = 0x1a0;
/// Materialized child count, u32.
pub const CHILD_COUNT: usize = 0x1ac;
/// Content width, i32 pixels, the first extent component.
pub const CONTENT_WIDTH: usize = 0x168;
/// Content height, i32 pixels, following content width.
pub const CONTENT_HEIGHT: usize = 0x16c;
/// Position method, observed after return for geometry.
pub const POSITION_RVA: usize = 0x022c_a2e0;
/// Grid position entry sentinel bytes.
pub const POSITION_SENTINEL: [u8; 16] =
    *b"\x48\x83\xec\x28\x4c\x8b\x99\x40\x01\x00\x00\x44\x8b\xc2\x48\xc1";
/// Equipment-grid append helper.
pub const APPEND_RVA: usize = 0x002d_e850;
/// Equipment-grid finalization helper.
pub const FINALIZE_RVA: usize = 0x022b_2f10;
