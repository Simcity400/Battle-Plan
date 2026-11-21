
pub use crate::advisor_mutations::MutationSite;
use engine_sites::Rva;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Domain {
    Roster,
    Scientists,
}

pub const PUSH: usize = 0;
pub const CLEAR: usize = 1;
pub const SCIENTIST_BUILD: usize = 2;
pub const SCIENTIST_ATTACH: usize = 3;
pub const SCIENTIST_CLEAR: usize = 4;
pub const SCIENTIST_TRAIT_ADD: usize = 5;
pub const SCIENTIST_TRAIT_REMOVE: usize = 6;
pub const SCIENTIST_TRAIT_RESET: usize = 7;

pub const MUTATIONS: [(MutationSite, Domain); 8] = [
    (
        MutationSite {
            name: "roster_push",
            rva: Rva(0x6f_a550),
        },
        Domain::Roster,
    ),
    (
        MutationSite {
            name: "roster_clear",
            rva: Rva(0x70_2520),
        },
        Domain::Roster,
    ),
    (
        MutationSite {
            name: "scientist_build",
            rva: Rva(0x14a_7530),
        },
        Domain::Scientists,
    ),
    (
        MutationSite {
            name: "scientist_attach",
            rva: Rva(0x102_7600),
        },
        Domain::Scientists,
    ),
    (
        MutationSite {
            name: "scientist_clear",
            rva: Rva(0x102_a640),
        },
        Domain::Scientists,
    ),
    (
        MutationSite {
            name: "scientist_trait_add",
            rva: Rva(0x14a_6890),
        },
        Domain::Scientists,
    ),
    (
        MutationSite {
            name: "scientist_trait_remove",
            rva: Rva(0x14a_8c90),
        },
        Domain::Scientists,
    ),
    (
        MutationSite {
            name: "scientist_trait_reset",
            rva: Rva(0x14a_8a20),
        },
        Domain::Scientists,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sites_are_distinct_and_their_indices_name_them() {
        for (index, (site, _)) in MUTATIONS.iter().enumerate() {
            assert!(
                MUTATIONS[..index]
                    .iter()
                    .all(|(other, _)| other.rva != site.rva && other.name != site.name)
            );
        }
        for (index, name) in [
            (PUSH, "roster_push"),
            (CLEAR, "roster_clear"),
            (SCIENTIST_BUILD, "scientist_build"),
            (SCIENTIST_ATTACH, "scientist_attach"),
            (SCIENTIST_CLEAR, "scientist_clear"),
            (SCIENTIST_TRAIT_ADD, "scientist_trait_add"),
            (SCIENTIST_TRAIT_REMOVE, "scientist_trait_remove"),
            (SCIENTIST_TRAIT_RESET, "scientist_trait_reset"),
        ] {
            assert_eq!(MUTATIONS[index].0.name, name);
        }
        for (site, _) in &MUTATIONS {
            for other in crate::advisor_mutations::MUTATIONS
                .iter()
                .chain(&crate::character_role_mutations::MUTATIONS)
            {
                assert_ne!(site.rva, other.rva);
            }
        }
    }
}
