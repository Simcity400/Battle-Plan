//! The runtime writers of ScriptVariables stores: seed (`cvariables::SEED`);
//! Robin Hood hash table (`TABLE`) with node storage, count, mask and probe
//! tail (`cvariables::STORAGE`, `ENTRY_COUNT`, `BUCKET_MASK`, `PROBE_TAIL`);
//! and node key string and value (`variable_hash_node::KEY`, `VALUE`). Recorded
//! scopes are global (`cgame_state::GLOBAL_VARIABLES`), country
//! (`ccountry::VARIABLES`), state (`cstate::VARIABLES`) and character
//! (`ccharacter::VARIABLES`), embedded. Observing writers tells the collector
//! which stores to reread; recording never calls them.
//!
//! Writer coverage includes relative calls, jumps and eight-byte pointers to table
//! operations, vtable and empty-storage uses, inline clears and 0x30-stride
//! probes:
//! - Find-or-insert returns a node value through lookup, two insertion paths,
//!   growth and rehash; its caller writes the returned value. Only scoped
//!   mutable-value access passes a recorded store; seven other callers pass
//!   the thread-local store. Callers finish writing before returning. Scope
//!   resolution supplies recorded stores for set, add, subtract, multiply,
//!   divide, round, clamp and randomize effects, effect-builder helpers,
//!   array effects, entity creation and console variable setting. Temporary
//!   effects and triggers, iteration and find-highest triggers, scripted GUI
//!   updating and two AI paths instead use the thread-local temporary store,
//!   which is not recorded. Its dedicated setter writes only that store.
//! - Erase follows variable clear, array clear, array removal and resize,
//!   plus the temporary-array path.
//! - Clear also resets the thread-local store in every outermost context.
//!   Only its three recorded-store calls are observed: country appearance
//!   reset during dynamic reuse, state reset and game runtime reset.
//! - Save loading inserts or overwrites values and loads the seed. On token
//!   0x2A4A, the character parser replaces the embedded store inline before
//!   calling the reader. Only save or database character loading reaches
//!   this path, covered by world-lifetime invalidation.
//! - Store destruction, another destruction path, character destruction and
//!   thread exit all end in hash-table release (the store's `TABLE`).
//!   Construction writes only a new empty header and seed. Callers are
//!   game-state, country, state and both character constructors, raids,
//!   projects, industrial organisations and that parser. Recorded stores are
//!   created only with their owners.
//! - Seed setting runs in those constructors and immediately after each of the
//!   three observed resets' clears, within the same main-thread call. Scoped
//!   random-seed begin sets the active byte, change byte and store pointer
//!   (`CONTEXT_ACTIVE`, `CONTEXT_CHANGE_SEED`, `CONTEXT_STORE`). Context end
//!   rewrites the store's two seed words (`cvariables::SEED`) only when both
//!   bytes are set and the thread's random seed moved. Of 68 begin calls, 61
//!   pass zero change in R8D and seven pass one in R8B. Those functions build
//!   stack contexts ended by the eight [`Patch::EndContext`] sites in
//!   [`MUTATIONS`]. Begin alone writes the change byte; the other 71 end calls,
//!   including trigger checking, never write a recorded seed and stay
//!   unpatched. Context end also runs during exception unwinding: all seven
//!   functions' C++ exception maps name it as an object destructor, and 24
//!   unwind funclets jump to it. Such an exception ends a context without an
//!   observed call, so the collector moves its variable structure token at
//!   every first-chance C++ exception. Version 3 and 4 C++ handler-map formats
//!   name end in 41 functions; no unwind action or funclet reaches clear.
//! - Nine other table functions only read, including save writing.

pub use crate::advisor_mutations::MutationSite;

/// How a site's arguments name the store.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Receiver {
    /// RCX is the variable store.
    First,
    /// RDX is the variable store.
    Second,
    /// RCX is the store's hash table, at `TABLE`.
    Table,
    /// RCX is a scripted context; [`context_store`] reads the store whose
    /// seed it may rewrite.
    Context,
}

