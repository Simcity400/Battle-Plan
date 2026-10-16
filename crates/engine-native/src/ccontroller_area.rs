//! CControllerArea geographic membership used by CTheatre.

/// Controller-area vtable.
pub const VTABLE_RVA: usize = 0x0294_7d00;
/// Representative province pointer.
pub const REPRESENTATIVE_PROVINCE: usize = 0x28;
/// Member-province pointer array.
pub const PROVINCE_DATA: usize = 0x30;
/// Member count, i32.
pub const PROVINCE_COUNT: usize = 0x3c;
/// Process-local instance counter assigned from a global sequence and never serialized.
pub const INSTANCE_COUNTER: usize = 0x110;
/// Allocation size.
pub const SIZE: usize = 0x118;
/// Constructor installing the vtable.
pub const CONSTRUCTOR_RVA: usize = 0x00cd_be90;
/// Allocator invoking construction.
pub const CREATE_RVA: usize = 0x00ce_0ec0;
/// World clear walking members and clearing province backlinks.
pub const CLEAR_RVA: usize = 0x00cd_e710;
/// Seed flood fill appending provinces and setting each backlink.
pub const FLOOD_FILL_RVA: usize = 0x00cd_cb90;
/// Join moving source members into the destination area.
pub const JOIN_RVA: usize = 0x00ce_20a0;
/// Controller-change update called by the province controller setter.
pub const CONTROLLER_CHANGE_RVA: usize = 0x00ce_2870;

/// One controller area listed by one stationed theatre in native order.
/// Theatre insertion deduplicates pointers, but rebuilding replaces the whole
/// vector, so ordinal is only a position. The process-local area counter is
/// unserialized and supplies no persistent key.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// Full native CTheatre `+0x08` handle of the theatre that lists this area.
    pub theatre: u64,
    pub ordinal: u32,
    /// Province id behind `+0x28`; a null representative pointer is absent, not zero.
    pub representative: Option<u32>,
    pub provinces_start: usize,
    pub provinces_len: usize,
}
