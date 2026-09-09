/// CPoliticalStatus vtable from the old validated_political_status_ptr reader.
pub const VTABLE_RVA: usize = 0x0293_9768;
/// CPoliticalStatus owning CCountry pointer; the old reader checks this backlink.
pub const COUNTRY: usize = 0xc8;
pub const POLITICAL_POWER: usize = 0xe0;

/// CPoliticalStatus party-pointer array; old offsets.rs:78 and read_political_parties_with_window follow it.
pub const PARTIES: usize = 0x20;
/// CPoliticalStatus party count, u32; old politics_parties_header reads +0x28 before +0x2c capacity.
pub const PARTY_COUNT: usize = 0x28;
/// CPoliticalStatus party capacity, u32; old offsets.rs:80 preserves this nonstandard count/capacity order.
pub const PARTY_CAPACITY: usize = 0x2c;
/// CPoliticalStatus active CIdea pointer array; old offsets.rs:81 and read_active_ideas_with_window.
pub const IDEAS: usize = 0x50;
/// CPoliticalStatus idea-array capacity, u32; old offsets.rs:82 and politics_ideas_header.
pub const IDEA_CAPACITY: usize = 0x58;
/// CPoliticalStatus idea-array count, u32; old offsets.rs:83 and politics_ideas_header.
pub const IDEA_COUNT: usize = 0x5c;
/// CPoliticalStatus last-election raw i32 hour; old read_country_politics reads +0xa0 then decode_datehour.
pub const LAST_ELECTION: usize = 0xa0;
/// CPoliticalStatus ruling CPoliticalParty pointer; old offsets.rs:85 and country-leader reader.
pub const RULING_PARTY: usize = 0xd0;
/// CPoliticalStatus election-frequency i32; old politics_status_i32_or_read uses offsets.rs:88.
pub const ELECTION_FREQUENCY: usize = 0xe8;
/// CPoliticalStatus elections-allowed u8; old politics_status_u8_or_read uses offsets.rs:89.
pub const ELECTIONS_ALLOWED: usize = 0xec;
