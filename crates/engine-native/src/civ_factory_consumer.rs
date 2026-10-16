//! One civilian factory consumer embedded in production status, 0x18 bytes.
//! The five consumers begin at +0x430 (goods), +0x448 (national projects),
//! +0x460 (licences), +0x478 (trade) and +0x490 (market payments).
//!
//! Recalculation invokes goods, payments, projects, licences and trade in
//! that order, adding each allocation to civilian unavailable factories before
//! the next consumer runs. These fields are not serialized.

/// Consumer size.
pub const SIZE: usize = 0x18;
/// Factories left before this consumer, i32. Before allocating, every
/// slot-1 override truncates the civilian group's total and damaged counts
/// separately, subtracts damaged, unavailable and used factories from the
/// total, then clamps the result at zero.
pub const AVAILABLE: usize = 0x0c;
/// Factories this consumer took, i32.
pub const ALLOCATED: usize = 0x10;

/// Consumer-goods primary vtable. Allocation takes the minimum of
/// available civilian factories and required consumer goods.
pub const CONSUMER_GOODS_VTABLE_RVA: usize = 0x0295_9a48;
/// National-projects factory-consumer primary vtable.
pub const SPECIAL_PROJECTS_VTABLE_RVA: usize = 0x0295_9a60;
/// Licensed-production factory-consumer primary vtable.
pub const LICENSED_PRODUCTION_VTABLE_RVA: usize = 0x0295_9a78;
/// Trade factory-consumer primary vtable.
pub const TRADE_VTABLE_RVA: usize = 0x0295_9a90;
/// Market-payment factory-consumer primary vtable.
pub const CONTRACT_PAYMENTS_VTABLE_RVA: usize = 0x0295_9aa8;
