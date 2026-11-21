
pub use crate::advisor_mutations::MutationSite;
use engine_sites::Rva;

pub const MUTATIONS: [MutationSite; 2] = [
    MutationSite {
        name: "register_mission",
        rva: Rva(0xf1_e560),
    },
    MutationSite {
        name: "unregister_mission",
        rva: Rva(0xf2_24a0),
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sites_are_distinct_and_the_reset_belongs_to_world_input() {
        assert_ne!(MUTATIONS[0].rva, MUTATIONS[1].rva);
        let (reset, _) = crate::world_input_mutations::MUTATIONS
            .iter()
            .find(|(site, _)| site.name == "reset_navy")
            .unwrap();
        for site in MUTATIONS {
            assert_ne!(site.rva, reset.rva, "{}", site.name);
        }
    }
}
