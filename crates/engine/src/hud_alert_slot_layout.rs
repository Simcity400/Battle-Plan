//! One active-alert vector slot; the old lane did not establish its C++ type name.

/// Active-alert slot stride; old hud_role_observer_live/src/a729_hook.rs:52,398 advances by 468.
pub const STRIDE: usize = 0x468;
/// Active-alert i32 priority; old a729_hook.rs:53,464 reads +430.
pub const PRIORITY: usize = 0x430;
/// Active-alert first pointer-sized dedup identity; old a729_hook.rs:54,399 reads +438.
pub const IDENTITY_0: usize = 0x438;
/// Active-alert second pointer-sized dedup identity; old a729_hook.rs:55,401 reads +440.
pub const IDENTITY_1: usize = 0x440;
/// Active-alert u32 template class; old a729_hook.rs:56,466 reads +448.
pub const TEMPLATE_CLASS: usize = 0x448;
/// Active-alert third pointer-sized dedup identity; old a729_hook.rs:57,403 reads +450.
pub const IDENTITY_2: usize = 0x450;
/// Active-alert optional icon pointer; old a729_hook.rs:58,439 reads +458.
pub const ICON: usize = 0x458;
/// Active-alert i32 display order; old a729_hook.rs:59,455 reads +460 and expects -1 without an icon.
pub const DISPLAY_ORDER: usize = 0x460;
