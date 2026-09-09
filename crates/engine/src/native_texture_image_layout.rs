//! Anonymous image description initialized by FUN1409DA100 and filled by the DDS loader.
//! Preserved from the a729 exports referenced by archived container_sprite_resources_gpu.rs.
//! This is distinct from opaque_texture_description_layout; no RTTI identity was established.

/// Texture image initializer; container_sprite_resources_gpu.rs:125 and the retained FUN1409DA100 decompile establish zero flags and one initial mip.
pub const INITIALIZE_RVA: usize = 0x009d_a100;
/// DDS image loader; container_sprite_resources_gpu.rs:60,73 and geometry.rs:137,153 confirm FUN142410030 fills native format and actual loaded mip count.
pub const LOAD_DDS_RVA: usize = 0x0241_0030;
/// Image width u32; retained FUN142410030 assigns param1[0] after mip skipping and rounding compressed extents to whole blocks.
pub const WIDTH: usize = 0;
/// Image height u32; retained FUN142410030 assigns param1[1] alongside the effective width.
pub const HEIGHT: usize = 4;
/// Actual loaded mip count u32; retained FUN142410030 updates param1[2] to the number of rows loaded, even with a zero DDS header count.
pub const MIP_COUNT: usize = 8;
/// Native pixel-format code u32; retained FUN142410030 selects param1[3], lowered separately by FUN142411320.
pub const FORMAT: usize = 0x0c;
/// Cubemap discriminator u32, zero for 2D and one for cube; retained FUN142410030 writes param1[4] and FUN1423FB4F0 chooses one or six slices.
pub const CUBEMAP: usize = 0x10;
/// Creation flag storage, zeroed as u16; FUN1409DA100 initializes +14 and FUN1423FB4F0 tests its low byte before choosing immutable usage.
pub const CREATION_FLAG_14: usize = 0x14;
/// Additional creation flag byte; FUN1409DA100 zeros +16 and FUN1423FB4F0 tests it before choosing immutable usage; semantic name unproved.
pub const CREATION_FLAG_16: usize = 0x16;
/// Source mip-record array pointer; retained FUN142410030 stores its 0x20-byte rows through param1+6.
pub const MIP_ROWS: usize = 0x18;
/// Source mip-record capacity i32; retained FUN142410030 grows the array using param1[8].
pub const MIP_ROWS_CAPACITY: usize = 0x20;
/// Source mip-record count i32; retained FUN142410030 appends each loaded row using param1[9].
pub const MIP_ROWS_COUNT: usize = 0x24;
/// Array allocator pointer; FUN1409DA100 initializes +28 and FUN142410030 calls its allocate/free virtuals when growing source rows.
pub const MIP_ROWS_ALLOCATOR: usize = 0x28;
