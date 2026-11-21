
pub use crate::advisor_mutations::MutationSite;
use engine_sites::Rva;

const SET_COUNTRY_RVA: Rva = Rva(0x102_a700);

pub const MUTATIONS: [MutationSite; 8] = [
    site("set_country", SET_COUNTRY_RVA),
    site("create_army_role", Rva(0x102_18f0)),
    site("create_navy_role", Rva(0x102_1d10)),
    site("clear_unit_role", Rva(0x102_a670)),
    site("add_country_leader_role", Rva(0x36_18c0)),
    site("erase_leader_roles", Rva(0x3d_2830)),
    site("add_scientist_role", Rva(0x36_4610)),
    site("set_operative_reference", Rva(0x102_a8b0)),
];

const fn site(name: &'static str, rva: Rva) -> MutationSite {
    MutationSite { name, rva }
}
