//! CInGameIdler, the in-game application owner, distinct from CGameState.
//!
//! In-game activation publishes both owner globals; frontend activation sets
//! only the first. Pause-writer coverage does not establish every indirect write.

/// Active application-idler pointer. In-game activation fills both roots;
/// frontend activation fills only this root.
pub const SINGLETON_A_RVA: usize = 0x0332_f698;
/// In-game idler primary vtable.
pub const VTABLE_RVA: usize = 0x0296_8ff0;

/// CInGameIdler update.
pub const UPDATE_RVA: usize = 0x00dd_3a50;
/// CInGameIdler map delegate pointer.
pub const MAP_DELEGATE: usize = 0x530;
/// The map delegate's CGraphicalMap pointer.
pub const GRAPHICAL_MAP: usize = 0x370;
/// In-game idler paused byte, requiring zero or one.
pub const PAUSED: usize = 0x6c1;
/// Pause-kind byte, separate from paused state.
pub const PAUSE_KIND: usize = 0x6c2;
/// Load/setup function also writes paused state.
pub const LOAD_SETUP_RVA: usize = 0x00dd_9b20;
