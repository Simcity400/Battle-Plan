//! CCountryTradeView presentation layout, excluded from hourly recording.

/// CCountryTradeView primary vtable.
pub const VTABLE_RVA: usize = 0x029c_b560;
/// Trade-view opening entry; the selected country is observed after opening.
pub const OPEN_RVA: usize = 0x015e_c520;
/// Exact trade-view opening entry bytes.
pub const OPEN_SENTINEL: [u8; 16] = [
    0x40, 0x53, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xd9, 0xe8, 0x82, 0x2b, 0x81, 0xff, 0x83, 0x38,
];
/// Trade-view closing entry.
pub const CLOSE_RVA: usize = 0x015e_af40;
/// Exact trade-view closing entry bytes.
pub const CLOSE_SENTINEL: [u8; 16] = [
    0x40, 0x53, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xd9, 0x48, 0x8b, 0x89, 0x10, 0x1f, 0x00, 0x00,
];
/// View +0x548 selected-country source-holder pointer.
pub const COUNTRY_SOURCE_HOLDER: usize = 0x548;
