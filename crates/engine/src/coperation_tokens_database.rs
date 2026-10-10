//! Operation-token database root and definition map.
//! A pointer and map snapshot alone does not establish hourly consistency.

/// Primary vtable.
pub const VTABLE_RVA: usize = 0x0270_4ed0;
/// Singleton pointer published when the database is constructed.
pub const SINGLETON_RVA: usize = 0x0331_6be8;
/// Inline numeric-ID map used to resolve and install definition objects.
pub const DEFINITIONS: usize = 0x40;
