//! Native writers of goal inventories and justification activities.
//! Only the owning country is invalidated. Loading and unknown receivers
//! revoke every certificate. The occupation-owned daily diplomacy update forwards
//! goal expiry observation; it is not a second patch in this inventory.
#[derive(Clone, Copy)]
pub enum Receiver {
    Diplomacy,
    ConsumedGoal,
    Goal,
    Politics,
    ActivityInsert,
    ActivityPrune,
    Activity,
}
impl Receiver {
    pub const fn part(self) -> usize {
        match self {
            Self::Politics | Self::ActivityInsert | Self::ActivityPrune | Self::Activity => 1,
            _ => 0,
        }
    }
}
pub struct Site {
    pub name: &'static str,
    pub rva: usize,
    pub sentinel: [u8; 16],
    pub receiver: Receiver,
}
pub const SITES: [Site; 16] = [
    Site {
        name: "grant_goal",
        rva: 0xd3f730,
        sentinel: [
            0x48, 0x89, 0x54, 0x24, 0x10, 0x53, 0x55, 0x56, 0x48, 0x83, 0xec, 0x50, 0x48, 0x8b,
            0x02, 0x48,
        ],
        receiver: Receiver::Diplomacy,
    },
    Site {
        name: "remove_country_goals",
        rva: 0xd3ac70,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24, 0x18, 0x48, 0x89, 0x7c, 0x24,
            0x20, 0x41,
        ],
        receiver: Receiver::Diplomacy,
    },
    Site {
        name: "invalidate_goals",
        rva: 0xd43280,
        sentinel: [
            0x40, 0x53, 0x57, 0x48, 0x83, 0xec, 0x28, 0x8b, 0x41, 0x74, 0xff, 0xc8, 0x48, 0x89,
            0x74, 0x24,
        ],
        receiver: Receiver::Diplomacy,
    },
    Site {
        name: "remove_goals",
        rva: 0xd437d0,
        sentinel: [
            0x4c, 0x89, 0x44, 0x24, 0x18, 0x48, 0x89, 0x54, 0x24, 0x10, 0x48, 0x89, 0x4c, 0x24,
            0x08, 0x53,
        ],
        receiver: Receiver::Diplomacy,
    },
    Site {
        name: "consume_goal",
        rva: 0xd199e0,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x74, 0x24, 0x20, 0x57, 0x48, 0x83, 0xec,
            0x30, 0x8b,
        ],
        receiver: Receiver::ConsumedGoal,
    },
    Site {
        name: "destroy_goal",
        rva: 0x15de480,
        sentinel: [
            0x40, 0x53, 0x48, 0x83, 0xec, 0x20, 0x45, 0x33, 0xc9, 0x48, 0x8d, 0x05, 0xc8, 0x1b,
            0x40, 0x01,
        ],
        receiver: Receiver::Goal,
    },
    Site {
        name: "read_goal",
        rva: 0x15e08f0,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec,
            0x20, 0x41,
        ],
        receiver: Receiver::Goal,
    },
    Site {
        name: "insert_activity",
        rva: 0xba7570,
        sentinel: [
            0x48, 0x89, 0x6c, 0x24, 0x20, 0x41, 0x56, 0x48, 0x83, 0xec, 0x20, 0x48, 0x63, 0x41,
            0x14, 0x48,
        ],
        receiver: Receiver::ActivityInsert,
    },
    Site {
        name: "cancel_activity",
        rva: 0xbae550,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x20, 0x56, 0x41, 0x54, 0x41, 0x57, 0x48, 0x83, 0xec, 0x20,
            0x48, 0x63,
        ],
        receiver: Receiver::Politics,
    },
    Site {
        name: "advance_activity",
        rva: 0xadcbd0,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec,
            0x20, 0x48,
        ],
        receiver: Receiver::Activity,
    },
    Site {
        name: "read_activity",
        rva: 0xadc130,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x40, 0x4c, 0x8b, 0xca, 0x48,
            0x8b, 0xd9,
        ],
        receiver: Receiver::Activity,
    },
    Site {
        name: "read_justification",
        rva: 0xadc6d0,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x30, 0x48, 0x8b, 0xda, 0x48,
            0x8b, 0xf9,
        ],
        receiver: Receiver::Activity,
    },
    Site {
        name: "destroy_justification",
        rva: 0xad7690,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x74, 0x24, 0x10, 0x57, 0x48, 0x83, 0xec,
            0x20, 0x48,
        ],
        receiver: Receiver::Activity,
    },
    // Virtual deletion inlines teardown instead of calling the direct destructor.
    Site {
        name: "delete_justification",
        rva: 0xad7940,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x08, 0x48, 0x89, 0x6c, 0x24, 0x10, 0x48, 0x89, 0x74, 0x24,
            0x18, 0x57,
        ],
        receiver: Receiver::Activity,
    },
    Site {
        name: "delete_goal",
        rva: 0x15de530,
        sentinel: [
            0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x45, 0x33, 0xc9, 0x48,
            0x8d, 0x05,
        ],
        receiver: Receiver::Goal,
    },
    // Post-peace and country-rebuild validity pruning also compacts activity ordinals.
    Site {
        name: "prune_activities",
        rva: 0xbae4a0,
        sentinel: [
            0x40, 0x57, 0x48, 0x83, 0xec, 0x20, 0x48, 0x63, 0x41, 0x14, 0x48, 0x8b, 0xf9, 0x8d,
            0x48, 0xff,
        ],
        receiver: Receiver::ActivityPrune,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daily_diplomacy_expiry_is_forwarded_instead_of_patched_again() {
        assert!(!SITES.iter().any(|site| site.name == "expire_goals"));
        assert_eq!(SITES.len(), 16);
    }

    #[test]
    fn virtual_deletion_hooks_match_the_executable_vtable_entries() {
        use engine_executable::executable_tables::PeImage;
        engine_executable::executable_tables::initialize_for_tests().unwrap();
        let root = std::env::var_os("BATTLE_PLAN_GAME_DIR").or_else(|| std::env::var_os("HOI4_GAME_DIR"))
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation");
        let image =
            PeImage::parse(std::fs::read(std::path::Path::new(&root).join("hoi4.exe")).unwrap())
                .unwrap();
        for (name, table, part) in [
            ("delete_goal", crate::cwar_goal::VTABLE_RVA, 0),
            (
                "delete_justification",
                crate::cwar_goal::inventory::JUSTIFICATION_VTABLE_RVA,
                1,
            ),
        ] {
            let site = SITES
                .iter()
                .find(|s| s.name == name)
                .expect("virtual deletion must revoke carry");
            assert_eq!(
                site.rva as u32,
                image.va_to_rva(image.u64(table as u32).unwrap()).unwrap()
            );
            assert_eq!(site.receiver.part(), part);
        }
        for site in &SITES {
            assert_eq!(
                image.bytes(site.rva as u32, site.sentinel.len()).unwrap(),
                site.sentinel,
                "{}",
                site.name
            );
        }
    }
}
