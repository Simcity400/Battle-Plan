
pub use crate::advisor_mutations::MutationSite;
use engine_sites::Rva;

const SET_NAME_RVA: Rva = Rva(0xc7_cc10);

const EXPERIENCE_ADD_UNCHECKED_RVA: Rva = Rva(0xc64da0);
const DAILY_PROGRESS_RVA: Rva = Rva(0xc75a30);
const TRAIT_PROGRESS_ADD_RVA: Rva = Rva(0xc6a800);
const TRAIT_PROGRESS_REMOVE_RVA: Rva = Rva(0xc6aae0);
const TRAIT_PROGRESS_PRUNE_RVA: Rva = Rva(0xc786e0);
const TRAIT_FACTOR_SET_RVA: Rva = Rva(0xc647e0);
const TRAIT_FACTOR_REMOVE_RVA: Rva = Rva(0xc790e0);
const TRAIT_PROGRESS_COMPLETE_RVA: Rva = Rva(0xc743b0);

pub const MUTATIONS: [MutationSite; 32] = [
    site("formation", Rva(0xc7_cea0)),
    site("skill", Rva(0xc7_d870)),
    site("category", Rva(0xc7_62e0)),
    site("army_attack", Rva(0xc7_ae90)),
    site("army_defense", Rva(0xc7_bcd0)),
    site("army_planning", Rva(0xc7_d310)),
    site("army_logistics", Rva(0xc7_c7d0)),
    site("navy_attack", Rva(0xc7_afc0)),
    site("navy_defense", Rva(0xc7_be00)),
    site("navy_maneuvering", Rva(0xc7_c900)),
    site("navy_coordination", Rva(0xc7_bb00)),
    site("capture", Rva(0xc6_7900)),
    site("release", Rva(0xc7_8f60)),
    site("owner", Rva(0xc7_bcc0)),
    site("operative_owner", Rva(0xc7_bc30)),
    site("exile_generation", Rva(0x72_dfa0)),
    site("exile_removal", Rva(0x30_05e0)),
    site("set_name", SET_NAME_RVA),
    site("rebuild_derived", Rva(0xc7_1100)),
    site("operative_rebuild", Rva(0xc7_0fe0)),
    site("link_character", Rva(0xc7_b3c0)),
    site("operative_codename", Rva(0xc7_b4a0)),
    site("operative_admission", Rva(0xc6_9030)),
    site(
        "experience_add",
        engine_native::cunit_leader::ADD_EXPERIENCE_RVA,
    ),
    site("experience_add_unchecked", EXPERIENCE_ADD_UNCHECKED_RVA),
    site("daily_progress", DAILY_PROGRESS_RVA),
    site("trait_progress_add", TRAIT_PROGRESS_ADD_RVA),
    site("trait_progress_remove", TRAIT_PROGRESS_REMOVE_RVA),
    site("trait_progress_prune", TRAIT_PROGRESS_PRUNE_RVA),
    site("trait_progress_complete", TRAIT_PROGRESS_COMPLETE_RVA),
    site("trait_factor_set", TRAIT_FACTOR_SET_RVA),
    site("trait_factor_remove", TRAIT_FACTOR_REMOVE_RVA),
];

pub const EXILE_GENERATION: usize = 15;
pub const EXILE_REMOVAL: usize = 16;

const fn site(name: &'static str, rva: Rva) -> MutationSite {
    MutationSite { name, rva }
}

