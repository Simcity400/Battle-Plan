
pub use crate::advisor_mutations::MutationSite;
use engine_sites::Rva;

pub const MUTATIONS: [MutationSite; 10] = [
    MutationSite {
        name: "recalculate_equipment_stats",
        rva: Rva(0xc9_37d0),
    },
    MutationSite {
        name: "refit",
        rva: Rva(0xc8_a2a0),
    },
    MutationSite {
        name: "rebuild_experience_modifiers",
        rva: Rva(0xc9_3590),
    },
    MutationSite {
        name: "add_carrier_air_base",
        rva: engine_native_air::cstrategic_air_manager::ADD_CARRIER_AIR_BASE_RVA,
    },
    MutationSite {
        name: "destroy_air_base",
        rva: Rva(0xca_4f20),
    },
    MutationSite {
        name: "rebuild_medal_modifiers",
        rva: Rva(0x14e_87f0),
    },
    MutationSite {
        name: "rebuild_doctrine_tree",
        rva: Rva(0xdc_c750),
    },
    MutationSite {
        name: "rebuild_region_weather_modifiers",
        rva: Rva(0xfa_4fc0),
    },
    MutationSite {
        name: "bind_fleet_leader",
        rva: Rva(0xdc_0550),
    },
    MutationSite {
        name: "reset_reserve_fleet",
        rva: Rva(0xdc_0810),
    },
];

pub const RECEIVER_ARGUMENT: [usize; 10] = [0, 0, 0, 1, 0, 0, 0, 0, 0, 0];

