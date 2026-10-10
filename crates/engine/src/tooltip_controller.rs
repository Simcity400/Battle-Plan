//! Tooltip controller presentation layout. Its virtual dispatch does
//! not establish a simulation table.

/// Screen position, i32 pair.
pub const POSITION: usize = 0x4e0;
/// Text-box pointer requiring class authentication.
pub const TOOLTIP_OWNER: usize = 0x4e8;
/// Viewport pointer supplying width and height.
pub const VIEWPORT: usize = 0x4f0;
/// Hover-context pointer.
pub const HOVER_CONTEXT: usize = 0x4f8;
/// Virtual dispatch offset with three implementations.
pub const PRODUCER_VSLOT: usize = 0x98;
/// Controller update entry.
pub const UPDATE_RVA: usize = 0x002a_4080;
/// Exact update-entry sentinel.
pub const UPDATE_SENTINEL: [u8; 16] = [
    0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x08, 0x88, 0x50, 0x10, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41,
];
