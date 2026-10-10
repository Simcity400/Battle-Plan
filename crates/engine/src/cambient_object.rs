//! Authored ambient placement, separate from its live instance. Presentation
//! state follows the camera and process clock and is excluded from hourly capture.

/// Placement allocation size.
pub const BYTES: usize = 0x60;
/// Definition pointer supplying spawn gates.
pub const OBJECT_TYPE: usize = 0x50;
/// Active byte adjusting the type's signed activation counter.
pub const ACTIVE: usize = 0x58;
/// Enabled byte, initially one and updated from the DLC trigger.
pub const ENABLED: usize = 0x59;
/// Copy constructor installing vtables and placement data.
pub const COPY_CONSTRUCTOR_RVA: usize = 0x0163_3c60;
/// Spawn gate reading active state and signed maximum-visible and counter values.
pub const SPAWN_GATE_RVA: usize = 0x0163_4330;
/// Enabled writer evaluating the optional type trigger.
pub const UPDATE_ENABLED_RVA: usize = 0x0163_4880;
/// Active setter decrementing with a zero clamp on activation and incrementing on removal.
pub const SET_ACTIVE_RVA: usize = 0x0163_48c0;
