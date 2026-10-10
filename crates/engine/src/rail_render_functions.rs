//! Rail-rendering routines and immutable datum metadata.
//! Receiver types are not established; these helpers do not define a shared
//! object layout.

/// Railroad DDS resource loader.
pub const LOAD_RAIL_TEXTURE_RVA: usize = 0x0124_9190;
/// Directed province-pair connector selector using the nearest border pixel.
pub const SELECT_DIRECTED_CONNECTOR_RVA: usize = 0x0159_9160;
/// Rail bridge connector geometry, using effective bridge width and gap defines.
pub const BUILD_BRIDGE_CONNECTOR_RVA: usize = 0x01c5_1a40;
/// Rail ribbon edges from a normalized perpendicular and signed half-widths.
pub const BUILD_RIBBON_EDGES_RVA: usize = 0x01c5_1790;
/// Immutable f32 -0.5 datum used by ribbon edges.
pub const NEGATIVE_HALF_WIDTH_RVA: usize = 0x0298_43a8;
/// Cubic Bezier evaluator with four Bernstein basis terms.
pub const EVALUATE_CUBIC_BEZIER_RVA: usize = 0x01c5_0df0;
/// Immutable f32 3.0 cubic-basis coefficient.
pub const BEZIER_COEFFICIENT_THREE_RVA: usize = 0x0293_4a80;
/// Bezier control handles use tangent times the smaller of half chord and 3.0.
pub const BUILD_BEZIER_HANDLES_RVA: usize = 0x01c5_1100;
/// Immutable f32 0.0625 step for 16-sample arc-length evaluation.
pub const ARC_LENGTH_SAMPLE_STEP_RVA: usize = 0x02a4_2a7c;
/// Rail texture-repeat integer-rounding helper.
pub const ROUND_TEXTURE_REPEAT_RVA: usize = 0x0254_e288;
