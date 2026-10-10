//! CButton state accessor used by the excluded scripted GUI observer.
//! The derived slot is called without a CButton check; its interpretation is
//! unverified for arbitrary CGuiObject-derived widgets.

/// Button primary vtable.
pub const VTABLE_RVA: usize = 0x02b2_da68;
/// Observable subobject vtable.
pub const OBSERVABLE_VTABLE_RVA: usize = 0x02b2_ddf0;
/// State accessor slot; a return value of three is treated as disabled
/// only for a CButton-authenticated receiver.
pub const STATE_VSLOT: usize = 0x278;
/// Candidate disabled state; interpreting the derived accessor
/// without checking its CButton receiver is unverified.
pub const DISABLED_STATE_CANDIDATE: i32 = 3;

/// Constructor installing button vtables after the GUI base.
pub const CONSTRUCTOR_RVA: usize = 0x0229_9f30;
/// Observable secondary subobject.
pub const OBSERVABLE_SUBOBJECT: usize = 0x80;
/// State, i32, initially zero.
pub const STATE: usize = 0xb0;
/// Input-control byte.
pub const INPUT_FLAGS: usize = 0xe4;
/// Initial input flags, independent of drawable effect.
pub const INITIAL_INPUT_FLAGS: u8 = 0xb8;
/// Input flag controlling the forced-disabled callback.
pub const FORCE_DISABLED_CALLBACK_MASK: u8 = 0x40;
/// Enabled up state.
pub const STATE_UP: i32 = 0;
/// Pressed state.
pub const STATE_DOWN: i32 = 1;
/// Hover state after a successful hit test.
pub const STATE_OVER: i32 = 2;
/// Disabled state, requiring an authenticated button receiver.
pub const STATE_DISABLED: i32 = 3;
/// Raw state getter.
pub const STATE_GETTER_RVA: usize = 0x0229_abe0;
/// Raw state setter, separate from drawable effect assignment.
pub const STATE_SETTER_RVA: usize = 0x0229_bcd0;
/// Disable operation setting state and dispatching the observer callback.
pub const DISABLE_RVA: usize = 0x0229_ab10;
/// Enable operation clearing disabled state only when the inherited veto bit is clear.
pub const ENABLE_RVA: usize = 0x0229_abb0;
/// Setter changing only the forced-disabled input flag.
pub const FORCE_DISABLED_CALLBACK_SETTER_RVA: usize = 0x0229_bcf0;
/// Input update applying state before optional forced-disabled callback and restoration.
pub const INPUT_UPDATE_RVA: usize = 0x0229_bd10;
/// State-setter virtual byte offset used to force and restore disabled state.
pub const SET_STATE_VSLOT: usize = 0x280;
/// State-callback virtual byte offset.
pub const STATE_CALLBACK_VSLOT: usize = 0x378;
/// Disabled-tooltip selector virtual byte offset.
pub const DISABLED_TOOLTIP_VSLOT: usize = 0x208;
/// Disabled-tooltip selector testing the button's disabled state.
pub const DISABLED_TOOLTIP_RVA: usize = 0x0229_b120;
/// Sprite-interface wrapper preserving state around derived replacement; icons are valid derived receivers.
pub const SPRITE_SETTER_RVA: usize = 0x0229_a780;
/// Sprite-wrapper entry sentinel.
pub const SPRITE_SETTER_SENTINEL: [u8; 20] = [
    0x48, 0x85, 0xc9, 0x0f, 0x84, 0x47, 0x01, 0x00, 0x00, 0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89,
    0x74, 0x24, 0x20, 0x57,
];
/// Derived sprite-setter virtual byte offset.
pub const SET_SPRITE_VSLOT: usize = 0x2d8;
/// Derived drawable-getter virtual byte offset; icons return their separate drawable pointer.
pub const DRAWABLE_VSLOT: usize = 0x2e8;