/// Bytes from a store to its hash table, the release receiver.
pub const TABLE: usize = 0x10;

/// Store clear invoked by [`Patch::Call`] sites.
pub const CLEAR_RVA: usize = 0xc1_b950;

/// Context end invoked by [`Patch::EndContext`] sites.
pub const END_CONTEXT_RVA: usize = 0x58_9510;

/// Context active byte, stored at begin and tested at end.
pub const CONTEXT_ACTIVE: usize = 0;
/// Context change-seed byte: begin's third argument, tested at end.
pub const CONTEXT_CHANGE_SEED: usize = 1;
/// Context store pointer, tested at end and used to write its store's seed.
pub const CONTEXT_STORE: usize = 8;

/// Game static TLS index used to locate the calling thread's temporary store:
/// the thread's TLS block at this index in the thread's TLS pointer array, with
/// the store at `TEMPORARY_STORE`.
pub const TLS_INDEX_RVA: usize = 0x369_3a98;
/// The temporary store's offset in the game's TLS block.
pub const TEMPORARY_STORE: usize = 0x30;

/// Whether a site is a function entry or a five-byte CALL that the detour
/// replaces and makes itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Patch {
    Entry,
    /// A CALL of the clear [`CLEAR_RVA`].
    Call,
    /// A CALL of [`END_CONTEXT_RVA`] ending a context that may change its
    /// store's seed.
    EndContext,
}

/// A [`Patch::EndContext`] call site at `rva`, preceded by placing the
/// frame-local context address in RCX, where begin built it.
const fn end_context(
    name: &'static str,
    rva: usize,
    sentinel: [u8; 16],
) -> (MutationSite, Receiver, Patch) {
    (
        MutationSite {
            name,
            rva,
            sentinel,
        },
        Receiver::Context,
        Patch::EndContext,
    )
}

