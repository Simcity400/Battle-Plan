
pub use crate::advisor_mutations::MutationSite;
use engine_sites::Rva;

pub const MUTATIONS: [MutationSite; 2] = [
    MutationSite {
        name: "add_unit",
        rva: Rva(0xee_b1d0),
    },
    MutationSite {
        name: "remove_unit",
        rva: Rva(0xef_10d0),
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sites_are_distinct() {
        let [add, remove] = MUTATIONS;
        assert_ne!(add.rva, remove.rva);
    }
}
