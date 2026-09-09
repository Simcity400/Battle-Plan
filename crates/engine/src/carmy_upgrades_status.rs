//! CArmyUpgradesStatus, reached through CCountry's upgrade-status pointer.
//!
//! The pinned executable's RTTI confirms both vtables and the persistent
//! subobject displacement. Fields come from the old D93 production reader;
//! their hourly consistency has not yet been assessed.

/// CArmyUpgradesStatus primary vtable; pinned-exe RTTI matches old country_army_upgrade_requests_live/src/lib.rs:12.
pub const VTABLE_RVA: usize = 0x29bb408;
/// CArmyUpgradesStatus persistent subobject displacement; pinned-exe RTTI confirms old D93 lib.rs:13.
pub const PERSISTENT_SUBOBJECT: usize = 0x18;
/// CArmyUpgradesStatus persistent vtable; pinned-exe RTTI confirms the +0x18 subobject in old D93 lib.rs:14.
pub const PERSISTENT_VTABLE_RVA: usize = 0x29bb440;
/// CArmyUpgradesStatus group pointer-array data; old D93 read_country_groups reads this vector at +0x20.
pub const GROUPS_DATA: usize = 0x20;
/// CArmyUpgradesStatus group vector capacity, i32; old D93 lib.rs:16 and vector_header_at confirm the width.
pub const GROUPS_CAPACITY: usize = 0x28;
/// CArmyUpgradesStatus group vector count, i32; old D93 lib.rs:17 and vector_header_at confirm the width.
pub const GROUPS_COUNT: usize = 0x2c;
/// CArmyUpgradesStatus group-vector entry width; old D93 read_country_groups walks 8-byte CArmyUpgradesRequests pointers.
pub const GROUP_ENTRY_STRIDE: usize = 0x08;
