//! CMapIconManagerImpl presentation state; membership depends on the
//! live camera and the census is excluded from recorded state.

/// CMapIconManagerImpl singleton.
pub const SINGLETON_RVA: usize = 0x0332_3638;
/// CMapIconManagerImpl allocation size.
pub const SIZE: usize = 0x7a0;
/// CMapIconManagerImpl vtable.
pub const VTABLE_RVA: usize = 0x0293_5800;
/// CMapIconManagerImpl factory.
pub const CREATE_RVA: usize = 0x00b6_29c0;
/// CMapIconManagerImpl constructor.
pub const CONSTRUCT_RVA: usize = 0x00b6_2190;
/// CMapIconManagerImpl render pass.
pub const UPDATE_RVA: usize = 0x00b2_f270;
/// Per-type icon allocator, using a lock and reusable pool storage.
pub const ALLOCATE_ICON_RVA: usize = 0x00b6_3880;
/// Map-icon erase path compacts the live list by swapping.
pub const ERASE_ICON_RVA: usize = 0x00b2_ce50;
/// CMapIconManagerImpl visible-list sort.
pub const SORT_VISIBLE_RVA: usize = 0x00b2_bef0;
/// CMapIconManagerImpl factory dispatch.
pub const CREATE_TYPED_ICON_RVA: usize = 0x00b6_2a20;
/// CMapIconManagerImpl capital wrapper.
pub const CREATE_CAPITAL_ICON_RVA: usize = 0x00b6_3b70;
/// CMapIconManagerImpl raid wrapper.
pub const CREATE_RAID_ICON_RVA: usize = 0x00b6_3f50;
/// CMapIconManagerImpl resource wrapper.
pub const CREATE_RESOURCE_ICON_RVA: usize = 0x00b6_4030;
