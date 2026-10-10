//! Event-popup presentation layout; this module installs no event hooks.

/// Event window instance id, u32.
pub const INSTANCE_ID: usize = 0x60;
/// Event window CEvent pointer.
pub const EVENT: usize = 0x68;
/// Event window inline CEventScope.
pub const SCOPE: usize = 0x78;
/// Event window scope country, u32.
pub const SCOPE_COUNTRY_TAG: usize = 0x80;
/// Event window absolute due tick, u32.
pub const DUE_TICK: usize = 0x1268;
/// Popup materialization.
pub const MATERIALISE_RVA: usize = 0x0122_6000;
/// Popup-materialization entry bytes.
pub const MATERIALISE_SENTINEL: [u8; 15] = [
    0x48, 0x8b, 0xc4, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57,
];
/// Popup-picture selection.
pub const PICTURE_SELECTOR_RVA: usize = 0x0122_5130;
/// Popup-picture selection entry bytes.
pub const PICTURE_SELECTOR_SENTINEL: [u8; 15] = [
    0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x08, 0x48, 0x89, 0x70, 0x18, 0x55, 0x57, 0x41, 0x54,
];
/// Player option-click handler.
pub const PLAYER_CHOICE_RVA: usize = 0x0122_49f0;
/// Option-click handler entry bytes.
pub const PLAYER_CHOICE_SENTINEL: [u8; 15] = [
    0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec, 0x30,
];
/// Timeout and no-choice resolution.
pub const TIMEOUT_RVA: usize = 0x0122_7360;
/// Timeout-resolution entry bytes.
pub const TIMEOUT_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x18, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57,
];
/// Visible-option builder.
pub const OPTION_BUILDER_RVA: usize = 0x0122_3bf0;
/// Title-setter call.
pub const TITLE_SETTER_CALL_RVA: usize = 0x0122_6334;
/// Title-setter call bytes.
pub const TITLE_SETTER_CALL_SENTINEL: [u8; 5] = [0xe8, 0xa7, 0xf6, 0x08, 0x01];
/// Title-setter return.
pub const TITLE_SETTER_RETURN_RVA: usize = 0x0122_6339;
/// Description-setter call.
pub const DESCRIPTION_SETTER_CALL_RVA: usize = 0x0122_6d84;
/// Description-setter call bytes.
pub const DESCRIPTION_SETTER_CALL_SENTINEL: [u8; 5] = [0xe8, 0x57, 0xec, 0x08, 0x01];
/// Description-setter return.
pub const DESCRIPTION_SETTER_RETURN_RVA: usize = 0x0122_6d89;
/// Option-setter call.
pub const OPTION_SETTER_CALL_RVA: usize = 0x0122_3deb;
/// Option-setter call bytes.
pub const OPTION_SETTER_CALL_SENTINEL: [u8; 5] = [0xe8, 0xf0, 0x1b, 0x09, 0x01];
/// Option-setter return.
pub const OPTION_SETTER_RETURN_RVA: usize = 0x0122_3df0;
