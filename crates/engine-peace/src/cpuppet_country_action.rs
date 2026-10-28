//! Puppet-country peace-action layout for the supported executable.

/// Primary vtable.
pub const VTABLE_RVA: usize = 0x02a4_c2b0;
/// Raw country reference read and serialized by the peace action.
pub const PUPPET: usize = 0x50;
/// Opaque eight bytes initialized and cloned by the action. Meaning remains
/// unresolved; do not decode or follow them as a pointer.
pub const AUXILIARY: usize = 0x58;
