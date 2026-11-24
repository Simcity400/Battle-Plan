
pub use crate::advisor_mutations::MutationSite;
use engine_sites::Rva;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Receiver {
    Status,
    BonusList,
    Unnamed,
}

pub const BONUS_LIST: usize = crate::cdeployment_status::UNIT_BONUSES;

pub const CLONE_COUNT: usize = crate::cdeployment_status::LOCAL_SUBUNITS + 0xc;

pub const MUTATIONS: [(MutationSite, Receiver); 6] = [
    (
        MutationSite {
            name: "ensure_local_subunit",
            rva: Rva(0xd6_f9c0),
        },
        Receiver::Status,
    ),
    (
        MutationSite {
            name: "rebuild_local_subunit",
            rva: Rva(0x10a_1060),
        },
        Receiver::BonusList,
    ),
    (
        MutationSite {
            name: "clear_local_subunits",
            rva: Rva(0xd7_0550),
        },
        Receiver::Status,
    ),
    (
        MutationSite {
            name: "initialize_default_subunits",
            rva: Rva(0xd7_5560),
        },
        Receiver::Status,
    ),
    (
        MutationSite {
            name: "find_or_create_subunit_definition",
            rva: Rva(0xb1_5120),
        },
        Receiver::Unnamed,
    ),
    (
        MutationSite {
            name: "reload_equipment_content",
            rva: Rva(0x17_e760),
        },
        Receiver::Unnamed,
    ),
];

