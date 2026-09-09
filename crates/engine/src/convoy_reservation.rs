//! A CConvoys reservation-vector pointee; its concrete C++ class remains unproved.

/// Reservation priority; old read_country_convoys reads signed i32 at the pointee's +0.
pub const PRIORITY: usize = 0;
/// Reservation count; old read_country_convoys reads signed i32 at the pointee's +4 and then applies nonnegative admission policy.
pub const COUNT: usize = 4;
