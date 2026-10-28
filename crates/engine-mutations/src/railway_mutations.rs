//! The runtime writers of fields recorded by RailCooldowns, RailEdges,
//! RailwayBuildingRow and its selected levels from each province rail info
//! (railway manager slot table `crailway_manager::PROVINCE_INFO_TABLE`).
//! Observing them tells the collector which infos to reread; recording never
//! calls them.
//!
//! Outputs include cooldown (`cprovince_railway_info::COOLDOWN_TICKS`);
//! sorted neighbour pairs and count (`NEIGHBOR_PAIRS`,
//! `NEIGHBOR_PAIR_COUNT`); neighbour-indexed levels and count
//! (`LEVELS_BY_NEIGHBOR_INDEX`, `LEVEL_COUNT`), sized from the province
//! definition's `province_definition::NEIGHBOUR_COUNT`; and rail-building
//! health through `cprovince_railway_info::BUILDING` (level, healthy level,
//! partial health and backlink: `cbuilding::LEVEL`, `HEALTHY_LEVEL`,
//! `PARTIAL_HEALTH`, `BUILDING_STATUS`). The walk also authenticates the
//! vtable, definition (`cprovince_railway_info::PROVINCE_RECORD`) and
//! building status (`cprovince_railway_info::BUILDING_STATUS`, the province's
//! `cprovince::BUILDING_STATUS`).
//!
//! Writer coverage:
//! - The cooldown setter takes manager, signed province id and ticks. It
//!   sign-extends the id and loads the slot without a bounds check. A nonnull
//!   info causes supply notification, appending the province to the manager's
//!   cooldown list (`crailway_manager::COOLDOWN_PROVINCES`, count
//!   `COOLDOWN_PROVINCE_COUNT`) when old cooldown is below one, and an
//!   unclamped ticks store. Only the hourly occupation controller-change path
//!   calls it. Ticks below one leave a listed value below one, so another
//!   call appends again; the list can contain duplicates. Daily countdown,
//!   called only by game-state daily update, walks backwards, decrements the
//!   info once per occurrence and swap-removes each occurrence when its value
//!   is below one. It writes no other info.
//! - Railway setting writes the indexed level as the greater of old and new,
//!   clamped to the define; updates or sorted-inserts the neighbour pair;
//!   creates the rail building when its pointer is null; and sets its level.
//!   Callers are railway-level addition from the railway effect, hub
//!   completion, daily construction and two debug paths, and the rail file
//!   loader. Edge levels never fall at runtime.
//! - Info construction takes the province id in EDX and writes every field:
//!   zero cooldown, province definition, levels sized from the definition's
//!   neighbour count and a looked-up rail-building pointer. Find-or-create
//!   calls it and stores the slot in the same call; manager token loading
//!   also constructs infos. Virtual destruction frees pairs. Info token
//!   loading reads cooldown, levels and pairs from a save.
//! - Manager construction, virtual destruction and token loading build,
//!   free or refill the slot table and cooldown list.
//! - Only [`crate::building_mutations`] writes rail-building health. Its
//!   status (`cprovince::BUILDING_STATUS`) names the province by location id;
//!   the collector forwards writes to the rail domain. Pending construction
//!   (`cprovince_railway_info::PENDING_CONSTRUCTION`) is not recorded.
//!
//! Every writer asserts game-thread access. Each prologue starts with five
//! bytes of whole relocatable instructions, and each site takes at most four
//! register arguments.

use crate::{
    Memory, cpdx_array::Array, cprovince_railway_info, crailway_manager, province_definition, read,
};

pub use crate::advisor_mutations::MutationSite;

/// How a site's arguments name the province whose info it writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Receiver {
    /// EDX is the province id: cooldown assignment writes its manager slot
    /// ([`info`]), and info construction builds that slot's info.
    Province,
    /// RCX is the province rail info; its province is its definition's
    /// id ([`province`]).
    Info,
    /// The daily countdown: it writes the info of every occurrence on the
    /// manager's cooldown list (`crailway_manager::COOLDOWN_PROVINCES`) and nothing else.
    CooldownList,
    /// The site writes infos it does not name, or the slot table itself.
    Every,
}

