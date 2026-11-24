
pub use crate::advisor_mutations::MutationSite;
use engine_sites::Rva;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Patch {
    Entry,
    Call,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Domain {
    Structure,
    Popularity,
    Both,
}

impl Domain {
    pub const fn structure(self) -> bool {
        matches!(self, Self::Structure | Self::Both)
    }
    pub const fn popularity(self) -> bool {
        matches!(self, Self::Popularity | Self::Both)
    }
}

pub const SET_POPULARITY_RVA: Rva = Rva(0x123_3110);

pub const MUTATIONS: [(MutationSite, Patch, Domain); 9] = [
    (
        MutationSite {
            name: "set_capital",
            rva: Rva(0x75_8f50),
        },
        Patch::Entry,
        Domain::Structure,
    ),
    (
        MutationSite {
            name: "set_politics",
            rva: Rva(0xbf_b120),
        },
        Patch::Entry,
        Domain::Structure,
    ),
    (
        MutationSite {
            name: "force_ruling_party",
            rva: Rva(0xc0_0af0),
        },
        Patch::Entry,
        Domain::Both,
    ),
    (
        MutationSite {
            name: "set_ruling_party",
            rva: Rva(0xc0_1a30),
        },
        Patch::Entry,
        Domain::Structure,
    ),
    (
        MutationSite {
            name: "set_ideas",
            rva: Rva(0xc0_23f0),
        },
        Patch::Entry,
        Domain::Structure,
    ),
    (
        MutationSite {
            name: "add_popularity",
            rva: Rva(0x123_0770),
        },
        Patch::Entry,
        Domain::Popularity,
    ),
    (
        MutationSite {
            name: "set_popularity_redistributing",
            rva: Rva(0x123_2fe0),
        },
        Patch::Entry,
        Domain::Popularity,
    ),
    (
        MutationSite {
            name: "set_political_party_popularity",
            rva: Rva(0x37_c50d),
        },
        Patch::Call,
        Domain::Popularity,
    ),
    (
        MutationSite {
            name: "set_popularities",
            rva: Rva(0x37_c847),
        },
        Patch::Call,
        Domain::Popularity,
    ),
];

pub const FORWARDED: [&str; 2] = ["reset_country", "politics_daily"];

