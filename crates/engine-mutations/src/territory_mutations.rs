
pub use crate::advisor_mutations::MutationSite;
use engine_sites::Rva;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StateRows {
    pub state: bool,
    pub provinces: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Receiver {
    Province,
    State(StateRows),
    EveryProvince,
    EveryState,
}

const BOTH: StateRows = StateRows {
    state: true,
    provinces: true,
};

pub const MUTATIONS: [(MutationSite, Receiver); 16] = [
    (
        MutationSite {
            name: "province_controller",
            rva: Rva(0xef_1860),
        },
        Receiver::Province,
    ),
    (
        MutationSite {
            name: "victory_points",
            rva: Rva(0xef_27b0),
        },
        Receiver::Province,
    ),
    (
        MutationSite {
            name: "read_province",
            rva: Rva(0xef_0de0),
        },
        Receiver::Province,
    ),
    (
        MutationSite {
            name: "insert_border_conflict",
            rva: Rva(0xa1_c020),
        },
        Receiver::State(StateRows {
            state: false,
            provinces: true,
        }),
    ),
    (
        MutationSite {
            name: "erase_border_conflict",
            rva: Rva(0xa2_7e70),
        },
        Receiver::State(StateRows {
            state: false,
            provinces: true,
        }),
    ),
    (
        MutationSite {
            name: "rebuild_owner_areas",
            rva: Rva(0xd5_74c0),
        },
        Receiver::EveryProvince,
    ),
    (
        MutationSite {
            name: "initialize_provinces",
            rva: Rva(0xa2_9aa0),
        },
        Receiver::EveryProvince,
    ),
    (
        MutationSite {
            name: "state_owner",
            rva: Rva(0xa2_9290),
        },
        Receiver::State(BOTH),
    ),
    (
        MutationSite {
            name: "state_controller",
            rva: Rva(0xa2_8770),
        },
        Receiver::State(StateRows {
            state: true,
            provinces: false,
        }),
    ),
    (
        MutationSite {
            name: "reset_state",
            rva: Rva(0xa2_1920),
        },
        Receiver::State(BOTH),
    ),
    (
        MutationSite {
            name: "read_state",
            rva: Rva(0xa2_6b30),
        },
        Receiver::State(BOTH),
    ),
    (
        MutationSite {
            name: "state_name_effect",
            rva: Rva(0x32_96d0),
        },
        Receiver::EveryState,
    ),
    (
        MutationSite {
            name: "recount_controller_groups",
            rva: Rva(0x00a2_a440),
        },
        Receiver::State(StateRows {
            state: true,
            provinces: false,
        }),
    ),
    (
        MutationSite {
            name: "refresh_cached_victory_point",
            rva: Rva(0x00a2_a1e0),
        },
        Receiver::State(StateRows {
            state: true,
            provinces: false,
        }),
    ),
    (
        MutationSite {
            name: "add_contested_owner",
            rva: Rva(0x00a1_c340),
        },
        Receiver::State(StateRows {
            state: true,
            provinces: false,
        }),
    ),
    (
        MutationSite {
            name: "remove_contested_owner",
            rva: Rva(0x00a2_8070),
        },
        Receiver::State(StateRows {
            state: true,
            provinces: false,
        }),
    ),
];

