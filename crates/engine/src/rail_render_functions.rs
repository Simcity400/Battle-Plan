//! Rail rendering function and immutable datum metadata in a729.
//! Receiver types for these helpers have not been established; these addresses
//! do not combine the helpers into an engine object layout.

/// Railroad DDS resource loader; rail_files.rs:2021 identifies the load/consume pair. Receiver class is unproven.
pub const LOAD_RAIL_TEXTURE_RVA: usize = 0x0124_9190;
/// Directed province-pair connector selector; rail_files.rs:2057 and va_rail_z_frame/RAIL_Z_FRAME_LAW.json identify gfx_railway_manager.cpp and its nearest-border-pixel selection.
pub const SELECT_DIRECTED_CONNECTOR_RVA: usize = 0x0159_9160;
/// Rail bridge connector geometry; rail_files.rs:2057 confirms effective bridge width/gap defines feed this helper.
pub const BUILD_BRIDGE_CONNECTOR_RVA: usize = 0x01c5_1a40;
/// Rail ribbon edge builder; rail_files.rs:2068 identifies the normalized perpendicular and positive/negative half-width operands.
pub const BUILD_RIBBON_EDGES_RVA: usize = 0x01c5_1790;
/// Native f32 -0.5 immutable datum; rail_files.rs:2068 identifies the read at VA 0x141c51914 in the ribbon edge builder.
pub const NEGATIVE_HALF_WIDTH_RVA: usize = 0x0298_43a8;
pub const EVALUATE_CUBIC_BEZIER_RVA: usize = 0x01c5_0df0;
/// Native f32 3.0 immutable datum; rail_files.rs:2075 identifies the cubic-basis coefficient read at VA 0x141c50e35.
pub const BEZIER_COEFFICIENT_THREE_RVA: usize = 0x0293_4a80;
/// Rail Bezier control-handle builder; rail_files.rs:2075 confirms tangent times min(half chord, 3.0).
pub const BUILD_BEZIER_HANDLES_RVA: usize = 0x01c5_1100;
/// Native f32 0.0625 immutable datum; rail_files.rs:2075 identifies the 16-sample arc-length step read at VA 0x141c51636.
pub const ARC_LENGTH_SAMPLE_STEP_RVA: usize = 0x02a4_2a7c;
/// Rail texture-repeat integer-rounding helper; rail_files.rs:2082 identifies the native call, but its general runtime symbol is unproven.
pub const ROUND_TEXTURE_REPEAT_RVA: usize = 0x0254_e288;
