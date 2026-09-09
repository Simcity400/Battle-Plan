//! CCountryCollaborationStatus, confirmed by the pinned executable's RTTI.
//! The old reader required each outgoing relation to have a reciprocal incoming country link.

/// CCountryCollaborationStatus vtable; old D52 collector_module.rs:44 and pinned RTTI agree.
pub const VTABLE_RVA: usize = 0x029a_5d58;
/// CCountryCollaborationStatus outgoing CCountryCollaborationData pointer array; old read_country_collaboration_relations at collector_module.rs:641.
pub const RELATIONS: usize = 0x28;
/// CCountryCollaborationStatus outgoing array count, u32; old collector_module.rs:642.
pub const RELATION_COUNT: usize = 0x34;
/// CCountryCollaborationStatus owning CCountry pointer; old collector_module.rs:643 checks this exact back-reference.
pub const OWNER: usize = 0x40;
/// CCountryCollaborationStatus incoming u32 country-link array; old collector_module.rs:680-700 validates reciprocal membership.
pub const INCOMING_COUNTRIES: usize = 0x48;
/// CCountryCollaborationStatus incoming country-link count, u32; old collector_module.rs:681-685.
pub const INCOMING_COUNT: usize = 0x54;
