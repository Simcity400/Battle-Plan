//! CPoliticalParty layout harvested from the old politics and cabinet readers.
//! The popularity reader copied four bytes; its complete native width still needs verification.

/// CPoliticalParty primary vtable, confirmed by pinned-exe RTTI; old offsets.rs:61.
pub const VTABLE_RVA: usize = 0x0298_77e0;
/// CPoliticalParty CIdeologyGroup pointer; old read_political_party_ideology_with_window validates both vtables.
pub const IDEOLOGY_GROUP: usize = 0x18;
/// CPoliticalParty country-leader pointer vector; old cabinet extractor.rs:935-960 and offsets.rs:92.
pub const COUNTRY_LEADERS: usize = 0x70;
/// CPoliticalParty leader-vector capacity u32; old cabinet extractor.rs:951 and singleton reader only accepts both words equal to one.
pub const COUNTRY_LEADER_CAPACITY: usize = 0x78;
/// CPoliticalParty leader-vector count u32; old cabinet extractor.rs:945; old politics helper swapped names when both were one.
pub const COUNTRY_LEADER_COUNT: usize = 0x7c;
/// CPoliticalParty popularity; old read_political_parties_with_window reads i32 and divides by 100000, native width unproved.
pub const POPULARITY: usize = 0x88;
