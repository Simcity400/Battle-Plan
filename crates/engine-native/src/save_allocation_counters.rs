//! Monotonic allocation counters serialized in the game-state save header. The loader restores
//! twelve of thirteen counters to the same globals or setters. The unsigned reference ID is
//! different: loading token 0x2d4d writes the game-state word after
//! `cgame_state::NEXT_EVENT_INSTANCE`, while recording reads the ID counter. All other counters
//! retain the signed i32 format used by the save writer.
//!
//! These are passive scalar reads of globals and two game-state fields.
//! No engine function is called and no hook is installed.

use crate::{Memory, cgame_state, creference_registry, cship, read};

/// Theater-group index global, saved with token 0x35d2 and restored through its setter.
pub const THEATER_GROUP_INDEX_RVA: usize = 0x0308_7258;
/// Deployment-line index global, saved with token 0x34b3 and restored through its setter.
pub const MILITARY_DEPLOYMENT_LINE_INDEX_RVA: usize = 0x030b_1358;
/// Deployment-conveyor index global, saved with token 0x34b4 and restored through its setter.
pub const MILITARY_DEPLOYMENT_CONVEYOR_INDEX_RVA: usize = 0x030b_135c;
/// Order index global, saved with token 0x3036 and restored directly.
pub const ORDER_INDEX_RVA: usize = 0x0308_7260;
/// Front index global, saved with token 0x35ad and restored directly.
pub const FRONT_INDEX_RVA: usize = 0x0308_7264;
/// Theatre index global, saved with token 0x35ae and restored directly.
pub const THEATRE_INDEX_RVA: usize = 0x0308_7268;
/// Country-leader index global, saved with token 0x393b and restored directly.
pub const COUNTRY_LEADER_INDEX_RVA: usize = 0x030b_12d0;
/// Equipment-variant index global, saved with token 0x3643 and restored through its setter.
pub const EQUIPMENT_VARIANT_INDEX_RVA: usize = 0x030b_1328;
/// RNG draw counter, saved with token 0x2cc3; wraps as signed i32.
pub const MULTIPLAYER_RANDOM_COUNT_RVA: usize = 0x0345_2520;

/// One passive sample of the counters the save header serializer emits.
///
/// Every counter is read independently, so one unreadable global leaves exactly
/// that counter absent instead of discarding the whole sample, and an absent
/// counter is never a zero. `debug_current_ref_id` is the only unsigned field,
/// matching the unsigned formatter the writer uses for it; every other counter
/// keeps the signed i32 the writer prints.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AllocationCounters {
    /// Unsigned reference ID saved with token 0x2d4d from `creference_registry::ID_COUNTER_RVA`.
    pub debug_current_ref_id: Option<u32>,
    /// Shared type-0x33 unit ID high-water mark saved with token 0x28a3 from `cship::NEXT_ID_RVA`.
    pub unit: Option<i32>,
    pub order_index: Option<i32>,
    pub front_index: Option<i32>,
    pub theatre_index: Option<i32>,
    pub theater_group_index: Option<i32>,
    pub military_deployment_line_index: Option<i32>,
    pub military_deployment_conveyor_index: Option<i32>,
    pub equipment_variant_index: Option<i32>,
    pub country_leader_index: Option<i32>,
    /// Navy ID saved with token 0x34a5 from `cgame_state::NAVY_ID`.
    pub navy_id: Option<i32>,
    /// Land-combat ID saved with token 0x34a6 from `cgame_state::LAND_COMBAT_ID`.
    pub land_combat_id: Option<i32>,
    pub multiplayer_random_count: Option<i32>,
}

/// Reads the counters at the sampled tick. `game_state` is the already validated
/// singleton pointer; every other value is a module-relative global.
pub fn read_counters(memory: &impl Memory, base: usize, game_state: usize) -> AllocationCounters {
    let global = |rva: usize| base.checked_add(rva).and_then(|at| read::<i32>(memory, at));
    let field = |offset: usize| {
        game_state
            .checked_add(offset)
            .and_then(|at| read::<i32>(memory, at))
    };
    AllocationCounters {
        debug_current_ref_id: base
            .checked_add(creference_registry::ID_COUNTER_RVA)
            .and_then(|at| read::<u32>(memory, at)),
        unit: global(cship::NEXT_ID_RVA),
        order_index: global(ORDER_INDEX_RVA),
        front_index: global(FRONT_INDEX_RVA),
        theatre_index: global(THEATRE_INDEX_RVA),
        theater_group_index: global(THEATER_GROUP_INDEX_RVA),
        military_deployment_line_index: global(MILITARY_DEPLOYMENT_LINE_INDEX_RVA),
        military_deployment_conveyor_index: global(MILITARY_DEPLOYMENT_CONVEYOR_INDEX_RVA),
        equipment_variant_index: global(EQUIPMENT_VARIANT_INDEX_RVA),
        country_leader_index: global(COUNTRY_LEADER_INDEX_RVA),
        navy_id: field(cgame_state::NAVY_ID),
        land_combat_id: field(cgame_state::LAND_COMBAT_ID),
        multiplayer_random_count: global(MULTIPLAYER_RANDOM_COUNT_RVA),
    }
}

