
use engine_sites::Rva;

pub struct MutationSite {
    pub name: &'static str,
    pub rva: Rva,
}

pub const MUTATIONS: [MutationSite; 1] = [MutationSite {
    name: "update_record",
    rva: crate::cair_region_combat_data::UPDATE_RECORD_RVA,
}];
