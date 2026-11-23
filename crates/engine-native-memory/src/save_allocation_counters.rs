
use crate::{Memory, read};
use engine_sites::Rva;

pub const NEXT_ID_RVA: Rva = Rva(0x030f_f228);
pub const ID_COUNTER_RVA: Rva = Rva(0x034f_ff20);
pub const NAVY_ID: usize = 0x74c;
pub const LAND_COMBAT_ID: usize = 0x748;
pub const THEATER_GROUP_INDEX_RVA: Rva = Rva(0x030f_f22c);
pub const MILITARY_DEPLOYMENT_LINE_INDEX_RVA: Rva = Rva(0x0312_93b8);
pub const MILITARY_DEPLOYMENT_CONVEYOR_INDEX_RVA: Rva = Rva(0x0312_93bc);
pub const ORDER_INDEX_RVA: Rva = Rva(0x030f_f230);
pub const FRONT_INDEX_RVA: Rva = Rva(0x030f_f234);
pub const THEATRE_INDEX_RVA: Rva = Rva(0x030f_f238);
pub const COUNTRY_LEADER_INDEX_RVA: Rva = Rva(0x0312_9330);
pub const EQUIPMENT_VARIANT_INDEX_RVA: Rva = Rva(0x0312_9388);
pub const MULTIPLAYER_RANDOM_COUNT_RVA: Rva = Rva(0x0350_0360);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AllocationCounters {
    pub debug_current_ref_id: Option<u32>,
    pub unit: Option<i32>,
    pub order_index: Option<i32>,
    pub front_index: Option<i32>,
    pub theatre_index: Option<i32>,
    pub theater_group_index: Option<i32>,
    pub military_deployment_line_index: Option<i32>,
    pub military_deployment_conveyor_index: Option<i32>,
    pub equipment_variant_index: Option<i32>,
    pub country_leader_index: Option<i32>,
    pub navy_id: Option<i32>,
    pub land_combat_id: Option<i32>,
    pub multiplayer_random_count: Option<i32>,
}

pub fn read_counters(memory: &impl Memory, base: usize, game_state: usize) -> AllocationCounters {
    let global = |key: Rva| engine_sites::at(base, key).and_then(|at| read::<i32>(memory, at));
    let field = |offset: usize| {
        game_state
            .checked_add(offset)
            .and_then(|at| read::<i32>(memory, at))
    };
    AllocationCounters {
        debug_current_ref_id: engine_sites::at(base, ID_COUNTER_RVA)
            .and_then(|at| read::<u32>(memory, at)),
        unit: global(NEXT_ID_RVA),
        order_index: global(ORDER_INDEX_RVA),
        front_index: global(FRONT_INDEX_RVA),
        theatre_index: global(THEATRE_INDEX_RVA),
        theater_group_index: global(THEATER_GROUP_INDEX_RVA),
        military_deployment_line_index: global(MILITARY_DEPLOYMENT_LINE_INDEX_RVA),
        military_deployment_conveyor_index: global(MILITARY_DEPLOYMENT_CONVEYOR_INDEX_RVA),
        equipment_variant_index: global(EQUIPMENT_VARIANT_INDEX_RVA),
        country_leader_index: global(COUNTRY_LEADER_INDEX_RVA),
        navy_id: field(NAVY_ID),
        land_combat_id: field(LAND_COMBAT_ID),
        multiplayer_random_count: global(MULTIPLAYER_RANDOM_COUNT_RVA),
    }
}

impl AllocationCounters {
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
        let mut globals = vec![0_u8; 0x0351_0000];
        let put = |bytes: &mut Vec<u8>, rva: usize, value: i32| {
            bytes[rva..rva + 4].copy_from_slice(&value.to_le_bytes());
        };
        globals[ID_COUNTER_RVA.0..ID_COUNTER_RVA.0 + 4]
            .copy_from_slice(&u32::MAX.to_le_bytes());
        put(&mut globals, NEXT_ID_RVA.0, 4012);
        put(&mut globals, ORDER_INDEX_RVA.0, 7693);
        put(&mut globals, FRONT_INDEX_RVA.0, 8230);
        put(&mut globals, THEATRE_INDEX_RVA.0, 112);
        put(&mut globals, THEATER_GROUP_INDEX_RVA.0, 924);
        put(&mut globals, MILITARY_DEPLOYMENT_LINE_INDEX_RVA.0, 7841);
        put(&mut globals, MILITARY_DEPLOYMENT_CONVEYOR_INDEX_RVA.0, 744);
        put(&mut globals, EQUIPMENT_VARIANT_INDEX_RVA.0, 6295);
        put(&mut globals, COUNTRY_LEADER_INDEX_RVA.0, 1);
        put(&mut globals, MULTIPLAYER_RANDOM_COUNT_RVA.0, -1_333_963_091);
        let mut state = vec![0_u8; 0x800];
        state[NAVY_ID..NAVY_ID + 4]
            .copy_from_slice(&8263_i32.to_le_bytes());
        state[LAND_COMBAT_ID..LAND_COMBAT_ID + 4]
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
            (0, ID_COUNTER_RVA.0),
            (1, NEXT_ID_RVA.0),
            (10, STATE + NAVY_ID),
            (12, MULTIPLAYER_RANDOM_COUNT_RVA.0),
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
