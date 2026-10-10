//! Map-icon manager base layout and per-type pool storage.
//! Visible lists and update watermarks describe presentation, not hourly state.

/// CMapIconManager<34> base vtable.
pub const VTABLE_RVA: usize = 0x0293_57c8;
/// CMapIconManager<34> type count.
pub const ICON_TYPE_COUNT: usize = 34;
/// CMapIconManager<34> per-type CPdxArray pools.
pub const POOLS: usize = 0x08;
/// CMapIconManager<34> per-type live CPdxArrays.
pub const LIVE_LISTS: usize = 0x338;
/// CMapIconManager<34> CPdxArray element stride.
pub const LIST_STRIDE: usize = 0x18;
/// CMapIconManager<34> round-robin u32 cursors.
pub const POOL_CURSORS: usize = 0x668;
/// CMapIconManager<34> cursor stride.
pub const POOL_CURSOR_STRIDE: usize = 4;
/// CMapIconManager<34> per-pass visible CPdxArray.
pub const VISIBLE_LIST: usize = 0x6f0;
/// CMapIconManager<34> container name pointer.
pub const CONTAINER_NAME: usize = 0x728;
/// CMapIconManager<34> u32 update index.
pub const UPDATE_INDEX: usize = 0x738;
/// CMapIconManager<34> refcounted handle.
pub const SHARED_HANDLE: usize = 0x740;
/// CMapIconManager<34> SRWLOCK.
pub const LOCK: usize = 0x748;

/// Collection pointers visited before updating icon live lists.
pub const GROUP_COLLECTIONS: usize = 0x708;
