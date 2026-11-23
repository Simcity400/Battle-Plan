
pub use crate::advisor_mutations::MutationSite;
use engine_sites::Rva;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Receiver {
    First,
    Second,
    Table,
    Context,
}

pub const TABLE: usize = 0x10;

pub const CLEAR_RVA: Rva = Rva(0xc1_b9c0);

pub const END_CONTEXT_RVA: Rva = Rva(0x58_9580);

pub const CONTEXT_ACTIVE: usize = 0;
pub const CONTEXT_CHANGE_SEED: usize = 1;
pub const CONTEXT_STORE: usize = 8;

pub const TLS_INDEX_RVA: Rva = Rva(0x369_3a98);
pub const TEMPORARY_STORE: usize = 0x30;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Patch {
    Entry,
    Call,
    EndContext,
}

const fn end_context(name: &'static str, rva: Rva) -> (MutationSite, Receiver, Patch) {
    (
        MutationSite { name, rva },
        Receiver::Context,
        Patch::EndContext,
    )
}

pub const MUTATIONS: [(MutationSite, Receiver, Patch); 15] = [
    (
        MutationSite {
            name: "get_mutable_value",
            rva: Rva(0x58_97d0),
        },
        Receiver::Second,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "erase_variable",
            rva: Rva(0xc1_e970),
        },
        Receiver::First,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "read_variables",
            rva: Rva(0xc1_e720),
        },
        Receiver::First,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "release_variables",
            rva: Rva(0x59_16f0),
        },
        Receiver::Table,
        Patch::Entry,
    ),
    (
        MutationSite {
            name: "reset_country_variables",
            rva: Rva(0x72_ac30),
        },
        Receiver::First,
        Patch::Call,
    ),
    (
        MutationSite {
            name: "reset_state_variables",
            rva: Rva(0xa2_1c6a),
        },
        Receiver::First,
        Patch::Call,
    ),
    (
        MutationSite {
            name: "reset_global_variables",
            rva: Rva(0x1e_76e0),
        },
        Receiver::First,
        Patch::Call,
    ),


    end_context("end_effect_command", Rva(0x29_69ec)),
    end_context("end_effect_command_run", Rva(0x29_6a94)),
    end_context("end_effect_execute", Rva(0x58_4fe9)),
    end_context("end_country_daily", Rva(0x72_fb65)),
    end_context("end_delayed_events", Rva(0x74_489f)),
    end_context("end_state_daily", Rva(0xa2_21c9)),
    end_context("end_scripted_window", Rva(0x11e_089f)),
    end_context("end_fire_event", Rva(0x142_fc2f)),
];

pub fn store(receiver: Receiver, first: usize, second: usize) -> usize {
    match receiver {
        Receiver::First | Receiver::Context => first,
        Receiver::Second => second,
        Receiver::Table => first.wrapping_sub(TABLE),
    }
}

pub fn context_store(
    memory: &impl engine_native_memory::Memory,
    context: usize,
) -> Result<Option<usize>, &'static str> {
    let bytes =
        engine_native_memory::read::<[u8; 16]>(memory, context).ok_or("scripted variable context unreadable")?;
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

    #[test]
    fn only_context_end_sites_take_a_context() {
        let mut ends = 0;
        for (site, receiver, patch) in MUTATIONS {
            if patch == Patch::EndContext {
                ends += 1;
            }
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
        assert_eq!(store(Receiver::Table, 0x5010, 0), 0x5000);
    }

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
            let memory = engine_native_memory::memory::SliceMemory { base: 0, bytes };
            context_store(&memory, 0x10)
        };
        assert_eq!(at(&context(1, 1, 0x9000)), Ok(Some(0x9000)));
        assert_eq!(at(&context(1, 0, 0x9000)), Ok(None));
        assert_eq!(at(&context(0, 1, 0x9000)), Ok(None));
        assert_eq!(at(&context(1, 1, 0)), Ok(None));
        assert!(at(&context(1, 1, 0x9000)[..0x18]).is_err());
    }
}
