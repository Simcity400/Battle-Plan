//! Particle-editor function layouts.

/// Subsystem comparator including position Y and hidden state.
pub const COMPARE_SUBSYSTEMS_RVA: usize = 0x0208_0120;
/// Editor registration selecting a subsystem and binding its fields.
pub const REGISTER_FIELDS_RVA: usize = 0x0208_5a00;
/// Attribute comparator for two subsystem members.
pub const COMPARE_ATTRIBUTE_RVA: usize = 0x0208_09c0;
/// Position-Y editor callback.
pub const ATTRIBUTE_CALLBACK_RVA: usize = 0x0207_5940;
/// Attribute registration receiving name, member and callback.
pub const REGISTER_ATTRIBUTE_RVA: usize = 0x024a_2a90;
/// Hidden/Boolean field registration.
pub const REGISTER_HIDDEN_RVA: usize = 0x024a_2f70;
/// Auxiliary editor-registration data; semantics are unknown.
pub const HIDDEN_AUXILIARY_DATA_RVA: usize = 0x0270_073d;