impl AllocationCounters {
    /// Every counter's presence, in the order the save header prints them.
    pub fn present(&self) -> [bool; 13] {
        [
            self.debug_current_ref_id.is_some(),
            self.unit.is_some(),
            self.order_index.is_some(),
            self.front_index.is_some(),
            self.theatre_index.is_some(),
            self.theater_group_index.is_some(),
            self.military_deployment_conveyor_index.is_some(),
            self.military_deployment_line_index.is_some(),
            self.equipment_variant_index.is_some(),
            self.country_leader_index.is_some(),
            self.navy_id.is_some(),
            self.land_combat_id.is_some(),
            self.multiplayer_random_count.is_some(),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::SliceMemory;

    struct Globals<'a> {
        bytes: &'a [u8],
        state: &'a [u8],
        state_base: usize,
        fail: usize,
    }

    impl Memory for Globals<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            if address == self.fail {
                return false;
            }
            if address >= self.state_base {
                return SliceMemory {
                    base: self.state_base,
                    bytes: self.state,
                }
                .copy(address, destination);
            }
            SliceMemory {
                base: 0,
                bytes: self.bytes,
            }
            .copy(address, destination)
        }
    }

    const STATE: usize = 0x1000_0000;

    fn fixture() -> (Vec<u8>, Vec<u8>) {
        let mut globals = vec![0_u8; 0x0346_0000];
        let put = |bytes: &mut Vec<u8>, rva: usize, value: i32| {
            bytes[rva..rva + 4].copy_from_slice(&value.to_le_bytes());
        };
        globals[creference_registry::ID_COUNTER_RVA..creference_registry::ID_COUNTER_RVA + 4]
            .copy_from_slice(&u32::MAX.to_le_bytes());
        put(&mut globals, cship::NEXT_ID_RVA, 4012);
        put(&mut globals, ORDER_INDEX_RVA, 7693);
        put(&mut globals, FRONT_INDEX_RVA, 8230);
        put(&mut globals, THEATRE_INDEX_RVA, 112);
        put(&mut globals, THEATER_GROUP_INDEX_RVA, 924);
        put(&mut globals, MILITARY_DEPLOYMENT_LINE_INDEX_RVA, 7841);
        put(&mut globals, MILITARY_DEPLOYMENT_CONVEYOR_INDEX_RVA, 744);
        put(&mut globals, EQUIPMENT_VARIANT_INDEX_RVA, 6295);
        put(&mut globals, COUNTRY_LEADER_INDEX_RVA, 1);
        put(&mut globals, MULTIPLAYER_RANDOM_COUNT_RVA, -1_333_963_091);
        let mut state = vec![0_u8; 0x800];
        state[cgame_state::NAVY_ID..cgame_state::NAVY_ID + 4]
            .copy_from_slice(&8263_i32.to_le_bytes());
        state[cgame_state::LAND_COMBAT_ID..cgame_state::LAND_COMBAT_ID + 4]
            .copy_from_slice(&15359_i32.to_le_bytes());
        (globals, state)
    }

    #[test]
    fn every_counter_keeps_the_width_its_save_writer_prints() {
        let (globals, state) = fixture();
        let memory = Globals {
            bytes: &globals,
            state: &state,
            state_base: STATE,
            fail: usize::MAX,
        };
        let counters = read_counters(&memory, 0, STATE);
        assert_eq!(counters.debug_current_ref_id, Some(u32::MAX));
        assert_eq!(counters.unit, Some(4012));
        assert_eq!(counters.order_index, Some(7693));
        assert_eq!(counters.front_index, Some(8230));
        assert_eq!(counters.theatre_index, Some(112));
        assert_eq!(counters.theater_group_index, Some(924));
        assert_eq!(counters.military_deployment_line_index, Some(7841));
        assert_eq!(counters.military_deployment_conveyor_index, Some(744));
        assert_eq!(counters.equipment_variant_index, Some(6295));
        assert_eq!(counters.country_leader_index, Some(1));
        assert_eq!(counters.navy_id, Some(8263));
        assert_eq!(counters.land_combat_id, Some(15359));
        assert_eq!(counters.multiplayer_random_count, Some(-1_333_963_091));
        assert_eq!(
            counters.present().iter().filter(|value| **value).count(),
            13
        );
    }

    #[test]
    fn an_unreadable_counter_is_absent_without_disturbing_the_others() {
        let (globals, state) = fixture();
        for (index, fail) in [
            (0, creference_registry::ID_COUNTER_RVA),
            (1, cship::NEXT_ID_RVA),
            (10, STATE + cgame_state::NAVY_ID),
            (12, MULTIPLAYER_RANDOM_COUNT_RVA),
        ] {
            let memory = Globals {
                bytes: &globals,
                state: &state,
                state_base: STATE,
                fail,
            };
            let present = read_counters(&memory, 0, STATE).present();
            assert!(!present[index], "{fail:#x}");
            assert_eq!(
                present.iter().filter(|value| **value).count(),
                12,
                "{fail:#x}"
            );
        }
    }
}
