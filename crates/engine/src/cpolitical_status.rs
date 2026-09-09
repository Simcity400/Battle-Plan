/// CPoliticalStatus vtable from the old validated_political_status_ptr reader.
pub const VTABLE_RVA: usize = 0x0293_9768;
/// CPoliticalStatus owning CCountry pointer; the old reader checks this backlink.
pub const COUNTRY: usize = 0xc8;
pub const POLITICAL_POWER: usize = 0xe0;
