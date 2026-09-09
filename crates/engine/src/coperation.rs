//! COperation, the authored espionage-operation definition.

/// COperation primary vtable; build_profile.rs:516 and pinned a729 RTTI confirm the class, subobject offset zero.
pub const VTABLE_RVA: usize = 0x0292_8c98;
/// COperation authored id as an MSVC std::string; read_operations at extractor.rs:7837 reads the definition at +10.
pub const TOKEN: usize = 0x10;
