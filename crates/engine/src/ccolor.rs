//! CColor native components retained from the pinned render consumer.

/// CColor red f32, read as bits by production_render_submit_live/src/a729_hook/rail_supply_arrow.rs:737.
pub const RED: usize = 0x10;
/// CColor green f32, read as bits by rail_supply_arrow.rs:738.
pub const GREEN: usize = 0x14;
/// CColor blue f32, read as bits by rail_supply_arrow.rs:739.
pub const BLUE: usize = 0x18;
/// CColor alpha f32, read as bits by rail_supply_arrow.rs:740.
pub const ALPHA: usize = 0x1c;

pub const CONSTRUCT_RVA: usize = 0x0223_6fa0;
