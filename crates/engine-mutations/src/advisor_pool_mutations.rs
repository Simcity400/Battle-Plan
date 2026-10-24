//! Runtime writers of each country's available-advisor array and count
//! (`ccountry_characters::AVAILABLE_ADVISORS`; AvailableAdvisors), appointed-advisor
//! array and count (`ccountry_characters::APPOINTED_ADVISORS`; HiredAdvisors), active-
//! and retired-character record arrays (`ccountry_characters::ACTIVE_CHARACTERS`,
//! `ccountry_characters::RETIRED_CHARACTERS`; CountryCharacters,
//! CountryRetiredCharacters), and the character manager's advisor idea-token set
//! (AdvisorTokens). Observing them selects pools to reread; recording never calls them.
//!
//! An AvailableAdvisors row also reads its character's id (advisor character
//! pointer `cadvisor::CHARACTER` to `ccharacter::ID`) and idea string
//! (`cadvisor::IDEA_TOKEN`). Only the advisor family's sites write those fields
//! ([`crate::advisor_mutations`]: advisor construction, destruction, idea changes
//! and save reading, and character parsing, merging and destruction). Each
//! notifies the character its role names, so carrying a row also requires its
//! character's certificate.
//!
//! - Available advisors are inserted by advisor-status updates, which also
//!   erase, and by the add-advisor and generate-advisor commands. The generic
//!   insert reaches this pool only through those three. Status updates have
//!   one caller, character-status processing, reached from adding an advisor
//!   role, recruitment, the pool's save reader and civil-war destination
//!   registration. Retirement (also reached from civil war) and civil-war
//!   transfer erase entries. Removing an advisor role erases inline from the
//!   scope character's country, which is not an argument; that hook names no
//!   pool and revokes every certificate.
//! - Pool construction and destruction run during country construction and
//!   dynamic reuse. Callers publish the new pool in the country's character-pool
//!   pointer (`ccountry::CHARACTERS`) after construction returns; the walk reads
//!   that pointer every hour.
//! - Appointed advisors are inserted only by the appointment helper,
//!   through country advisor activation, reached by the activation effect,
//!   add-advisor command and replace-advisor command. Only the unappointment
//!   helper erases them: from country advisor removal behind deactivation,
//!   remove-ideas, remove-advisor commands and advisor-role removal, and from
//!   another pool helper, retirement, civil-war transfer, advisor-status
//!   updates, unit-leader capture, the deferred queue, another advisor helper,
//!   the activation effect and removal of ideas with a trait. The save reader,
//!   constructor and destructor build and free the array; the save writer and
//!   country modifier recalculation only read it. Advisor fields (character,
//!   idea token and slot) belong to the advisor family, and character ids to roster
//!   identity repair. The unappointment helper, the last site below, is patched
//!   by the stability and research family, which forwards each call here.
//! - Character records (active and retired) contain a character pointer and four
//!   role-status bytes no table records, in 16 bytes. The ordinary insert is
//!   called only by save reading, retirement and the civil-war retired copy;
//!   the sorted insert only by registration and two generated-advisor
//!   registrations, whose only caller is the generate-advisor command. Only
//!   retirement erases active records, and registrations erase retired ones.
//!   Registration is reached by recruitment through character and unit-leader
//!   addition, setup's name-index rebuild, unit-leader addition (including
//!   daily exile-leader generation), civil-war transfer, country-leader and
//!   scientist creation, and scientist recruitment. Only civil-war start
//!   reaches the retired copy. The save reader fills only a pool constructed
//!   in the same load. Records sort by the character's name token
//!   (`ccharacter::NAME_TOKEN_ID`); after publication only roster identity
//!   repair writes it and the id recorded by rows.
//! - Token-set updates are the only runtime token writer, reached through four
//!   shared set helpers. Callers are advisor-status updates, the save reader
//!   and advisor-role removal. Only the manager's virtual load clear empties
//!   the set; construction and destruction build and free it. The three other
//!   callers of the shared intermediate helper operate on equipment graphics
//!   pools.

pub use crate::advisor_mutations::MutationSite;

/// How a site's first argument (RCX) names the watched object.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Receiver {
    /// RCX is a country character pool.
    Pool,
    /// RCX is the character manager that owns the token set.
    Tokens,
    /// The written pool is not an argument: every certificate is revoked.
    Unnamed,
}

const DESTROY_POOL_RVA: usize = 0x10e8c70;
const UNAPPOINT_ADVISOR_RVA: usize = 0x10eb5a0;

