pub use crate::advisor_mutations::MutationSite;
use engine_sites::Rva;

pub const ADD_UNIT_RVA: Rva = Rva(0x147_d030);
pub const REMOVE_UNIT_RVA: Rva = Rva(0x147_f390);
pub const NAVAL_ADD_TRANSFER_RVA: Rva = Rva(0x16b_4740);
pub const CHANGE_CONTROLLER_RVA: Rva = Rva(0x11d_cfb0);
pub const UPDATE_ALL_COMBATS_RVA: Rva = Rva(0xc0_e940);
pub const COMBAT_MANAGER_HOURLY_UPDATE_RVA: Rva = Rva(0xc0_c660);

pub const COMBAT_MANAGER_VTABLE_RVA: Rva = Rva(0x29c_d318);

pub const CHANGE_CONTROLLER_TARGET: usize = 0x28;
pub const COMBAT_MANAGER_COUNT: usize = 0x14;
pub const COMBAT_MANAGER_CACHED_VIEWER: usize = 0x20;

pub const MUTATIONS: [MutationSite; 6] = [
    MutationSite {
        name: "combatant_add_unit",
        rva: ADD_UNIT_RVA,
    },
    MutationSite {
        name: "combatant_remove_unit",
        rva: REMOVE_UNIT_RVA,
    },
    MutationSite {
        name: "naval_add_transfer_to_group",
        rva: NAVAL_ADD_TRANSFER_RVA,
    },
    MutationSite {
        name: "change_country_controller_execute",
        rva: CHANGE_CONTROLLER_RVA,
    },
    MutationSite {
        name: "update_all_combats_shared_vision",
        rva: UPDATE_ALL_COMBATS_RVA,
    },
    MutationSite {
        name: "combat_manager_hourly_update",
        rva: COMBAT_MANAGER_HOURLY_UPDATE_RVA,
    },
];
