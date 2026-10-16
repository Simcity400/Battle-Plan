//! CMap dimensions and content rasters.
//! Weather display dimensions divide each signed full dimension by four,
//! rounding toward zero.

/// Map-dimensions owner singleton pointer.
pub const SINGLETON_RVA: usize = 0x0332_0db8;
/// Owner +0x40 map width, native i32.
pub const WIDTH: usize = 0x40;
/// Owner +0x44 map height, native i32.
pub const HEIGHT: usize = 0x44;

/// City bitmap pointer; storage is established, terrain-mask meaning remains unknown.
pub const CITY_TYPE_MASK: usize = 0x170;
/// CMap height bitmap pointer at +0x178.
pub const HEIGHT_BITMAP: usize = 0x178;
/// City-neighbour bitmap pointer; river interpretation remains unproven.
pub const CITY_NEIGHBOUR_MASK: usize = 0x180;
/// Province-id raster owner, a 0x78-byte object.
pub const PROVINCE_ID_RASTER: usize = 0x830;
/// CMap provinces.bmp path std::string.
pub const PROVINCES_BITMAP_PATH: usize = 0xb0;
/// CMap definition.csv path std::string.
pub const DEFINITION_PATH: usize = 0x1b8;
/// CMap i32 province record count.
pub const PROVINCE_RECORD_COUNT: usize = 0x230;
/// Province-raster loader allocates its storage and fills dimensions.
pub const LOAD_PROVINCE_RASTER_RVA: usize = 0x00a5_0210;
/// Province-raster run builder uses u16 runs.
pub const BUILD_PROVINCE_RUNS_RVA: usize = 0x00a5_07d0;
/// Province-template population keeps one template per definition-record position.
pub const BUILD_PROVINCE_TEMPLATES_RVA: usize = 0x00a5_5740;
/// Naval distance-cache load/build/save validates topology digest and payload length.
pub const LOAD_NAVAL_DISTANCE_CACHE_RVA: usize = 0x00a4_e4b0;
/// City-neighbour mask byte sampler.
pub const SAMPLE_CITY_NEIGHBOUR_MASK_RVA: usize = 0x00a5_23d0;
/// Signed naval-distance matrix dimension, selecting the triangular pair count.
pub const NAVAL_DISTANCE_DIMENSION: usize = 0x2a0;
/// CMap native 16-bit naval-distance matrix pointer.
pub const NAVAL_DISTANCES: usize = 0x2a8;
/// CMap signed naval-distance element count.
pub const NAVAL_DISTANCE_COUNT: usize = 0x2b4;
/// CMap naval-distance element width.
pub const NAVAL_DISTANCE_STRIDE: usize = 2;
/// Height sampler subtracts half a pixel and clamps the base cell.
pub const SAMPLE_HEIGHT_BASE_CLAMP_RVA: usize = 0x00a5_1f30;
/// Height sampler adds half a pixel and clamps each neighbour.
pub const SAMPLE_HEIGHT_NEIGHBOUR_CLAMP_RVA: usize = 0x00a5_1d40;
/// CMap samplers' immutable native f32 sea-floor literal (9.5), not an object field.
pub const SEA_FLOOR_RVA: usize = 0x0292_7a14;
/// Bitmap-derived coastal-flag rebuild.
pub const REBUILD_COASTAL_FLAGS_RVA: usize = 0x00a5_63f0;
/// Four-neighbour adjacency derivation reads the province raster.
pub const DERIVE_FOUR_NEIGHBOUR_ADJACENCY_RVA: usize = 0x00a4_ede0;
