//! Resolved scripted GUI property-binding record; exact engine type name unproven.
//!
//! Source: `extractor/native/lanes/scripted_state_flags_variables_live/src/a729_scripted_gui_runtime.rs`.
//! The old presentation walk read these records; no hourly-state validity is claimed.

/// Property-binding record size; old read_property_bindings uses this array stride (source:50,912).
pub const STRIDE: usize = 0x88;
/// Binding target pointer; old read_property_bindings reads a pointer at the row base (source:915).
pub const TARGET: usize = 0;
/// Authored element name, MSVC string; old read_property_bindings reads it here (source:916).
pub const AUTHORED_ELEMENT: usize = 0x08;
/// Authored property name, MSVC string; old read_property_bindings reads it here (source:917).
pub const AUTHORED_PROPERTY: usize = 0x28;
/// Resolved element name, MSVC string; old read_property_bindings reads it here (source:918).
pub const RESOLVED_ELEMENT: usize = 0x48;
/// Resolved property name, MSVC string; old read_property_bindings reads it here (source:919).
pub const RESOLVED_PROPERTY: usize = 0x68;