/// Every site with its receiver and patch; each returns RAX unchanged from
/// its original, so a detour returns the original's value.
pub const MUTATIONS: [(MutationSite, Receiver, Patch); 15] = [
    // Mutable-value access (variable, store, scope, fallback).
    (
        MutationSite {
            name: "get_mutable_value",
            rva: 0x58_9760,
            // The RBX spill to RDX's home slot occupies exactly the five patched bytes.
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x6c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24,
                0x20, 0x57,
            ],
        },
        Receiver::Second,
        Patch::Entry,
    ),
    // Erase (store, key): the RSI spill to R8's home slot occupies five patched bytes.
    (
        MutationSite {
            name: "erase_variable",
            rva: 0xc1_e900,
            sentinel: [
                0x48, 0x89, 0x74, 0x24, 0x18, 0x57, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8d, 0x71, 0x10,
                0x4c, 0x8b,
            ],
        },
        Receiver::First,
        Patch::Entry,
    ),
    // Store save reader (store, reader): saving RBX and reserving 0x40 stack
    // bytes occupy six relocated bytes.
    (
        MutationSite {
            name: "read_variables",
            rva: 0xc1_e6b0,
            sentinel: [
                0x40, 0x53, 0x48, 0x83, 0xec, 0x40, 0x81, 0x7a, 0x30, 0xbb, 0x27, 0x00, 0x00, 0x48,
                0x8b, 0xda,
            ],
        },
        Receiver::First,
        Patch::Entry,
    ),
    // Release (store hash table): the RBX spill to R9's home slot occupies five
    // patched bytes.
    (
        MutationSite {
            name: "release_variables",
            rva: 0x59_1680,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x20, 0x55, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0x59, 0x08,
                0x48, 0x8d,
            ],
        },
        Receiver::Table,
        Patch::Entry,
    ),
    // Country appearance reset clears its variable store (`ccountry::VARIABLES`).
    (
        MutationSite {
            name: "reset_country_variables",
            rva: 0x72_abc0,
            sentinel: [
                0xe8, 0x8b, 0x0d, 0x4f, 0x00, 0x48, 0x8b, 0x9e, 0x18, 0x02, 0x00, 0x00, 0x44, 0x39,
                0x6e, 0x08,
            ],
        },
        Receiver::First,
        Patch::Call,
    ),
    // State reset clears its variable store (`cstate::VARIABLES`).
    (
        MutationSite {
            name: "reset_state_variables",
            rva: 0xa2_1bfa,
            sentinel: [
                0xe8, 0x51, 0x9d, 0x1f, 0x00, 0x8b, 0x55, 0x58, 0x48, 0x8b, 0x8d, 0xf8, 0x07, 0x00,
                0x00, 0xe8,
            ],
        },
        Receiver::First,
        Patch::Call,
    ),
    // Game runtime reset clears its global variable store
    // (`cgame_state::GLOBAL_VARIABLES`).
    (
        MutationSite {
            name: "reset_global_variables",
            rva: 0x1e_76e0,
            sentinel: [
                0xe8, 0x6b, 0x42, 0xa3, 0x00, 0x33, 0xd2, 0x48, 0x8b, 0x8b, 0x80, 0x09, 0x00, 0x00,
                0xe8, 0x9d,
            ],
        },
        Receiver::First,
        Patch::Call,
    ),
    // The registered console scripted-effect command has two context ends on
    // one stack-local context, following begin with change zero or change one.
    end_context(
        "end_effect_command",
        0x29_697c,
        [
            0xe8, 0x8f, 0x2b, 0x2f, 0x00, 0x90, 0x48, 0x8d, 0x4c, 0x24, 0x48, 0xe8, 0xa4, 0x9b,
            0xe8, 0xff,
        ],
    ),
    end_context(
        "end_effect_command_run",
        0x29_6a24,
        [
            0xe8, 0xe7, 0x2a, 0x2f, 0x00, 0x41, 0xc6, 0x04, 0x24, 0x01, 0x45, 0x84, 0xed, 0x0f,
            0x85, 0x3f,
        ],
    ),
    // Scope-validated effect execution: a frame-local context addressed through RBP.
    end_context(
        "end_effect_execute",
        0x58_4f79,
        [
            0xe8, 0x92, 0x45, 0x00, 0x00, 0xe9, 0x90, 0x04, 0x00, 0x00, 0x48, 0x8b, 0xcf, 0xe8,
            0x65, 0x9b,
        ],
    ),
    // Country daily update: a stack-local context.
    end_context(
        "end_country_daily",
        0x72_faf5,
        [
            0xe8, 0x16, 0x9a, 0xe5, 0xff, 0x90, 0x48, 0x8d, 0x4d, 0x40, 0xe8, 0x7c, 0xca, 0xe4,
            0xff, 0x48,
        ],
    ),
    // Country delayed events: a stack-local context.
    end_context(
        "end_delayed_events",
        0x74_482f,
        [
            0xe8, 0xdc, 0x4c, 0xe4, 0xff, 0x90, 0x48, 0x8d, 0x4c, 0x24, 0x60, 0xe8, 0x41, 0x7d,
            0xe3, 0xff,
        ],
    ),
    // State daily update, from game-state daily update: a stack-local context.
    end_context(
        "end_state_daily",
        0xa2_2159,
        [
            0xe8, 0xb2, 0x73, 0xb6, 0xff, 0x90, 0x48, 0x8d, 0x4c, 0x24, 0x60, 0xe8, 0x17, 0xa4,
            0xb5, 0xff,
        ],
    ),
    // Scripted-window effect execution: a stack-local context.
    end_context(
        "end_scripted_window",
        0x11e_082f,
        [
            0xe8, 0xdc, 0x8c, 0x3a, 0xff, 0x90, 0x48, 0x85, 0xf6, 0x74, 0x2f, 0x48, 0x8b, 0xfb,
            0x85, 0xdb,
        ],
    ),
    // Event-firing effect execution: a stack-local context.
    end_context(
        "end_fire_event",
        0x142_fbbf,
        [
            0xe8, 0x4c, 0x99, 0x15, 0xff, 0xe9, 0xdc, 0x01, 0x00, 0x00, 0x48, 0x8d, 0x8b, 0x40,
            0x07, 0x00,
        ],
    ),
];

