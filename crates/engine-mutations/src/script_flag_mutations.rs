
pub use crate::advisor_mutations::MutationSite;
use engine_sites::Rva;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Receiver {
    Manager,
    Rows,
}

pub const ROWS: usize = 0x8;

pub const MUTATIONS: [(MutationSite, Receiver); 10] = [
    (
        MutationSite {
            name: "set_flag",
            rva: Rva(0xd1fd20),
        },
        Receiver::Manager,
    ),
    (
        MutationSite {
            name: "set_playthrough_flag",
            rva: Rva(0xd1fd50),
        },
        Receiver::Manager,
    ),
    (
        MutationSite {
            name: "read_flags",
            rva: Rva(0xd1fa00),
        },
        Receiver::Manager,
    ),
    (
        MutationSite {
            name: "daily_flags",
            rva: Rva(0xd1f910),
        },
        Receiver::Manager,
    ),
    (
        MutationSite {
            name: "clear_flag",
            rva: Rva(0xd1fce0),
        },
        Receiver::Manager,
    ),
    (
        MutationSite {
            name: "clear_flags",
            rva: Rva(0xd1f4f0),
        },
        Receiver::Manager,
    ),
    (
        MutationSite {
            name: "assign_flags",
            rva: Rva(0x1e2b90),
        },
        Receiver::Rows,
    ),
    (
        MutationSite {
            name: "construct_flags",
            rva: Rva(0xd1f320),
        },
        Receiver::Manager,
    ),
    (
        MutationSite {
            name: "release_flags",
            rva: Rva(0xd1f3b0),
        },
        Receiver::Manager,
    ),
    (
        MutationSite {
            name: "destroy_flags",
            rva: Rva(0xd1f410),
        },
        Receiver::Manager,
    ),
];

pub fn manager(receiver: Receiver, argument: usize) -> usize {
    match receiver {
        Receiver::Manager => argument,
        Receiver::Rows => argument.wrapping_sub(ROWS),
    }
}
