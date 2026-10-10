//! Scripted GUI updater presentation layout.
//! No update hook or virtual call is enabled.

/// Updater entry.
pub const UPDATE_RVA: usize = 0x00b7_9820;
/// Exact updater-entry sentinel.
pub const UPDATE_SENTINEL: [u8; 20] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x55, 0x56, 0x57, 0x41, 0x56, 0x41, 0x57, 0x48, 0x81, 0xec, 0x20,
    0x01, 0x00, 0x00, 0x44,
];
/// Accepted text-setter return site.
pub const PRIMARY_TEXT_RETURN_RVA: usize = 0x00b7_c3ca;
/// Accepted sprite-setter return site.
pub const SPRITE_RETURN_RVA: usize = 0x00b7_cac1;
/// Definition pointer after update.
pub const DEFINITION: usize = 0xe8;
/// Live-window pointer compared before and after update.
pub const LIVE_WINDOW: usize = 0xf8;
/// Embedded script context with country field at +0xa8.
pub const CONTEXT: usize = 0x3c0;
/// Effect-enabled widget-pointer array.
pub const EFFECT_WIDGETS: usize = 0x990;
/// Effect-enabled widget count, u32.
pub const EFFECT_WIDGET_COUNT: usize = 0x99c;
/// First visible-widget pointer array.
pub const VISIBLE_WIDGETS_PRIMARY: usize = 0x9a8;
/// First visible-widget count, u32.
pub const VISIBLE_WIDGETS_PRIMARY_COUNT: usize = 0x9b4;
/// Second visible-widget pointer array.
pub const VISIBLE_WIDGETS_SECONDARY: usize = 0x9c0;
/// Second visible-widget count, u32.
pub const VISIBLE_WIDGETS_SECONDARY_COUNT: usize = 0x9cc;
/// Resolved-property record array with 0x88-byte rows.
pub const RESOLVED_PROPERTIES: usize = 0xa50;
/// Resolved-property count, u32.
pub const RESOLVED_PROPERTY_COUNT: usize = 0xa5c;
/// Root visibility byte after update, interpreted as nonzero.
pub const ROOT_VISIBLE: usize = 0xb79;
