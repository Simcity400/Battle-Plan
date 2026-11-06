//! The runtime writers of the orders-group fields the division census selects through: the
//! leader (`corders_group::LEADER`), HQ provider (`corders_group::HQ_PROVIDER`), training
//! byte (`corders_group::TRAINING`) and identity (vtable and `corders_group::HANDLE`).
//! Observing them tells the census which groups to reread; recording never calls them.
//!
//! - The leader has one setter, which every leader change goes through.
//! - The HQ provider is set by deployed-leader selection and cleared by the provider clear,
//!   which ungrouping, withdrawal and leader changes call.
//! - Training is set by its setter; army-group assignment copies an army group's training
//!   into the child it names second; the daily group update clears it from a group training
//!   to its maximum.
//! - Identity is written at construction and creation's registration, before any division
//!   names the group, and at destruction, whose patch is [`crate::template_mutations`]'
//!   `destroy_hq_group`, which forwards it.
//!
//! Every other store to these fields is in save loading and its container repair; a load
//! changes the epoch.

pub use crate::advisor_mutations::MutationSite;
use engine_sites::Rva;

/// Which argument names the group a site writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Receiver {
    First,
    Second,
}

/// The daily update writes only a group training to its maximum.
pub const DAILY_UPDATE: usize = 3;

/// The writers, in callback order. Each takes at most three integer or pointer arguments
/// and returns RAX unchanged from its original.
pub const MUTATIONS: [(MutationSite, Receiver); 6] = [
    // The five-byte RBX spill to RCX's home slot relocates.
    (site("set_leader", Rva(0xc4_b460)), Receiver::First),
    (site("set_training", Rva(0xc4_bd10)), Receiver::First),
    (
        site("assign_to_army_group", Rva(0xc4_03a0)),
        Receiver::Second,
    ),
    // Saving RBX and RDI and reserving 0x38 stack bytes occupy seven relocated bytes.
    (site("daily_update", Rva(0xc4_1cd0)), Receiver::First),
    // Copying RSP to R11 and saving RBP and R13 occupy six relocated bytes.
    (site("set_deployed_leader", Rva(0xc4_b600)), Receiver::First),
    // Copying RSP to R11 and spilling R8B to its home slot occupy seven relocated bytes.
    (site("clear_hq_provider", Rva(0xc4_98c0)), Receiver::First),
];

const fn site(name: &'static str, rva: Rva) -> MutationSite {
    MutationSite { name, rva }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sites_are_distinct_and_the_daily_index_names_the_daily_update() {
        for (index, (site, _)) in MUTATIONS.iter().enumerate() {
            assert!(
                MUTATIONS[..index]
                    .iter()
                    .all(|(other, _)| other.rva != site.rva && other.name != site.name),
                "{}",
                site.name
            );
        }
        assert_eq!(MUTATIONS[DAILY_UPDATE].0.name, "daily_update");
        assert!(
            crate::template_mutations::MUTATIONS
                .iter()
                .any(|(site, _)| site.name == "destroy_hq_group")
        );
    }
}
