//! Resource origin, separate from the smaller exchange object.

/// Primary vtable.
pub const VTABLE_RVA: usize = 0x2945b90;
/// Embedded resource-delivery route.
pub const DELIVERY_ROUTE: usize = 0x350;
/// State pointer matched by resource-map selection and serialized by ID.
pub const STATE: usize = 0x80;
/// Embedded-route accessor.
pub const GET_ROUTE_RVA: usize = 0xc96b10;
/// Destructor restoring the embedded route vtable.
pub const DESTRUCTOR_RVA: usize = 0xc96490;
