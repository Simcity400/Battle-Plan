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
//!   erase, and by the add-advisor command. The generic insert reaches this
//!   pool only through those two. Status updates have
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
//!   the sorted insert only by registration. Only
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
use engine_sites::Rva;

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

const DESTROY_POOL_RVA: Rva = Rva(0x1170690);
const UNAPPOINT_ADVISOR_RVA: Rva = Rva(0x1172b90);

/// Every site with its receiver; each returns RAX unchanged from its
/// original, so a detour returns the original's value.
pub const MUTATIONS: [(MutationSite, Receiver); 15] = [
    // Advisor-status update (pool, status record). The stack-pointer copy to RAX
    // and the RBX store through it to RCX's home slot relocate together in seven
    // bytes.
    (
        MutationSite {
            name: "update_advisor_status",
            rva: Rva(0x1176330),
        },
        Receiver::Pool,
    ),
    // Add-advisor command (pool, character key, template): saves RBP/RSI/RDI/R14.
    (
        MutationSite {
            name: "add_advisor",
            rva: Rva(0x1170bc0),
        },
        Receiver::Pool,
    ),
    // Retire (pool, character, record, flags) -> bool.
    (
        MutationSite {
            name: "retire_character",
            rva: Rva(0x1175260),
        },
        Receiver::Pool,
    ),
    // Civil-war transfer (source pool, list, destination party).
    (
        MutationSite {
            name: "transfer_characters",
            rva: Rva(0x1175a10),
        },
        Receiver::Pool,
    ),
    // Advisor-role removal effect (effect, scope).
    (
        MutationSite {
            name: "remove_advisor_role",
            rva: Rva(0x374f80),
        },
        Receiver::Unnamed,
    ),
    // Country character pool construction (pool, country) -> pool.
    (
        MutationSite {
            name: "construct_pool",
            rva: Rva(0x1170440),
        },
        Receiver::Pool,
    ),
    // Country character pool destruction (pool).
    (
        MutationSite {
            name: "destroy_pool",
            rva: DESTROY_POOL_RVA,
        },
        Receiver::Pool,
    ),
    // Token set update (manager, character, erase list, insert list).
    (
        MutationSite {
            name: "update_advisor_tokens",
            rva: Rva(0x702600),
        },
        Receiver::Tokens,
    ),
    // Character-manager clear (manager, flags).
    (
        MutationSite {
            name: "clear_character_manager",
            rva: Rva(0x7016f0),
        },
        Receiver::Tokens,
    ),
    // Character-manager construction (manager): saves RBX and reserves 0x20 stack bytes.
    (
        MutationSite {
            name: "construct_character_manager",
            rva: Rva(0x6fc660),
        },
        Receiver::Tokens,
    ),
    // Character manager destruction (manager, flags).
    (
        MutationSite {
            name: "destroy_character_manager",
            rva: Rva(0x6fc920),
        },
        Receiver::Tokens,
    ),
    // Advisor appointment (pool, advisor).
    (
        MutationSite {
            name: "insert_appointed_advisor",
            rva: Rva(0x1171600),
        },
        Receiver::Pool,
    ),
    // Character registration (pool, character, flag): stores RBX in RCX's home slot,
    // then saves RBP/RSI/RDI/R12..R15.
    (
        MutationSite {
            name: "register_character",
            rva: Rva(0x1171070),
        },
        Receiver::Pool,
    ),
    // Civil-war retired-record copy (destination pool, source pool): saves
    // RBP/RDI/R13 and reserves 0x30 stack bytes.
    (
        MutationSite {
            name: "copy_retired_characters",
            rva: Rva(0x1172250),
        },
        Receiver::Pool,
    ),
    // Advisor unappointment (pool, advisor, flag).
    (
        MutationSite {
            name: "unappoint_advisor",
            rva: UNAPPOINT_ADVISOR_RVA,
        },
        Receiver::Pool,
    ),
];
