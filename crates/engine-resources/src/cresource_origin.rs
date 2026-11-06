//! Resource origin, separate from the smaller exchange object.

use engine_sites::Rva;
/// Primary vtable.
pub const VTABLE_RVA: Rva = Rva(0x29d8dc0);
/// Embedded resource-delivery route.
pub const DELIVERY_ROUTE: usize = 0x350;
/// State pointer matched by resource-map selection and serialized by ID.
pub const STATE: usize = 0x80;
/// Destructor restoring the embedded route vtable.
pub const DESTRUCTOR_RVA: Rva = Rva(0xd04cf0);
