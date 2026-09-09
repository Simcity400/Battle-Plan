//! NInternationalMarket::CPurchaseContract from the pinned build.
//!
//! The old walker authenticated identity with RTTI/type and seller/buyer roots.
//! Its equipment/delivery subbuffer scans did not prove a native row layout or
//! amount width. Scalar offsets read as i32 below retain that uncertainty;
//! they are not a claim that the engine's underlying values are 32-bit.

/// CPurchaseContract primary vtable; pinned RTTI confirms build_profile.rs:776.
pub const VTABLE_RVA: usize = 0x2954c70;
/// CPurchaseContract persistent handle type, u32; old read_compact_market_trade_row validates value 81.
pub const ID_TYPE: usize = 0x08;
/// CPurchaseContract persistent handle id, u32; old read_compact_market_trade_row reads build_profile.rs:778.
pub const ID: usize = 0x0c;
/// CPurchaseContract handle type 81; old build_profile.rs:793 and authenticated contract reader agree.
pub const TYPE: u32 = 81;
/// CPurchaseContract equipment subbuffer pointer; build_profile.rs:779, old reader's interior scan remains heuristic.
pub const EQUIPMENT_SUBBUFFER: usize = 0x38;
/// CPurchaseContract seller raw country link, u32; old reader matches +0x58 against the country census.
pub const SELLER: usize = 0x58;
/// CPurchaseContract buyer/delivery raw country link, u32; old reader matches +0x5C against the country census.
pub const BUYER: usize = 0x5c;
/// CPurchaseContract delivery subbuffer pointer; build_profile.rs:783, old reader's amount scan remains heuristic.
pub const DELIVERY_SUBBUFFER: usize = 0x80;
/// CPurchaseContract speed at +0xC0; old detail reader reads i32, native writer width unproved.
pub const SPEED: usize = 0xc0;
/// CPurchaseContract assigned convoy count; old detail reader reads i32 at build_profile.rs:785.
pub const CONVOYS_ASSIGNED: usize = 0x118;
/// CPurchaseContract total convoy count; old detail reader reads i32 at build_profile.rs:786.
pub const CONVOYS_TOTAL: usize = 0x11c;
/// CPurchaseContract buyer CCountry pointer; old reader cross-checks +0x128 against buyer identity.
pub const BUYER_POINTER: usize = 0x128;
/// CPurchaseContract efficiency at +0x1C0; old detail reader reads i32, native writer width unproved.
pub const EFFICIENCY: usize = 0x1c0;
/// CPurchaseContract remaining days at +0x1D0; old detail reader reads i32 from build_profile.rs:789.
pub const DAYS: usize = 0x1d0;
/// CPurchaseContract factory progress at +0x1E0; old detail reader reads i32, native writer width unproved.
pub const FACTORY_PROGRESS: usize = 0x1e0;
/// CPurchaseContract equipment progress at +0x1E8; old detail reader reads i32, native writer width unproved.
pub const EQUIPMENT_PROGRESS: usize = 0x1e8;
/// CPurchaseContract completion flag at +0x25C; old detail reader reads i32, engine field width unproved.
pub const COMPLETED: usize = 0x25c;
