//! Resource-delivery route returned by convoy clients, separate from the
//! exchange object in classic trade buckets.

/// Primary vtable.
pub const VTABLE_RVA: usize = 0x2945b40;
/// Candidate route-kind field; meaning and width unresolved.
pub const KIND_CANDIDATE: usize = 0x08;
/// Owner/source raw country link, u32.
pub const SOURCE: usize = 0x30;
/// Counterpart/destination raw country link, u32, resolved through country links.
pub const DESTINATION: usize = 0x34;
/// Delivery-route constructor receiving both endpoints.
pub const CONSTRUCTOR_RVA: usize = 0xc958c0;
