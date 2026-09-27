//! The writers of the relation opinion `CRelationStatus+0x300` for
//! hash-pinned a729. Observing them tells the collector which statuses to
//! re-read (`crelation_status::Baseline`); recording never calls them.
//!
//! Proof (hoi4re, instruction level; recorded in `captures/hooks/writers.md`,
//! "Relation opinions"):
//! - Direct stores: every instruction with a displacement in `0x2E0..=0x303`
//!   (5,899) was listed. 272 stores cover `+0x300..+0x303` at any width,
//!   including 16-byte stores at `+0x2F0`/`+0x2F8`. 186 of them go to the
//!   stack, and the remaining 86, in 79 functions, were each typed: only
//!   `RecomputeOpinion` (`MOV [RDI+0x300],ECX` at `140D191F5`), `Reset`
//!   (0 at `140D0A327`) and the constructor (0 at `140D03661`) store to a
//!   status.
//! - Addresses: of the 124 `LEA`/`ADD +0x300` sites, only `LoadToken` takes a
//!   status field's address; token `0x2D4E` passes it to the signed-integer
//!   reader `1424AB970`.
//! - Derived pointers and bulk writes: 22 stores through `base+K` reach
//!   `+0x300`, none on a status. None of the 89 constant-length
//!   `memset`/`memcpy` calls and 83 `rep` string operations that could cover
//!   it targets a status. No member embedded in a status spans `+0x300`, so
//!   the 995 callee escapes of interior pointers cannot reach it.
//! - Coverage: the 99 function ranges Ghidra lacks were disassembled, and
//!   `.text` was byte-searched for the displacements; the only extra write is
//!   a stack store.
//! - Lifetime: statuses are allocated only by `CDiplomacyStatus::constructor`
//!   `140D1E540` at `CGameState::reset`, freed only in `~CDiplomacyStatus`
//!   (`140D1EE00`), and never copied whole.
//!
//! `RecomputeOpinion` runs on `CGameState::DailyUpdate` TBB workers (the daily
//! sweep `1406DB660`) as well as the main thread, and it stores unconditionally;
//! a detour must be thread-safe and should mark only a changed value. `Reset`
//! is reachable at runtime (civil war, annexation, state transfer, dynamic
//! country reuse). `LoadToken` and the constructor run only while a game state
//! is loaded or rebuilt.

pub struct MutationSite {
    pub name: &'static str,
    pub rva: usize,
    pub sentinel: [u8; 16],
}

/// Receivers (rcx): a live `CRelationStatus*` for all but the constructor,
/// whose receiver is its raw 0x9C0-byte storage. None takes more than four
/// integer arguments or any floating-point argument.
pub const MUTATIONS: [MutationSite; 4] = [
    // CRelationStatus::RecomputeOpinion(status)
    MutationSite {
        name: "recompute_opinion",
        rva: 0xd19090,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x30, 0x80, 0x3d, 0x51, 0xfb,
            0x8a, 0x02,
        ],
    },
    // CRelationStatus::Reset(status, char, char, int*)
    MutationSite {
        name: "reset",
        rva: 0xd09d50,
        sentinel: [
            0x44, 0x88, 0x44, 0x24, 0x18, 0x88, 0x54, 0x24, 0x10, 0x48, 0x89, 0x4c, 0x24, 0x08,
            0x55, 0x53,
        ],
    },
    // CRelationStatus::LoadToken(status, reader, token)
    MutationSite {
        name: "load_token",
        rva: 0xd16ae0,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x08, 0x56, 0x57, 0x41, 0x56, 0x48, 0x83, 0xec, 0x50, 0x41,
            0x8b, 0xd8,
        ],
    },
    // CRelationStatus::constructor(storage, diplomacy, target*)
    MutationSite {
        name: "construct",
        rva: crate::crelation_status::CONSTRUCTOR_RVA,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x6c, 0x24, 0x20, 0x48, 0x89, 0x4c, 0x24,
            0x08, 0x56,
        ],
    },
];
