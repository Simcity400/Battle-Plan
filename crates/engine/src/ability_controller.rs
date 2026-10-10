//! Selected command-ability GUI controller; its concrete type is unknown.
//!
//! This presentation state is populated by GUI callbacks. Persistent all-country
//! ability records use a separate object and acquisition. Controller vtable
//! identity is unavailable.

/// Controller refresh function.
pub const REFRESH_RVA: usize = 0x01d0_72f0;
/// Controller refresh entry sentinel.
pub const REFRESH_SENTINEL: [u8; 16] = [
    0x40, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56, 0x41, 0x57, 0x48, 0x8d, 0xac,
];
/// Special-context probe returning u8.
pub const SPECIAL_CONTEXT_PROBE_RVA: usize = 0x01d0_6480;
/// Special-context probe entry sentinel.
pub const SPECIAL_CONTEXT_PROBE_SENTINEL: [u8; 16] = [
    0x40, 0x53, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xd9, 0xb9, 0x39, 0x00, 0x00, 0x00, 0xe8, 0xad,
];
/// Tooltip builder receiving the embedded callback subobject.
pub const TOOLTIP_BUILDER_RVA: usize = 0x01d0_49c0;
/// Tooltip builder entry sentinel.
pub const TOOLTIP_BUILDER_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x54, 0x24, 0x10, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41,
];
/// Tooltip callback subobject; subtract from the callback owner to recover the controller.
pub const TOOLTIP_CALLBACK: usize = 0x28;
/// Row-owner pointer vector header; bracket its data, count and capacity.
pub const ROW_WIDGETS: usize = 0x70;
/// Source ability-definition pointer vector header, read at refresh completion.
pub const SOURCE_DEFINITIONS: usize = 0x718;
/// Selected context mode, u32; no complete enum is assumed.
pub const CONTEXT_MODE: usize = 0x730;
/// Selected context root pointer.
pub const SELECTED_ROOT: usize = 0x738;
/// Selected unit-leader pointer.
pub const SELECTED_LEADER: usize = 0x748;
/// Selected formation pointer; its dynamic type requires validation.
pub const SELECTED_ARMY: usize = 0x750;
/// Selected army-group pointer.
pub const SELECTED_ARMY_GROUP: usize = 0x758;
/// Other selected-object pointer; its concrete type is unknown.
pub const SELECTED_OTHER: usize = 0x760;
/// Field-marshal aggregate flag, u8 interpreted as nonzero.
pub const FIELD_MARSHAL_AGGREGATE: usize = 0x770;
/// Unit-leader pointer vector header.
pub const LEADERS: usize = 0x778;
/// Field-marshal aggregate-state vector header with 0x68-byte elements.
pub const FIELD_MARSHAL_STATES: usize = 0x790;
/// Return address after the disabled-reason append call.
pub const DISABLED_REASON_APPEND_RETURN_RVA: usize = 0x01d0_6057;
/// Disabled-reason append callsite, five bytes before its return address.
pub const DISABLED_REASON_APPEND_CALLSITE_RVA: usize = 0x01d0_6052;
/// Disabled-reason append callsite sentinel.
pub const DISABLED_REASON_APPEND_CALLSITE_SENTINEL: [u8; 16] = [
    0xe8, 0xa9, 0x36, 0x42, 0xfe, 0x48, 0x8b, 0x4e, 0x10, 0x48, 0x85, 0xc9, 0x74, 0x54, 0x4c, 0x8b,
];
/// Return address after the effective command-power cost call.
pub const EFFECTIVE_COST_RETURN_RVA: usize = 0x01d0_4e38;
/// Effective command-power cost callsite, five bytes before its return address.
pub const EFFECTIVE_COST_CALLSITE_RVA: usize = 0x01d0_4e33;
/// Effective command-power cost callsite sentinel.
pub const EFFECTIVE_COST_CALLSITE_SENTINEL: [u8; 16] = [
    0xe8, 0xf8, 0xfd, 0x2e, 0xff, 0x48, 0x8b, 0x9b, 0xf0, 0x01, 0x00, 0x00, 0x45, 0x38, 0xa6, 0xe9,
];
/// Return address after the icon-resolver call.
pub const ICON_RESOLVER_RETURN_RVA: usize = 0x01d0_8ba6;
/// Icon-resolver callsite, five bytes before its return address.
pub const ICON_RESOLVER_CALLSITE_RVA: usize = 0x01d0_8ba1;
/// Icon-resolver callsite sentinel.
pub const ICON_RESOLVER_CALLSITE_SENTINEL: [u8; 16] = [
    0xe8, 0x0a, 0xb3, 0x51, 0x00, 0x48, 0x85, 0xc0, 0x0f, 0x85, 0x28, 0x01, 0x00, 0x00, 0x38, 0x05,
];
