//! Texture-file normalization and loading metadata. Initial normalization
//! state and lifetime are not established; these sites do not authorize
//! simulation acquisition or hook installation.

/// Power-of-two normalization gate byte.
pub const FORCE_POWER_OF_TWO_RVA: usize = 0x035c_194a;
/// Normalization gate getter.
pub const GET_FORCE_POWER_OF_TWO_RVA: usize = 0x023f_07b0;
/// Normalization gate setter.
pub const SET_FORCE_POWER_OF_TWO_RVA: usize = 0x023f_0790;
/// Texture file loader checking file length before parsing or upload.
pub const LOAD_FILE_RVA: usize = 0x023f_2d50;
/// Padding helper retaining power-of-two extents and zero-padding other extents.
pub const PAD_POWER_OF_TWO_RVA: usize = 0x023f_8fa0;
