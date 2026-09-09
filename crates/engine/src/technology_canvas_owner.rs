//! Unnamed owner of technology-mode CCountryTechTreeView instances.
//!
//! The archived hook confirms construction and owner-slot relationships only;
//! no C++ class name or hourly-state walk is claimed.

/// Technology canvas owner constructor; archived dynamic_collection_trace_live/src/a729_technology_canvas.rs:24 and a729_technology_canvas_hook.rs:452-468.
pub const CONSTRUCTOR_RVA: usize = 0x00b5_2390;
/// Owner constructor entry bytes; archived dynamic_collection_trace_live/src/a729_technology_canvas.rs:103-107 static sentinel.
pub const CONSTRUCTOR_SENTINEL: [u8; 16] =
    *b"\x48\x89\x5c\x24\x20\x48\x89\x4c\x24\x08\x55\x56\x57\x41\x54\x41";
/// Owner's mode-6 controller pointer; archived dynamic_collection_trace_live/src/a729_technology_canvas.rs:54 and canvas_hook.rs:456,555.
pub const MODE6_CONTROLLER: usize = 0xf8;
/// Owner's mode-7 controller pointer; archived dynamic_collection_trace_live/src/a729_technology_canvas.rs:55 static contract, not live-read by that hook.
pub const MODE7_CONTROLLER: usize = 0x100;
