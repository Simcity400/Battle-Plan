//! The runtime writers of scripted-flag managers recorded by ScriptFlags
//! for global, country, state and character scopes. Managers hold dense
//! rows, capacity, count and allocator (`cflag_manager::ROWS`,
//! `SLOT_COUNT`, `ENTRY_COUNT`, `ALLOCATOR`); each row holds token, set
//! hour, value and remaining days (`cscript_flag::TOKEN_ID`, `SET_HOUR`,
//! `VALUE`, `REMAINING_DAYS`). Observing writers tells the collector which
//! managers to reread; recording never calls them.
//!
//! Writer coverage:
//! - Flag rows are built only by get-or-create, row-array assignment,
//!   row-array move (from get-or-create growth and project copy), and three
//!   project-only paths. Only the constructors below and project construction
//!   install a manager vtable. Other flag implementation routines only read:
//!   checksum and manager/row save writers.
//! - Get-or-create finds or appends a row, stamps its hour from game state
//!   (`cgame_state::CURRENT_HOUR`) and reallocates through the allocator when
//!   full. Its only three callers name the manager and write the returned row
//!   before returning, so their sites cover both operations. Value/duration
//!   setting writes value and positive duration for all scope set/modify flag
//!   effects and three indirect console callbacks. Playthrough flag setting
//!   handles career-profile flags. Manager save loading reads each returned
//!   row.
//! - Only indexed row removal removes rows, from daily expiry and clear-flag
//!   effects. Daily expiry also decrements positive remaining days inline,
//!   writing nothing else; a manager with none is untouched. Callers are
//!   daily updates of game state, countries, states and industrial
//!   organisations through daily production.
//! - Full clear destroys every row and zeroes the count during appearance
//!   reset for dynamic country reuse (`ccountry::FLAG_MANAGER`), state reset
//!   (`cstate::FLAGS`) and game runtime reset (`cgame_state::GLOBAL_FLAGS`).
//! - Row-array assignment takes the manager's row array (`ROWS`), rows and
//!   count and replaces them at load start, character bulk copy into a new
//!   character's embedded manager's row array and project move assignment.
//! - Construction starts a manager empty: country and state heap managers,
//!   character embedded manager (`ccharacter::FLAGS`) and the game state's
//!   manager. Embedded release frees rows during character role-tree
//!   destruction; virtual destruction frees heap-manager rows.
//! - Country-flag lookup returns a row pointer. Triggers and four other
//!   readers only read it; modify effects then call value/duration setting.
//!   No other routine walks manager rows from the scope roots (country,
//!   state, game and character managers) with a 0x30 stride.

pub use crate::advisor_mutations::MutationSite;

/// How a site's first argument (RCX) names the manager.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Receiver {
    /// RCX is the flag manager.
    Manager,
    /// RCX is the manager's row array, at `ROWS`.
    Rows,
}

/// Bytes from a manager to its row array, the row-assignment receiver.
pub const ROWS: usize = 0x8;

/// Every site with its receiver; each returns RAX unchanged from its
/// original, so a detour returns the original's value.
pub const MUTATIONS: [(MutationSite, Receiver); 10] = [
    // Flag value/duration assignment (manager, token, value, duration).
    (
        MutationSite {
            name: "set_flag",
            rva: 0xd1fd20,
            // The RBX spill to RCX's home slot occupies exactly the five patched bytes.
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x41, 0x0f, 0xb7, 0xf9,
                0x41, 0x8b,
            ],
        },
        Receiver::Manager,
    ),
    // Playthrough flag set (manager, token, value, duration).
    (
        MutationSite {
            name: "set_playthrough_flag",
            rva: 0xd1fd50,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x41, 0x0f, 0xb7, 0xf9,
                0x41, 0x0f,
            ],
        },
        Receiver::Manager,
    ),
    // Flag-manager save reader (manager, reader).
    (
        MutationSite {
            name: "read_flags",
            rva: 0xd1fa00,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x7c, 0x24,
                0x20, 0x55,
            ],
        },
        Receiver::Manager,
    ),
    // Daily expiry (manager).
    (
        MutationSite {
            name: "daily_flags",
            rva: 0xd1f910,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x18, 0x55, 0x48, 0x83, 0xec, 0x30, 0x48, 0x8b, 0xe9, 0x4c,
                0x8d, 0x05,
            ],
        },
        Receiver::Manager,
    ),
    // Clear a flag (manager, token): a frameless leaf tail-jumps to indexed removal.
    // The sign-extended count load (`cflag_manager::ENTRY_COUNT`) into RAX and
    // zeroing R8D occupy seven relocated bytes.
    (
        MutationSite {
            name: "clear_flag",
            rva: 0xd1fce0,
            sentinel: [
                0x48, 0x63, 0x41, 0x14, 0x45, 0x33, 0xc0, 0x85, 0xc0, 0x7e, 0x34, 0x4c, 0x8d, 0x51,
                0x08, 0x4c,
            ],
        },
        Receiver::Manager,
    ),
    // Clear every row (manager).
    (
        MutationSite {
            name: "clear_flags",
            rva: 0xd1f4f0,
            sentinel: [
                0x48, 0x89, 0x74, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec, 0x20, 0x48, 0x63, 0x71, 0x14,
                0x48, 0x8b,
            ],
        },
        Receiver::Manager,
    ),
    // Row array assignment (the manager's row array, rows, count).
    (
        MutationSite {
            name: "assign_flags",
            rva: 0x1e2b90,
            sentinel: [
                0x44, 0x89, 0x44, 0x24, 0x18, 0x48, 0x89, 0x54, 0x24, 0x10, 0x56, 0x57, 0x48, 0x83,
                0xec, 0x48,
            ],
        },
        Receiver::Rows,
    ),
    // Construction (manager): saving RBX and reserving 0x20 stack bytes occupy
    // six relocated bytes.
    (
        MutationSite {
            name: "construct_flags",
            rva: 0xd1f320,
            sentinel: [
                0x40, 0x53, 0x48, 0x83, 0xec, 0x20, 0x8b, 0x15, 0x6c, 0x47, 0x97, 0x02, 0x48, 0x8d,
                0x05, 0x65,
            ],
        },
        Receiver::Manager,
    ),
    // Embedded manager release (manager).
    (
        MutationSite {
            name: "release_flags",
            rva: 0xd1f3b0,
            sentinel: [
                0x40, 0x53, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8d, 0x05, 0xdb, 0xa9, 0xcb, 0x01, 0x48,
                0x8d, 0x59,
            ],
        },
        Receiver::Manager,
    ),
    // Virtual destructor (manager, flags).
    (
        MutationSite {
            name: "destroy_flags",
            rva: 0xd1f410,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec,
                0x20, 0x48,
            ],
        },
        Receiver::Manager,
    ),
];

/// The manager a site's first argument names.
pub fn manager(receiver: Receiver, argument: usize) -> usize {
    match receiver {
        Receiver::Manager => argument,
        Receiver::Rows => argument.wrapping_sub(ROWS),
    }
}
