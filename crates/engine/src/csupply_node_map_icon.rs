//! Supply-node map icon presentation.

/// Naval-overlay visibility cache, u8, for kinds 3 or 5.
pub const NAVAL_OVERLAY_VISIBLE: usize = 0x19bb;
/// Background-control pointer.
pub const BACKGROUND_BUTTON: usize = 0x19c0;
/// Common supply-hub overlay pointer used for visibility and frame selection.
pub const COMMON_ICON: usize = 0x19c8;
/// Naval supply-hub overlay pointer used for visibility.
pub const NAVAL_OVERLAY: usize = 0x19d8;
/// Background selector, u32, with domain 0..4.
pub const BACKGROUND_SELECTOR: usize = 0x1a30;
/// Common-icon selector, u32: values zero through three select sprite
/// frames one, three, two and four.
pub const COMMON_ICON_SELECTOR: usize = 0x1a34;
/// World placement selects node locator or province-template anchor and
/// applies the loaded supply shift.
pub const WORLD_POSITION_RVA: usize = 0x018f_b1f0;
/// Control binder for background and both overlays.
pub const BIND_CONTROLS_RVA: usize = 0x018f_ea80;
/// Supply-node map-icon constructor.
pub const CONSTRUCT_RVA: usize = 0x018f_a9c0;
