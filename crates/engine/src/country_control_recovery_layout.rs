//! Country control-recovery helper. Country references are resolved before
//! reading the owned-province list and changing controllers.

/// Owned-province control-recovery callable.
pub const RECOVER_OWNED_PROVINCE_CONTROL_RVA: usize = 0x00d1_1190;
