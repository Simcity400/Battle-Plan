//! Province definition/raster owner reached through the map, distinct from bitmap and definition records.

/// Raster-owner allocation size.
pub const SIZE: usize = 0x78;
/// Definition-record array pointer.
pub const DEFINITIONS: usize = 0;
/// Signed definition-record count.
pub const DEFINITION_COUNT: usize = 0xc;
/// U16 province-ID raster pointer.
pub const PIXELS: usize = 0x18;
/// Signed raster-element capacity.
pub const PIXEL_CAPACITY: usize = 0x20;
/// Signed raster-element count, width times height.
pub const PIXEL_COUNT: usize = 0x24;
/// Raster allocator pointer.
pub const PIXEL_ALLOCATOR: usize = 0x28;
/// Signed i32 raster width used for row indexing.
pub const WIDTH: usize = 0x30;
/// Signed i32 raster height used for bounds checks.
pub const HEIGHT: usize = 0x34;
/// Definition-path MSVC string.
pub const DEFINITION_PATH: usize = 0x38;
/// Bitmap-path MSVC string.
pub const BITMAP_PATH: usize = 0x58;
/// U16 raster-element stride; samples retain zero and sea IDs.
pub const PIXEL_STRIDE: usize = 2;
/// Raster-owner constructor.
pub const CONSTRUCT_RVA: usize = 0x0153_0960;
/// Raster-owner loader using definition sidecars.
pub const LOAD_RVA: usize = 0x0153_18f0;
/// Definition-record appender used when a raster colour requires a generated record.
pub const APPEND_DEFINITION_RVA: usize = 0x0153_07c0;
/// Colour-to-ID raster builder using u16 cells and record-count-based IDs.
pub const BUILD_ID_RASTER_RVA: usize = 0x0153_2390;
/// Cache bitmap/definition digest-pair builder.
pub const BUILD_CACHE_DIGESTS_RVA: usize = 0x0153_0c20;
/// Cache digest-pair reader.
pub const READ_CACHE_DIGESTS_RVA: usize = 0x0153_2690;
/// Fixed-definition sidecar reader.
pub const READ_FIXED_DEFINITIONS_RVA: usize = 0x0153_29e0;
/// Definition writer for ID, colour, type, coastal, terrain and continent.
pub const WRITE_DEFINITION_RVA: usize = 0x0153_2c80;
/// Definition-sidecar save helper.
pub const SAVE_RVA: usize = 0x0153_3810;
/// Sidecar writer with an authored-definition branch when write and base directories coincide.
pub const WRITE_SIDECARS_RVA: usize = 0x0153_3940;
/// Bitmap-digest lexer token for the cache.
pub const CACHE_BITMAP_DIGEST_TOKEN: u32 = 0x2830;
/// Definition-digest lexer token for the cache.
pub const CACHE_DEFINITION_DIGEST_TOKEN: u32 = 0x2832;
/// Fixed-definition suffix bytes read from the installed executable.
pub const FIXED_CSV_SUFFIX_RVA: usize = 0x029b_f4a8;
/// Cache suffix bytes read from the installed executable.
pub const CACHE_SUFFIX_RVA: usize = 0x029b_f4b4;
