
use engine_native_memory::{Memory, cpdx_array::Array, read};
use crate::{cprovince_railway_info, crailway_manager};
use engine_native::cprovince::definition as province_definition;

pub use crate::advisor_mutations::MutationSite;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Receiver {
    Province,
    Info,
    CooldownList,
    Every,
}

pub const MUTATIONS: [(MutationSite, Receiver); 9] = [
    (
        MutationSite {
            name: "set_province_on_cooldown",
            rva: crailway_manager::SET_PROVINCE_ON_COOLDOWN_RVA,
        },
        Receiver::Province,
    ),
    (
        MutationSite {
            name: "update_cooldowns",
            rva: crailway_manager::UPDATE_COOLDOWNS_RVA,
        },
        Receiver::CooldownList,
    ),
    (
        MutationSite {
            name: "set_railway",
            rva: crate::cprovince_railway_info::SET_RAILWAY_RVA,
        },
        Receiver::Info,
    ),
    (
        MutationSite {
            name: "construct_info",
            rva: crate::cprovince_railway_info::CONSTRUCTOR_RVA,
        },
        Receiver::Province,
    ),
    (
        MutationSite {
            name: "destroy_info",
            rva: crate::cprovince_railway_info::DESTRUCTOR_RVA,
        },
        Receiver::Info,
    ),
    (
        MutationSite {
            name: "read_info",
            rva: crate::cprovince_railway_info::LOAD_TOKEN_RVA,
        },
        Receiver::Every,
    ),
    (
        MutationSite {
            name: "construct_manager",
            rva: crailway_manager::CONSTRUCTOR_RVA,
        },
        Receiver::Every,
    ),
    (
        MutationSite {
            name: "destroy_manager",
            rva: crailway_manager::DESTRUCTOR_RVA,
        },
        Receiver::Every,
    ),
    (
        MutationSite {
            name: "read_manager",
            rva: crailway_manager::LOAD_TOKEN_RVA,
        },
        Receiver::Every,
    ),
];

pub fn province(memory: &impl Memory, info: usize) -> Option<u32> {
    let definition = info
        .checked_add(cprovince_railway_info::PROVINCE_RECORD)
        .and_then(|at| read::<u64>(memory, at))
        .and_then(|pointer| usize::try_from(pointer).ok())
        .filter(|&pointer| pointer != 0)?;
    definition
        .checked_add(province_definition::ID)
        .and_then(|at| read::<u32>(memory, at))
        .filter(|&id| id != 0)
}

pub fn info(memory: &impl Memory, manager: usize, province: i32) -> Option<usize> {
    let province = u32::try_from(province).ok()?;
    let slots = manager
        .checked_add(crailway_manager::PROVINCE_INFO_TABLE)
        .and_then(|at| Array::read(memory, at, crailway_manager::PROVINCE_INFO_STRIDE))?;
    let slot = slots.element(province, crailway_manager::PROVINCE_INFO_STRIDE)?;
    usize::try_from(read::<u64>(memory, slot)?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;

    #[test]
    fn sites_are_distinct() {
        let mut rvas: Vec<_> = MUTATIONS.iter().map(|(site, _)| site.rva).collect();
        rvas.sort_unstable();
        rvas.dedup();
        assert_eq!(rvas.len(), MUTATIONS.len());
    }

    #[test]
    fn an_info_names_its_province_through_its_definition() {
        const BASE: usize = 0x1000;
        let mut bytes = vec![0_u8; 0x400];
        let info = BASE;
        let definition = BASE + 0x100;
        let at = cprovince_railway_info::PROVINCE_RECORD;
        bytes[at..at + 8].copy_from_slice(&(definition as u64).to_le_bytes());
        let id = 0x100 + province_definition::ID;
        bytes[id..id + 4].copy_from_slice(&517_u32.to_le_bytes());
        let memory = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        assert_eq!(province(&memory, info), Some(517));
        assert_eq!(province(&memory, BASE + 0x1000), None);
        bytes[id..id + 4].copy_from_slice(&0_u32.to_le_bytes());
        let memory = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        assert_eq!(province(&memory, info), None);
        bytes[at..at + 8].copy_from_slice(&0_u64.to_le_bytes());
        let memory = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        assert_eq!(province(&memory, info), None);
    }

    #[test]
    fn each_site_names_its_receiver() {
        let receiver = |name: &str| {
            MUTATIONS
                .iter()
                .find(|(site, _)| site.name == name)
                .unwrap()
                .1
        };
        assert_eq!(receiver("set_province_on_cooldown"), Receiver::Province);
        assert_eq!(receiver("construct_info"), Receiver::Province);
        assert_eq!(receiver("update_cooldowns"), Receiver::CooldownList);
        assert_eq!(receiver("set_railway"), Receiver::Info);
        assert_eq!(receiver("destroy_info"), Receiver::Info);
        for name in [
            "read_info",
            "construct_manager",
            "destroy_manager",
            "read_manager",
        ] {
            assert_eq!(receiver(name), Receiver::Every, "{name}");
        }
    }

    #[test]
    fn the_cooldown_setter_names_the_slot_it_writes() {
        const BASE: usize = 0x1000;
        let mut bytes = vec![0_u8; 0x200];
        let manager = BASE;
        let table = BASE + 0x100;
        let header = crailway_manager::PROVINCE_INFO_TABLE;
        bytes[header..header + 8].copy_from_slice(&(table as u64).to_le_bytes());
        bytes[header + 8..header + 12].copy_from_slice(&4_u32.to_le_bytes());
        bytes[header + 12..header + 16].copy_from_slice(&3_u32.to_le_bytes());
        bytes[0x108..0x110].copy_from_slice(&0x7000_u64.to_le_bytes());
        let memory = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        assert_eq!(info(&memory, manager, 1), Some(0x7000));
        assert_eq!(info(&memory, manager, 2), Some(0));
        assert_eq!(info(&memory, manager, 3), None);
        assert_eq!(info(&memory, manager, -1), None);
        assert_eq!(info(&memory, BASE + 0x1000, 1), None);
        bytes[header + 12..header + 16].copy_from_slice(&5_u32.to_le_bytes());
        let memory = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        assert_eq!(info(&memory, manager, 1), None);
    }
}
