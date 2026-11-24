
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

