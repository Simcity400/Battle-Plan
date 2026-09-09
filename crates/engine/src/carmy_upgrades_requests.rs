//! CArmyUpgradesRequests for one CArmy, with request rows and delivery pointers.
//!
//! Both vtables are named by pinned-executable RTTI. The old D93 reader checks
//! the status back-reference and version, but no hourly consistency proof is
//! carried forward from its external-read retries.

/// CArmyUpgradesRequests primary vtable; pinned-exe RTTI confirms old D93 lib.rs:19.
pub const VTABLE_RVA: usize = 0x29bb380;
/// CArmyUpgradesRequests native version, u32; old D93 read_country_groups reads and checks +0x08.
pub const VERSION: usize = 0x08;
/// CArmyUpgradesRequests native version sentinel; old D93 read_country_groups requires the a729 value 1.
pub const EXPECTED_VERSION: u32 = 1;
/// CArmyUpgradesRequests persistent subobject displacement; pinned-exe RTTI confirms old D93 lib.rs:22.
pub const PERSISTENT_SUBOBJECT: usize = 0x18;
/// CArmyUpgradesRequests persistent vtable; pinned-exe RTTI confirms the +0x18 subobject in old D93 lib.rs:23.
pub const PERSISTENT_VTABLE_RVA: usize = 0x29bb3b8;
/// CArmyUpgradesRequests CArmy pointer; old D93 read_country_groups validates both target vtables and its persistent id.
pub const ARMY: usize = 0x20;
/// CArmyUpgradesRequests CArmyUpgradesStatus back-reference; old D93 read_country_groups checks it against the owning status.
pub const STATUS: usize = 0x28;
/// CArmyUpgradesRequests request-row vector data; old D93 read_request_rows walks 0x18-byte entries from +0x30.
pub const REQUESTS_DATA: usize = 0x30;
/// CArmyUpgradesRequests request-row vector capacity, i32; old D93 lib.rs:27 and vector_header_at confirm the width.
pub const REQUESTS_CAPACITY: usize = 0x38;
/// CArmyUpgradesRequests request-row vector count, i32; old D93 lib.rs:28 and vector_header_at confirm the width.
pub const REQUESTS_COUNT: usize = 0x3c;
/// CArmyUpgradesRequests CUpgradeDelivery pointer-vector data; old D93 read_deliveries starts at +0x48.
pub const DELIVERIES_DATA: usize = 0x48;
/// CArmyUpgradesRequests delivery-vector capacity, i32; old D93 lib.rs:34 and vector_header_at confirm the width.
pub const DELIVERIES_CAPACITY: usize = 0x50;
/// CArmyUpgradesRequests delivery-vector count, i32; old D93 lib.rs:35 and vector_header_at confirm the width.
pub const DELIVERIES_COUNT: usize = 0x54;
/// CArmyUpgradesRequests delivery-vector entry width; old D93 read_deliveries reads 8-byte CUpgradeDelivery pointers.
pub const DELIVERY_ENTRY_STRIDE: usize = 0x08;
