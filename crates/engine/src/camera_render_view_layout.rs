//! Camera render-view record embedded three times in the camera root. Its
//! concrete type is unknown. Offsets are relative to the view, independently of
//! the root and render-context argument. Another owner embeds the same layout.
//! Matrix builders and normalization are associated helpers, with no established
//! view-method receiver.

/// Observed view span; this does not establish its complete allocation size.
pub const OBSERVED_BYTES: usize = 0x1b4;
/// View-projection matrix, [f32; 16].
pub const VIEW_PROJECTION: usize = 0x80;
/// Current eye, [f32; 3], compared with the camera root's current eye.
pub const CURRENT_EYE: usize = 0x180;
/// Current look-at, [f32; 3], compared with the camera root's current look-at.
pub const CURRENT_LOOK_AT: usize = 0x18c;
/// Up vector, f32[3], passed to both basis builders.
pub const UP: usize = 0x198;
/// Eye Y, f32; mirroring subtracts it from twice the plane height.
pub const EYE_Y: usize = 0x184;
/// Look-at Y, f32, mirrored before rebuilding matrices.
pub const LOOK_AT_Y: usize = 0x190;
/// Up Y, f32; mirroring flips its sign bit.
pub const UP_Y: usize = 0x19c;
/// View matrix, f32[16].
pub const VIEW_MATRIX: usize = 0x00;
/// Camera-to-world matrix, f32[16].
pub const CAMERA_TO_WORLD_MATRIX: usize = 0xc0;
/// Rebuild matrices from eye, target, up and projection inputs.
pub const UPDATE_RVA: usize = 0x022c_4440;
/// Mirror through a supplied f32 plane height before rebuilding matrices.
pub const MIRRORED_UPDATE_RVA: usize = 0x022c_3ec0;
/// View-matrix builder.
pub const VIEW_MATRIX_BUILDER_RVA: usize = 0x024e_8330;
/// Camera-to-world builder with right, up and forward rows.
pub const CAMERA_TO_WORLD_BUILDER_RVA: usize = 0x024e_80a0;
/// Normalize f32[3], returning zero for zero length.
pub const VECTOR_NORMALIZE_RVA: usize = 0x024e_6f10;
/// Both basis builders normalize forward, right and up once each.
pub const NORMALIZATIONS_PER_BASIS: usize = 3;

/// Projection builder dividing by the rounded depth quotient.
pub const PROJECTION_MATRIX_BUILDER_RVA: usize = 0x024e_8280;
