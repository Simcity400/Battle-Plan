//! CCountryOccupationStatus, confirmed by the pinned executable's RTTI.
//! Its primary base is CGarrisonStatus/CEquipmentDistributable; CPersistent is at +0x18.
//! These harvested fields do not establish hourly consistency or writer ownership.

/// CCountryOccupationStatus primary vtable; old D52 collector_module.rs:37, confirmed by pinned RTTI.
pub const VTABLE_RVA: usize = 0x0296_dd50;
/// CCountryOccupationStatus CPersistent vtable; old D52 collector_module.rs:38 and pinned RTTI.
pub const SECONDARY_VTABLE_RVA: usize = 0x0296_dd88;
/// CCountryOccupationStatus CPersistent base displacement; old D52 double-vtable guard and pinned RTTI agree.
pub const SECONDARY_BASE: usize = 0x18;
/// CCountryOccupationStatus embedded occupation-data robin-hood table; old D66 attach_log.rs:148589-148628 enumerates live buckets.
pub const OCCUPATION_DATA_TABLE: usize = 0x48;
/// CCountryOccupationStatus occupation-data pointer vector; old a729_garrison_alert.rs:446-453 reads this independent index.
pub const OCCUPATION_DATA_ROWS: usize = 0x60;
/// CCountryOccupationStatus occupation-data vector count, i32 read; old a729_garrison_alert.rs:449-453.
pub const OCCUPATION_DATA_COUNT: usize = 0x6c;
/// CCountryOccupationStatus garrison CReferencedDivisionTemplate pointer; old D52 read_country_occupation_status validates type 52 and vtable.
pub const GARRISON_TEMPLATE: usize = 0x78;
/// CCountryOccupationStatus embedded assigned-manpower container; old a729_garrison_alert.rs:348 sums its i32 entry amounts.
pub const ASSIGNED_MANPOWER: usize = 0x90;
/// CCountryOccupationStatus default COccupationLaw pointer; old D52 collector_module.rs:40 and read_country_occupation_status.
pub const DEFAULT_LAW: usize = 0x118;
pub const RESET_RVA: usize = 0x00fe_0c30;
