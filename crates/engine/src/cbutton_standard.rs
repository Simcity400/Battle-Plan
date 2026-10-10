//! `CButtonStandard`; native drawable effect values are separate from CButton state.
//!
//! These presentation constants do not establish a retained widget's current
//! state, nor do they make this widget interchangeable with CIcon.

/// Standard-button primary vtable.
pub const VTABLE_RVA: usize = 0x02b3_0528;
/// Observable subobject vtable.
pub const OBSERVABLE_VTABLE_RVA: usize = 0x02b3_08d8;
/// Constructor building the button base before installing derived vtables.
pub const CONSTRUCTOR_RVA: usize = 0x022b_a900;
/// Fresh drawable effect; zero is unassigned, independently of widget Up.
pub const EFFECT_UNASSIGNED: i32 = 0;
/// Drawable Up effect, distinct from CButton state Up.
pub const EFFECT_UP: i32 = 1;
/// Drawable Down effect.
pub const EFFECT_DOWN: i32 = 2;
/// Drawable Over effect.
pub const EFFECT_OVER: i32 = 3;
/// Drawable Disable effect.
pub const EFFECT_DISABLED: i32 = 4;
