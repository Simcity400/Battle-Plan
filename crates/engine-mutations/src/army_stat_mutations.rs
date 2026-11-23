
pub use crate::advisor_mutations::MutationSite;
use engine_sites::Rva;

const UPDATE_ACTIVE_STATS_RVA: Rva = Rva(0xce_a3c0);

const AWARD_EXPERIENCE_RVA: Rva = Rva(0xccb810);
const ADD_MANPOWER_EXPERIENCE_RVA: Rva = Rva(0xccb940);
const REFRESH_EXPERIENCE_RVA: Rva = Rva(0xcecd10);
const REBIND_EXPERIENCE_RVA: Rva = Rva(0xcd6700);

const CONSTRUCT_DIVISION_HISTORY_RVA: Rva = Rva(0x14e23c0);
const DESTROY_DIVISION_HISTORY_RVA: Rva = Rva(0x14e2980);
const APPEND_DIVISION_HISTORY_RVA: Rva = Rva(0x14e2de0);
const APPLY_DIVISION_AWARD_RVA: Rva = Rva(0x14e33b0);
const CLEAR_MEDAL_INHERITANCE_RVA: Rva = Rva(0x14e76e0);
const CONSTRUCT_MEDAL_STORE_RVA: Rva = Rva(0x14e28d0);
const BIND_DIVISION_TEMPLATE_RVA: Rva = Rva(0xceb320);
const ADD_OFFICER_EXPERIENCE_RVA: Rva = Rva(0x148_4b80);
const SET_OFFICER_EXPERIENCE_RVA: Rva = Rva(0x148_69e0);
const PROMOTE_DIVISION_OFFICER_RVA: Rva = Rva(0x14ecd00);
const DESTROY_MEDAL_STORE_RVA: Rva = Rva(0x14e2d40);

pub const MUTATIONS: [MutationSite; 19] = [
    MutationSite {
        name: "update_active_stats",
        rva: UPDATE_ACTIVE_STATS_RVA,
    },
    MutationSite {
        name: "refresh_cached_name",
        rva: Rva(0xa0_c820),
    },
    MutationSite {
        name: "refresh_country_names",
        rva: Rva(0xa0_5020),
    },
    MutationSite {
        name: "award_experience",
        rva: AWARD_EXPERIENCE_RVA,
    },
    MutationSite {
        name: "add_manpower_experience",
        rva: ADD_MANPOWER_EXPERIENCE_RVA,
    },
    MutationSite {
        name: "refresh_experience",
        rva: REFRESH_EXPERIENCE_RVA,
    },
    MutationSite {
        name: "rebind_experience",
        rva: REBIND_EXPERIENCE_RVA,
    },
    MutationSite {
        name: "construct_division",
        rva: engine_native::carmy::CONSTRUCTOR_RVA,
    },
    MutationSite {
        name: "construct_division_history",
        rva: CONSTRUCT_DIVISION_HISTORY_RVA,
    },
    MutationSite {
        name: "destroy_division_history",
        rva: DESTROY_DIVISION_HISTORY_RVA,
    },
    MutationSite {
        name: "append_division_history",
        rva: APPEND_DIVISION_HISTORY_RVA,
    },
    MutationSite {
        name: "apply_division_award",
        rva: APPLY_DIVISION_AWARD_RVA,
    },
    MutationSite {
        name: "clear_medal_inheritance",
        rva: CLEAR_MEDAL_INHERITANCE_RVA,
    },
    MutationSite {
        name: "construct_medal_store",
        rva: CONSTRUCT_MEDAL_STORE_RVA,
    },
    MutationSite {
        name: "destroy_medal_store",
        rva: DESTROY_MEDAL_STORE_RVA,
    },
    MutationSite {
        name: "promote_division_officer",
        rva: PROMOTE_DIVISION_OFFICER_RVA,
    },
    MutationSite {
        name: "bind_division_template",
        rva: BIND_DIVISION_TEMPLATE_RVA,
    },
    MutationSite {
        name: "add_officer_experience",
        rva: ADD_OFFICER_EXPERIENCE_RVA,
    },
    MutationSite {
        name: "set_officer_experience",
        rva: SET_OFFICER_EXPERIENCE_RVA,
    },
];

pub const EXPERIENCE_TEMPLATE_SITE: usize = 16;

pub const EXPERIENCE_FIRST: usize = 3;
pub const HISTORY_FIRST: usize = 8;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_site_is_the_function_the_carmy_layout_names() {
        assert_eq!(MUTATIONS[0].rva, engine_native::carmy::UPDATE_ACTIVE_STATS_RVA);
    }
}
