//! Resource origin, separate from the smaller exchange object.

/// Primary vtable.
pub const VTABLE_RVA: usize = 0x29d8d90;
/// Embedded resource-delivery route.
pub const DELIVERY_ROUTE: usize = 0x350;
/// State pointer matched by resource-map selection and serialized by ID.
pub const STATE: usize = 0x80;
/// Destructor restoring the embedded route vtable.
pub const DESTRUCTOR_RVA: usize = 0xd04c80;