/// The store a site's first two arguments name. A [`Receiver::Context`]
/// names its store only through memory ([`context_store`]); this returns the
/// context itself.
pub fn store(receiver: Receiver, first: usize, second: usize) -> usize {
    match receiver {
        Receiver::First | Receiver::Context => first,
        Receiver::Second => second,
        Receiver::Table => first.wrapping_sub(TABLE),
    }
}

/// The store whose seed context end may rewrite: `CONTEXT_STORE`, reached
/// only when active and change-seed bytes are set and the pointer is nonnull,
/// as the end routine tests. Return `Ok(None)` when no seed is written and
/// `Err` when the context does not read.
pub fn context_store(
    memory: &impl crate::Memory,
    context: usize,
) -> Result<Option<usize>, &'static str> {
    let bytes =
        crate::read::<[u8; 16]>(memory, context).ok_or("scripted variable context unreadable")?;
    if bytes[CONTEXT_ACTIVE] == 0 || bytes[CONTEXT_CHANGE_SEED] == 0 {
        return Ok(None);
    }
    let store = u64::from_le_bytes(
        bytes[CONTEXT_STORE..CONTEXT_STORE + 8]
            .try_into()
            .expect("eight bytes"),
    );
    Ok(usize::try_from(store).ok().filter(|&store| store != 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each sentinel starts with its relative call of clear or context end;
    /// only context-end sites take a context.
    #[test]
    fn call_sites_call_their_target() {
        let mut ends = 0;
        for (site, receiver, patch) in MUTATIONS {
            let target = match patch {
                Patch::Entry => continue,
                Patch::Call => CLEAR_RVA,
                Patch::EndContext => {
                    ends += 1;
                    END_CONTEXT_RVA
                }
            };
            assert_eq!(site.sentinel[0], 0xe8, "{}", site.name);
            let displacement = i32::from_le_bytes(site.sentinel[1..5].try_into().unwrap());
            assert_eq!(
                (site.rva + 5).wrapping_add_signed(displacement as isize),
                target,
                "{}",
                site.name
            );
            assert_eq!(
                receiver == Receiver::Context,
                patch == Patch::EndContext,
                "{}",
                site.name
            );
        }
        assert_eq!(ends, 8);
        let mut names: Vec<_> = MUTATIONS.iter().map(|(site, _, _)| site.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), MUTATIONS.len());
    }

    #[test]
    fn receivers_name_the_store() {
        assert_eq!(store(Receiver::First, 0x5000, 0x9000), 0x5000);
        assert_eq!(store(Receiver::Second, 0x5000, 0x9000), 0x9000);
        // Release receives the hash table, at `TABLE`.
        assert_eq!(store(Receiver::Table, 0x5010, 0), 0x5000);
    }

    /// Context end writes a seed only for an active context with change set and a store.
    #[test]
    fn a_context_names_its_store_only_when_it_may_change_the_seed() {
        let context = |active: u8, change: u8, store: u64| {
            let mut bytes = vec![0_u8; 0x40];
            bytes[0x10 + CONTEXT_ACTIVE] = active;
            bytes[0x10 + CONTEXT_CHANGE_SEED] = change;
            bytes[0x10 + CONTEXT_STORE..][..8].copy_from_slice(&store.to_le_bytes());
            bytes
        };
        let at = |bytes: &[u8]| {
            let memory = crate::memory::SliceMemory { base: 0, bytes };
            context_store(&memory, 0x10)
        };
        assert_eq!(at(&context(1, 1, 0x9000)), Ok(Some(0x9000)));
        assert_eq!(at(&context(1, 0, 0x9000)), Ok(None));
        assert_eq!(at(&context(0, 1, 0x9000)), Ok(None));
        assert_eq!(at(&context(1, 1, 0)), Ok(None));
        assert!(at(&context(1, 1, 0x9000)[..0x18]).is_err());
    }
}
