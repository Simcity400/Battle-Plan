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
    (
        site(
            "set_leader",
            0xc4_b460,
            [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24,
                0x18, 0x57,
            ],
        ),
        Receiver::First,
    ),
    (
        site(
            "set_training",
            0xc4_bd10,
            [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57, 0x48, 0x83, 0xec,
                0x30, 0x80,
            ],
        ),
        Receiver::First,
    ),
    (
        site(
            "assign_to_army_group",
            0xc4_03a0,
            [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x7c, 0x24,
                0x20, 0x48,
            ],
        ),
        Receiver::Second,
    ),
    // Saving RBX and RDI and reserving 0x38 stack bytes occupy seven relocated bytes.
    (
        site(
            "daily_update",
            0xc4_1cd0,
            [
                0x40, 0x53, 0x57, 0x48, 0x83, 0xec, 0x38, 0x48, 0x8b, 0x81, 0xf8, 0x01, 0x00, 0x00,
                0x48, 0x8d,
            ],
        ),
        Receiver::First,
    ),
    // Copying RSP to R11 and saving RBP and R13 occupy six relocated bytes.
    (
        site(
            "set_deployed_leader",
            0xc4_b600,
            [
                0x4c, 0x8b, 0xdc, 0x55, 0x41, 0x55, 0x48, 0x83, 0xec, 0x78, 0x48, 0x83, 0xb9, 0x88,
                0x00, 0x00,
            ],
        ),
        Receiver::First,
    ),
    // Copying RSP to R11 and spilling R8B to its home slot occupy seven relocated bytes.
    (
        site(
            "clear_hq_provider",
            0xc4_98c0,
            [
                0x4c, 0x8b, 0xdc, 0x45, 0x88, 0x43, 0x18, 0x49, 0x89, 0x53, 0x10, 0x49, 0x89, 0x4b,
                0x08, 0x57,
            ],
        ),
        Receiver::First,
    ),
];

const fn site(name: &'static str, rva: usize, sentinel: [u8; 16]) -> MutationSite {
    MutationSite {
        name,
        rva,
        sentinel,
    }
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