/// Every site with its receiver; each returns RAX unchanged from its
/// original, so a detour returns the original's value.
pub const MUTATIONS: [(MutationSite, Receiver); 9] = [
    // Cooldown assignment (manager, province, ticks): spill RBX to R9's home slot.
    (
        MutationSite {
            name: "set_province_on_cooldown",
            rva: crailway_manager::SET_PROVINCE_ON_COOLDOWN_RVA,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x20, 0x89, 0x54, 0x24, 0x10, 0x55, 0x56, 0x41, 0x56, 0x48,
                0x83, 0xec,
            ],
        },
        Receiver::Province,
    ),
    // Daily cooldown countdown (manager): spill RBX to R9's home slot.
    (
        MutationSite {
            name: "update_cooldowns",
            rva: crailway_manager::UPDATE_COOLDOWNS_RVA,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x20, 0x56, 0x57, 0x41, 0x56, 0x48, 0x83, 0xec, 0x30, 0x80,
                0x3d, 0xad,
            ],
        },
        Receiver::CooldownList,
    ),
    // Railway assignment (info, neighbour, level): copy the stack pointer
    // to R11, then save RBX and RSI; five bytes relocate.
    (
        MutationSite {
            name: "set_railway",
            rva: crate::cprovince_railway_info::SET_RAILWAY_RVA,
            sentinel: crate::cprovince_railway_info::SET_RAILWAY_SENTINEL,
        },
        Receiver::Info,
    ),
    // Info construction (info, province id): spill RBX to RDX's home slot.
    (
        MutationSite {
            name: "construct_info",
            rva: crate::cprovince_railway_info::CONSTRUCTOR_RVA,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x10, 0x48, 0x89, 0x4c, 0x24, 0x08, 0x55, 0x56, 0x57, 0x48,
                0x83, 0xec,
            ],
        },
        Receiver::Province,
    ),
    // Info deleting destructor (info, flags): spill RBX to RCX's home slot.
    (
        MutationSite {
            name: "destroy_info",
            rva: crate::cprovince_railway_info::DESTRUCTOR_RVA,
            sentinel: crate::cprovince_railway_info::DESTRUCTOR_SENTINEL,
        },
        Receiver::Info,
    ),
    // Info save reader (info, reader, token): spill RBX to RCX's home slot.
    // Revoke every certificate instead of trusting a definition the load may
    // not have set yet.
    (
        MutationSite {
            name: "read_info",
            rva: crate::cprovince_railway_info::LOAD_TOKEN_RVA,
            sentinel: crate::cprovince_railway_info::LOAD_TOKEN_SENTINEL,
        },
        Receiver::Every,
    ),
    // Manager construction (manager): spill RBX to R8's home slot.
    (
        MutationSite {
            name: "construct_manager",
            rva: crailway_manager::CONSTRUCTOR_RVA,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x18, 0x48, 0x89, 0x6c, 0x24, 0x20, 0x48, 0x89, 0x4c, 0x24,
                0x08, 0x56,
            ],
        },
        Receiver::Every,
    ),
    // Manager deleting destructor (manager, flags): spill RBX to RCX's home slot.
    (
        MutationSite {
            name: "destroy_manager",
            rva: crailway_manager::DESTRUCTOR_RVA,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x57, 0x48, 0x83, 0xec, 0x20, 0x48, 0x8d, 0x05, 0xaf,
                0x9e, 0xae,
            ],
        },
        Receiver::Every,
    ),
    // Manager token loading (manager, reader, token): spill RBX to RCX's home slot.
    (
        MutationSite {
            name: "read_manager",
            rva: crailway_manager::LOAD_TOKEN_RVA,
            sentinel: [
                0x48, 0x89, 0x5c, 0x24, 0x08, 0x55, 0x56, 0x57, 0x41, 0x54, 0x41, 0x55, 0x41, 0x56,
                0x41, 0x57,
            ],
        },
        Receiver::Every,
    ),
];

/// The province of `info`: its definition's
/// (`cprovince_railway_info::PROVINCE_RECORD`) id
/// (`province_definition::ID`), which the walk authenticates against the
/// info's slot. `None` when either does not read or the id is zero, which no
/// info has.
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

/// The info cooldown assignment writes for `province`: the manager slot
/// `table[province]`. A zero slot names no info, and the game writes nothing.
/// Return `None` for a negative id, unreadable or invalid table header, id
/// beyond the count (the game does not bound it), or unreadable slot.
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
    use crate::memory::SliceMemory;

    /// The sites already named beside their layouts keep those addresses
    /// and sentinels.
    #[test]
    fn sites_agree_with_their_layout_constants() {
        use crate::cprovince_railway_info as info;
        let site = |name: &str| {
            &MUTATIONS
                .iter()
                .find(|(site, _)| site.name == name)
                .unwrap()
                .0
        };
        assert_eq!(site("set_railway").sentinel, info::SET_RAILWAY_SENTINEL);
        assert_eq!(site("destroy_info").sentinel, info::DESTRUCTOR_SENTINEL);
        assert_eq!(site("read_info").sentinel, info::LOAD_TOKEN_SENTINEL);
        // The manager's and info's token readers share their first 16 bytes.
        assert_eq!(site("read_manager").sentinel, info::LOAD_TOKEN_SENTINEL);
        let mut rvas: Vec<usize> = MUTATIONS.iter().map(|(site, _)| site.rva).collect();
        rvas.sort_unstable();
        rvas.dedup();
        assert_eq!(rvas.len(), MUTATIONS.len());
    }

    /// An info names its province through its definition's id; a null or
    /// unreadable definition, or a zero id, names none.
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

    /// The setter and the constructor name their province in EDX, the
    /// countdown its list, and the save readers and the manager's lifetime
    /// sites nothing.
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

    /// The cooldown setter's receiver is the slot the native loads; an id
    /// it cannot bound or a table it cannot read names none.
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
        // A null slot names no info: the native writes nothing.
        assert_eq!(info(&memory, manager, 2), Some(0));
        assert_eq!(info(&memory, manager, 3), None);
        assert_eq!(info(&memory, manager, -1), None);
        assert_eq!(info(&memory, BASE + 0x1000, 1), None);
        // A count above the capacity is no table.
        bytes[header + 12..header + 16].copy_from_slice(&5_u32.to_le_bytes());
        let memory = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        assert_eq!(info(&memory, manager, 1), None);
    }
}
