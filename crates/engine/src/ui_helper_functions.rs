//! Helper metadata used by label and color updaters.
//! These caller relationships do not establish receiver ownership.

/// Current-phase getter returning an i32 pointer; value eight suppresses text.
pub const GAME_PHASE_CANDIDATE_RVA: usize = 0x00df_f0b0;
/// Localized-name helper taking a subject and destination string; subject ownership is unestablished.
pub const LOCALIZED_DISPLAY_NAME_CANDIDATE_RVA: usize = 0x00e6_9320;
/// Heap-string deallocation helper.
pub const DEALLOCATE_CANDIDATE_RVA: usize = 0x021b_4b60;
/// Assertion reporter whose output is checked before the debugger trap.
pub const ASSERT_REPORT_CANDIDATE_RVA: usize = 0x024b_3120;
/// Thread-local game-state guard used when TLS +0x860 is zero, before checking forbid count +0x10.
pub const THREAD_GAMESTATE_GUARD_CANDIDATE_RVA: usize = 0x0250_57d4;
/// Nonreturning allocation failure for invalid string allocation headers.
pub const ALLOCATION_CHECK_FAILURE_CANDIDATE_RVA: usize = 0x0253_5234;
/// Byte comparison taking new/cached text pointers and length.
pub const MEMORY_COMPARE_CANDIDATE_RVA: usize = 0x0257_4d50;
