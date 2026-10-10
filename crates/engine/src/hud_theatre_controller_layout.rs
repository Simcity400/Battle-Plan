//! Theatre-selector GUI setup receiver, distinct from simulation CTheatre.
//! The controller's exact C++ class name remains unproven.

/// Theatre GUI controller setup.
pub const SETUP_RVA: usize = 0x0187_8e40;
/// Controller-setup entry bytes.
pub const SETUP_SENTINEL: [u8; 16] = [
    0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x55, 0x57, 0x41, 0x56, 0x48, 0x8b,
];
/// Theatre GUI controller runtime pointer.
pub const RUNTIME_ROOT: usize = 0x58;
