//! Image description populated by DDS loading, distinct from
//! `opaque_texture_description_layout`.

/// Image initializer setting zero flags and one initial mip.
pub const INITIALIZE_RVA: usize = 0x009d_a100;
/// DDS loader filling format and actual loaded mip count.
pub const LOAD_DDS_RVA: usize = 0x0241_0030;
/// Loaded image u32 width after mip skipping and compressed-block rounding.
pub const WIDTH: usize = 0;
/// Loaded image u32 height.
pub const HEIGHT: usize = 4;
/// Actual loaded u32 mip count, including a zero header count.
pub const MIP_COUNT: usize = 8;
/// Image u32 pixel-format code, lowered separately for D3D.
pub const FORMAT: usize = 0x0c;
/// Cubemap u32 discriminator: zero for 2D, one for cube.
pub const CUBEMAP: usize = 0x10;
/// Creation-flag storage initialized as u16; its low byte selects immutable usage.
pub const CREATION_FLAG_14: usize = 0x14;
/// Additional creation-flag byte used in immutable-usage selection.
pub const CREATION_FLAG_16: usize = 0x16;
/// Source mip-record array pointer.
pub const MIP_ROWS: usize = 0x18;
/// Source mip-record signed i32 capacity.
pub const MIP_ROWS_CAPACITY: usize = 0x20;
/// Source mip-record signed i32 count.
pub const MIP_ROWS_COUNT: usize = 0x24;
/// Array allocator pointer used when growing source rows.
pub const MIP_ROWS_ALLOCATOR: usize = 0x28;
