//! CInGameIdler, the in-game application owner, distinct from CGameState.
//!
//! In-game activation publishes both owner globals; frontend activation sets
//! only the first. Pause-writer coverage does not establish every indirect write.

use engine_sites::Rva;
/// Active application-idler pointer. In-game activation fills both roots;
/// frontend activation fills only this root.
pub const SINGLETON_A_RVA: Rva = Rva(0x033d_db08);
/// In-game idler primary vtable.
pub const VTABLE_RVA: Rva = Rva(0x029e_58d8);

/// CInGameIdler update.
pub const UPDATE_RVA: Rva = Rva(0x00e4_1670);
/// CInGameIdler map delegate pointer.
pub const MAP_DELEGATE: usize = 0x530;
/// The map delegate's CGraphicalMap pointer.
pub const GRAPHICAL_MAP: usize = 0x370;
/// In-game idler paused byte, requiring zero or one.
pub const PAUSED: usize = 0x6c1;
/// Pause-kind byte, separate from paused state.
pub const PAUSE_KIND: usize = 0x6c2;
/// Load/setup function also writes paused state.
pub const LOAD_SETUP_RVA: Rva = Rva(0x00e4_7560);
