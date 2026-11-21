
pub use crate::advisor_mutations::MutationSite;
use engine_sites::Rva;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Receiver {
    Party,
    Parties,
    Vector,
}

pub const VECTOR: usize = 0x70;

pub const MUTATIONS: [(MutationSite, Receiver); 11] = [
    (
        MutationSite {
            name: "insert_leader_front",
            rva: Rva(0x1230590),
        },
        Receiver::Party,
    ),
    (
        MutationSite {
            name: "append_leader",
            rva: Rva(0x1230620),
        },
        Receiver::Party,
    ),
    (
        MutationSite {
            name: "remove_leader",
            rva: Rva(0x1232b80),
        },
        Receiver::Party,
    ),
    (
        MutationSite {
            name: "promote_leader",
            rva: Rva(0x12324a0),
        },
        Receiver::Party,
    ),
    (
        MutationSite {
            name: "retire_first_leader",
            rva: Rva(0x1232c40),
        },
        Receiver::Party,
    ),
    (
        MutationSite {
            name: "kill_first_leader",
            rva: Rva(0x1232410),
        },
        Receiver::Party,
    ),
    (
        MutationSite {
            name: "clear_leaders",
            rva: Rva(0x1232230),
        },
        Receiver::Party,
    ),
    (
        MutationSite {
            name: "transfer_leaders",
            rva: Rva(0x1233620),
        },
        Receiver::Parties,
    ),
    (
        MutationSite {
            name: "validate_leaders",
            rva: Rva(0x1232140),
        },
        Receiver::Party,
    ),
    (
        MutationSite {
            name: "read_leaders",
            rva: Rva(0x1232590),
        },
        Receiver::Vector,
    ),
    (
        MutationSite {
            name: "destroy_party",
            rva: Rva(0x1230450),
        },
        Receiver::Party,
    ),
];
