//! Naval-base convoy client layout.

/// CNavalBaseConvoyClient vtable.
pub const VTABLE_RVA: usize = 0x0298_bab0;
/// CNavalBaseConvoyClient embedded CResourceDeliveryRoute.
pub const ROUTE: usize = 0xa0;
/// CNavalBaseConvoyClient GetRoute.
pub const GET_ROUTE_RVA: usize = 0x0069_f150;
/// CNavalBaseConvoyClient destructor.
pub const DESTRUCTOR_RVA: usize = 0x0120_5df0;
