//! Native design mutation boundaries. Every site has integer/pointer arguments
//! in the four register argument slots and preserves its native return value.
//! Commands name the variant through their stored reference; publication names
//! it in the second argument; bulk flag writers name an ordered pointer array.
#[derive(Clone, Copy)]
pub enum Receiver {
    Variant,
    Command,
    Published,
    Removed,
    Roster,
    CreateCommand,
}
pub struct Mutation {
    pub name: &'static str,
    pub rva: usize,
    pub sentinel: [u8; 16],
    pub receiver: Receiver,
    pub witness: Option<usize>,
}
pub const COMMAND_VARIANT: usize = 0x28;
/// World-input owns these entries; receivers retain the complete descriptor order.
pub const FORWARDED_SITES: [usize; 3] = [2, 3, 9];
/// Hook installation authenticates sites, not completeness of their writer set.
/// Accepted publication/edit boundaries, base destruction, shared clone/move,
/// source/provider lifetime and narrow author registration boundaries close
/// the writer set. Runtime admission additionally requires all three baselines.
pub const WRITER_AUDIT_REFUSAL: Option<&str> = None;
pub const MUTATIONS: &[Mutation] = &[
    Mutation {
        name: "RebuildStats",
        rva: 0x00be0260,
        sentinel: [
            0x48, 0x8b, 0xc4, 0x48, 0x89, 0x58, 0x20, 0x88, 0x50, 0x10, 0x48, 0x89, 0x48, 0x08,
            0x55, 0x56,
        ],
        receiver: Receiver::Variant,
        witness: None,
    },
    Mutation {
        name: "RefreshDerivedState",
        rva: 0x00bd8400,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x18, 0x57, 0x48, 0x83, 0xec, 0x50, 0x48, 0x8b, 0xd9, 0xe8,
            0xfe, 0x74,
        ],
        receiver: Receiver::Variant,
        witness: None,
    },
    Mutation {
        name: "RefreshOrganisation",
        rva: 0x00be2450,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24,
            0x18, 0x48,
        ],
        receiver: Receiver::Variant,
        witness: None,
    },
    Mutation {
        name: "RefreshOrganisationBonus",
        rva: 0x00bd3b80,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24,
            0x18, 0x57,
        ],
        receiver: Receiver::Variant,
        witness: None,
    },
    Mutation {
        name: "SetObsolete",
        rva: 0x00be2040,
        sentinel: [
            0x48, 0x83, 0xec, 0x28, 0x88, 0x91, 0x24, 0x04, 0x00, 0x00, 0x48, 0x81, 0xc1, 0x88,
            0x04, 0x00,
        ],
        receiver: Receiver::Variant,
        witness: None,
    },
    Mutation {
        name: "PropagateObsolete",
        rva: 0x00be20b0,
        sentinel: [
            0x48, 0x89, 0x6c, 0x24, 0x20, 0x57, 0x48, 0x83, 0xec, 0x20, 0x0f, 0xb6, 0xea, 0x48,
            0x8b, 0xf9,
        ],
        receiver: Receiver::Variant,
        witness: None,
    },
    Mutation {
        name: "SetHighlight",
        rva: 0x00be1d60,
        sentinel: [
            0x40, 0x57, 0x48, 0x83, 0xec, 0x30, 0x88, 0x91, 0x26, 0x04, 0x00, 0x00, 0x48, 0x8b,
            0xf9, 0x84,
        ],
        receiver: Receiver::Variant,
        witness: None,
    },
    Mutation {
        name: "EnableUpgradeVariant",
        rva: 0x00be2b90,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x80, 0xb9, 0x28, 0x04,
            0x00, 0x00,
        ],
        receiver: Receiver::Variant,
        witness: Some(0x428),
    },
    Mutation {
        name: "RefreshNiches",
        rva: 0x00be25b0,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xda, 0xc7,
            0x81, 0xc4,
        ],
        receiver: Receiver::Variant,
        witness: None,
    },
    Mutation {
        name: "MoveAssign",
        rva: 0x0062cd60,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xfa, 0x48,
            0x8b, 0xd9,
        ],
        receiver: Receiver::Variant,
        witness: None,
    },
    Mutation {
        name: "Publish",
        rva: 0x001ca780,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x18, 0x55, 0x48, 0x83, 0xec, 0x20, 0x48, 0x63, 0x81, 0x14,
            0x07, 0x00,
        ],
        receiver: Receiver::Published,
        witness: None,
    },
    Mutation {
        name: "Destroy",
        rva: 0x00bd1680,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x10, 0x56, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8d, 0x05, 0x77,
            0xff, 0xd7,
        ],
        receiver: Receiver::Removed,
        witness: None,
    },
    Mutation {
        name: "Rename",
        rva: 0x019a41f0,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x18, 0x57, 0x48, 0x83, 0xec, 0x40, 0x48, 0x8b, 0xd9, 0x48,
            0x83, 0xc1,
        ],
        receiver: Receiver::Command,
        witness: None,
    },
    Mutation {
        name: "SetAutoUpgraded",
        rva: 0x019a4380,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x18, 0x57, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0xd9, 0x48,
            0x83, 0xc1,
        ],
        receiver: Receiver::Command,
        witness: None,
    },
    Mutation {
        name: "SetNamesGroup",
        rva: 0x019a4580,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57, 0x48, 0x83, 0xec,
            0x20, 0x48,
        ],
        receiver: Receiver::Command,
        witness: None,
    },
    Mutation {
        name: "SetRoleIcon",
        rva: 0x019a46f0,
        sentinel: [
            0x40, 0x53, 0x55, 0x56, 0x48, 0x83, 0xec, 0x40, 0x48, 0x8b, 0xd9, 0x48, 0x89, 0x7c,
            0x24, 0x70,
        ],
        receiver: Receiver::Command,
        witness: None,
    },
    Mutation {
        name: "SetModel",
        rva: 0x019a4930,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57, 0x48, 0x83, 0xec,
            0x20, 0x48,
        ],
        receiver: Receiver::Command,
        witness: None,
    },
    Mutation {
        name: "SetIcon",
        rva: 0x019a4a80,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x57, 0x48, 0x83, 0xec,
            0x20, 0x48,
        ],
        receiver: Receiver::Command,
        witness: None,
    },
    Mutation {
        name: "Update",
        rva: 0x019a4f60,
        sentinel: [
            0x48, 0x89, 0x4c, 0x24, 0x08, 0x55, 0x53, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41,
            0x56, 0x41,
        ],
        receiver: Receiver::Command,
        witness: None,
    },
    Mutation {
        name: "BulkUpgradeModules",
        rva: 0x00be26c0,
        sentinel: [
            0x40, 0x56, 0x41, 0x56, 0x41, 0x57, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8b, 0x31, 0x4c,
            0x8b, 0xf2,
        ],
        receiver: Receiver::Roster,
        witness: None,
    },
    Mutation {
        name: "BulkUpgradeTypes",
        rva: 0x00be2a70,
        sentinel: [
            0x40, 0x53, 0x55, 0x56, 0x48, 0x83, 0xec, 0x30, 0x48, 0x8b, 0x31, 0x48, 0x8b, 0xda,
            0x48, 0x63,
        ],
        receiver: Receiver::Roster,
        witness: None,
    },
    Mutation {
        name: "BulkUpgradeVariants",
        rva: 0x00bdf140,
        sentinel: [
            0x41, 0x54, 0x41, 0x56, 0x41, 0x57, 0x48, 0x83, 0xec, 0x20, 0x4c, 0x8b, 0x31, 0x4c,
            0x8b, 0xf9,
        ],
        receiver: Receiver::Roster,
        witness: None,
    },
    Mutation {
        name: "CreateCommand",
        rva: 0x019a30e0,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x20, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56,
            0x41, 0x57,
        ],
        receiver: Receiver::CreateCommand,
        witness: None,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variant_design_mutation_sites_match_supported_executable() {
        let path = std::path::PathBuf::from(
            std::env::var_os("BATTLE_PLAN_GAME_DIR").or_else(|| std::env::var_os("HOI4_GAME_DIR")).expect("design site test requires BATTLE_PLAN_GAME_DIR"),
        )
        .join("hoi4.exe");
        let bytes = std::fs::read(path).unwrap();
        let identity =
            engine_executable::build::hash_executable(std::io::Cursor::new(&bytes)).unwrap();
        assert!(engine_executable::build::PINNED_BUILD.matches(&identity));
        let image = engine_executable::executable_tables::PeImage::parse(bytes).unwrap();
        let mut entries = std::collections::BTreeSet::new();
        let all = MUTATIONS
            .iter()
            .map(|site| (site.name, site.rva, site.sentinel))
            .chain(
                crate::cequipment_variant::source_mutations::MUTATIONS
                    .iter()
                    .map(|site| (site.name, site.rva, site.sentinel)),
            )
            .chain(
                crate::cequipment_variant::source_mutations::ORGANISATIONS
                    .iter()
                    .map(|site| (site.name, site.rva, site.sentinel)),
            );
        for (name, rva, sentinel) in all {
            assert!(entries.insert(rva), "duplicate design hook {}", name);
            assert_eq!(
                image.bytes(rva as u32, sentinel.len()).unwrap(),
                sentinel,
                "{}",
                name
            );
            match image.function(rva as u32) {
                Ok((start, _)) => assert_eq!(start, rva as u32, "{name} is not a function entry"),
                // This native reset is a leaf without an exception-directory
                // entry. Its authenticated bytes still identify its entrance.
                Err(_) => assert_eq!(name, "ResetAppliedSource", "{name} lacks a function extent"),
            }
        }
    }
}
