//! Owner of authored unitstack locator arrays; its concrete type is unknown.
//! This content table is separate from copied runtime province locators.

/// Number of inline kind-array headers.
pub const KIND_COUNT: usize = 0x27;
/// First inline kind-array header.
pub const KINDS: usize = 0;
/// Kind-array header stride.
pub const KIND_STRIDE: usize = 0x18;
/// Initializer and parser seeding each kind from province anchors before authored overrides.
pub const INITIALIZE_RVA: usize = 0x0163_d0a0;
/// Exporter walking kind headers and testing each record's presence byte.
pub const EXPORT_RVA: usize = 0x0163_da30;
/// Free kind-name lookup accepting an integer kind and returning its display name.
pub const KIND_NAME_RVA: usize = 0x0163_cec0;
