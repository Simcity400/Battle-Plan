//! Tree/map render owner reached through graphical map +0x58. Lighting,
//! fade, border palette and tree-model arrays share this receiver. These
//! presentation fields do not establish hourly consistency.

/// Tree caster producer.
pub const PRODUCER_RVA: usize = 0x0126_5c10;
/// Exact tree-producer entry sentinel.
pub const PRODUCER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57,
];
/// Category-indexed model pointers.
pub const MODELS: usize = 0x4b38;
/// Model-pointer stride.
pub const MODEL_POINTER_STRIDE: usize = 8;
/// Tree source-row array pointer.
pub const SOURCE_ROWS: usize = 0x5330;
/// Tree source-row count, i32.
pub const SOURCE_ROW_COUNT: usize = 0x533c;
/// State-indexed partition-mask byte.
pub const STATE_MASKS: usize = 0x5348;
/// State-indexed distance, f32 bits.
pub const STATE_DISTANCES: usize = 0x534c;
/// State record stride for mask/distance pairs.
pub const STATE_STRIDE: usize = 8;
/// Excluded-distance f32 sentinel bits.
pub const EXCLUDED_DISTANCE_F32_BITS: u32 = 0x7f7f_ffff;
/// Central, west and east partition bits.
pub const PARTITION_MASK: u8 = 0x07;
/// Snow/mud fade, f32.
pub const SNOW_MUD_FADE: usize = 0x5ce8;
/// Primary sun vector, f32[3].
pub const PRIMARY_SUN: usize = 0x5d18;
/// Primary moon vector, f32[3].
pub const PRIMARY_MOON: usize = 0x5d24;
/// Lighting and fade update on this owner.
pub const LIGHTING_FADE_UPDATE_RVA: usize = 0x0126_7270;
/// Map-body draw sequence on this owner.
pub const DRAW_MAP_BODY_RVA: usize = 0x0126_8c60;
/// First optional render-object pointer.
pub const OPTIONAL_DRAW_OBJECT: usize = 0x4968;
/// Second optional render-object pointer.
pub const OPTIONAL_DRAW_OBJECT_5CE0: usize = 0x5ce0;
/// Smoothed lighting factor, f32, clamped to [0,1].
pub const SMOOTHED_LIGHTING_FACTOR: usize = 0x5d04;
/// Strait shader-program handle, 32 bits.
pub const STRAIT_PROGRAM: usize = 0x50;
/// Terrain submission on this owner.
pub const DRAW_TERRAIN_RVA: usize = 0x0126_4d30;
/// Border-bucket submission for both ranges.
pub const DRAW_BORDERS_RVA: usize = 0x0126_4420;
/// River submission on this owner.
pub const DRAW_RIVERS_RVA: usize = 0x0126_4830;
/// Main tree submission on this owner.
pub const DRAW_TREES_RVA: usize = 0x0126_53e0;
/// Water submission on this owner.
pub const DRAW_WATER_RVA: usize = 0x0126_61e0;
/// Unlit terrain/tree pass on this owner.
pub const DRAW_UNLIT_SHADOW_RVA: usize = 0x0126_9a30;
/// Sky reflection pass on this owner.
pub const DRAW_SKY_REFLECTION_RVA: usize = 0x0126_8860;
/// Border-bucket array bank at +0x4a00.
pub const BORDER_BUCKETS: usize = 0x4a00;
/// Exclusive end of the border-bucket bank.
pub const BORDER_BUCKETS_END: usize = 0x4b20;
/// Border-bucket header stride.
pub const BORDER_BUCKET_STRIDE: usize = 0x18;
/// Border-bucket count.
pub const BORDER_BUCKET_COUNT: usize = 12;
/// Secondary bank increment applied after hidden-material remapping.
pub const BORDER_SECONDARY_BANK_OFFSET: u8 = 6;
/// Border selection-palette array at +0x4b20/+0x4b28/+0x4b2c.
pub const BORDER_SELECTION_PALETTE: usize = 0x4b20;
/// RGBA palette entry size.
pub const BORDER_SELECTION_PALETTE_ENTRY_STRIDE: usize = 0x10;
/// Per-topology-row border material-byte array.
pub const BORDER_MATERIAL_ORDINALS: usize = 0x5ca8;
/// Hidden material byte; selected overlay branches remap it, otherwise emission is suppressed.
pub const BORDER_HIDDEN_MATERIAL: u8 = 0xff;
/// Border selector writing the twelve buckets.
pub const CLASSIFY_BORDERS_RVA: usize = 0x0126_a450;
/// Palette initializer writing nine defines-backed colors after the zero entry.
pub const INITIALIZE_BORDER_SELECTION_PALETTE_RVA: usize = 0x0126_6aa0;
/// Instruction site remapping hidden material before adding the secondary bank increment.
pub const BORDER_SECONDARY_BANK_BRANCH_RVA: usize = 0x0126_b414;
/// Instruction site applying reason-zero bucket 0/1 fade gates.
pub const BORDER_CAMERA_FADE_GATE_RVA: usize = 0x0126_b557;