/// Every site with its receiver; each returns RAX unchanged from its
/// original, so a detour returns the original's value.
pub const MUTATIONS: [(MutationSite, Receiver); 16] = [
    // Advisor-status update (pool, status record). The stack-pointer copy to RAX
    // and the RBX store through it to RCX's home slot relocate together in seven
    // bytes.
    (
        MutationSite {
            name: "update_advisor_status",
            rva: 0x10ef4c0,
            sentinel: [
                0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x08, 0x48, 0x89, 0x50, 0x10, 0x55, 0x56, 0x57,
                0x41, 0x54,
            ],
        },
        Receiver::Pool,
    ),
    // Add-advisor command (pool, character key, template): saves RBP/RSI/RDI/R14.
    (
        MutationSite {
            name: "add_advisor",
            rva: 0x10e91a0,
            sentinel: [
                0x40, 0x55, 0x56, 0x57, 0x41, 0x56, 0x48, 0x8b, 0xec, 0x48, 0x83, 0xec, 0x58, 0x49,
                0x83, 0x78,
            ],
        },
        Receiver::Pool,
    ),
    // Generate-advisor command (pool, slot, template): saves RBP/RSI/R14.
    (
        MutationSite {
            name: "generate_advisor",
            rva: 0x10ea930,
            sentinel: [
                0x40, 0x55, 0x56, 0x41, 0x56, 0x41, 0x57, 0x48, 0x8b, 0xec, 0x48, 0x83, 0xec, 0x48,
                0x48, 0x83,
            ],
        },
        Receiver::Pool,
    ),
    // Retire (pool, character, record, flags) -> bool.
    (
        MutationSite {
            name: "retire_character",
            rva: 0x10ee3f0,
            sentinel: [
                0x44, 0x89, 0x4c, 0x24, 0x20, 0x4c, 0x89, 0x44, 0x24, 0x18, 0x48, 0x89, 0x54, 0x24,
                0x10, 0x55,
            ],
        },
        Receiver::Pool,
    ),
    // Civil-war transfer (source pool, list, destination party).
    (
        MutationSite {
            name: "transfer_characters",
            rva: 0x10eeba0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56,
                0x41, 0x57,
            ],
        },
        Receiver::Pool,
    ),
    // Advisor-role removal effect (effect, scope).
    (
        MutationSite {
            name: "remove_advisor_role",
            rva: 0x35c9b0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x7c, 0x24,
                0x20, 0x55,
            ],
        },
        Receiver::Unnamed,
    ),
    // Country character pool construction (pool, country) -> pool.
    (
        MutationSite {
            name: "construct_pool",
            rva: 0x10e8a20,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x4c, 0x24, 0x08, 0x55, 0x56, 0x57, 0x41,
                0x56, 0x41,
            ],
        },
        Receiver::Pool,
    ),
    // Country character pool destruction (pool).
    (
        MutationSite {
            name: "destroy_pool",
            rva: DESTROY_POOL_RVA,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24,
                0x18, 0x57,
            ],
        },
        Receiver::Pool,
    ),
    // Token set update (manager, character, erase list, insert list).
    (
        MutationSite {
            name: "update_advisor_tokens",
            rva: 0x6baac0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24,
                0x20, 0x57,
            ],
        },
        Receiver::Tokens,
    ),
    // Character-manager clear (manager, flags).
    (
        MutationSite {
            name: "clear_character_manager",
            rva: 0x6b9bb0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24,
                0x20, 0x57,
            ],
        },
        Receiver::Tokens,
    ),
    // Character-manager construction (manager): saves RBX and reserves 0x20 stack bytes.
    (
        MutationSite {
            name: "construct_character_manager",
            rva: 0x6b4b20,
            sentinel: [
                0x40, 0x53, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8d, 0x05, 0x03, 0x12, 0x13, 0x02, 0xc7,
                0x41, 0x08,
            ],
        },
        Receiver::Tokens,
    ),
    // Character manager destruction (manager, flags).
    (
        MutationSite {
            name: "destroy_character_manager",
            rva: 0x6b4de0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24,
                0x18, 0x57,
            ],
        },
        Receiver::Tokens,
    ),
    // Advisor appointment (pool, advisor).
    (
        MutationSite {
            name: "insert_appointed_advisor",
            rva: 0x10e9be0,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20, 0x55, 0x57, 0x41, 0x55,
                0x41, 0x56,
            ],
        },
        Receiver::Pool,
    ),
    // Character registration (pool, character, flag): stores RBX in RCX's home slot,
    // then saves RBP/RSI/RDI/R12..R15.
    (
        MutationSite {
            name: "register_character",
            rva: 0x10e9650,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56,
                0x41, 0x57,
            ],
        },
        Receiver::Pool,
    ),
    // Civil-war retired-record copy (destination pool, source pool): saves
    // RBP/RDI/R13 and reserves 0x30 stack bytes.
    (
        MutationSite {
            name: "copy_retired_characters",
            rva: 0x10ea830,
            sentinel: [
                0x40, 0x55, 0x57, 0x41, 0x55, 0x48, 0x83, 0xec, 0x30, 0x48, 0x8b, 0x7a, 0x28, 0x4c,
                0x8b, 0xe9,
            ],
        },
        Receiver::Pool,
    ),
    // Advisor unappointment (pool, advisor, flag).
    (
        MutationSite {
            name: "unappoint_advisor",
            rva: UNAPPOINT_ADVISOR_RVA,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x18, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56,
                0x41, 0x57,
            ],
        },
        Receiver::Pool,
    ),
];
