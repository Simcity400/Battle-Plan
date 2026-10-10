//! One active-alert vector slot.

/// Active-alert slot stride.
pub const STRIDE: usize = 0x468;
/// Active-alert i32 priority.
pub const PRIORITY: usize = 0x430;
/// Active-alert first pointer-sized dedup identity.
pub const IDENTITY_0: usize = 0x438;
/// Active-alert second pointer-sized dedup identity.
pub const IDENTITY_1: usize = 0x440;
/// Active-alert u32 template class.
pub const TEMPLATE_CLASS: usize = 0x448;
/// Active-alert third pointer-sized dedup identity.
pub const IDENTITY_2: usize = 0x450;
/// Active-alert optional icon pointer.
pub const ICON: usize = 0x458;
/// Signed i32 display order; negative one when no icon is present.
pub const DISPLAY_ORDER: usize = 0x460;
