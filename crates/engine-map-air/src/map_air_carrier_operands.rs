
use engine_native_memory::{Memory, read as read_native};
use engine_native_parts::modifier_accumulator;
use engine_sites::Rva;

pub const CARRIER_STANCE: usize = 0x4ec;
const FLEET_LEADER_REFERENCE: usize = 0xb0;
const LEADER_MODIFIER_COOLDOWN: usize = 0xe84;
const LEADER_MODIFIERS: usize = 0x418;
const SORTIE_EFFICIENCY_KEY: u32 = 0x11;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub carrier_stance: Option<i32>,
    pub leader_modifier: engine_native_memory::modifier_cache::Value,
    pub task_force_combats: Option<i32>,
    pub task_force_region: Option<u32>,
}

fn at(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .ok_or("map air carrier address overflow")
}

fn site(base: usize, key: Rva) -> Result<usize, &'static str> {
    engine_sites::at(base, key).ok_or("map air carrier address overflow")
}

fn field<T: engine_native_memory::memory::Raw>(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
) -> Result<T, &'static str> {
    read_native(memory, at(pointer, offset)?).ok_or("map air carrier field unreadable")
}

fn pointer(memory: &impl Memory, source: usize, offset: usize) -> Result<usize, &'static str> {
    usize::try_from(field::<u64>(memory, source, offset)?)
        .map_err(|_| "map air carrier pointer overflow")
}

fn authenticate(
    memory: &impl Memory,
    base: usize,
    object: usize,
    vtable_rva: Rva,
) -> Result<(), &'static str> {
    if object == 0 || field::<u64>(memory, object, 0)? != site(base, vtable_rva)? as u64 {
        return Err("map air carrier object identity mismatch");
    }
    Ok(())
}

pub fn read(
    memory: &impl Memory,
    base: usize,
    air_base: usize,
    air_base_handle: u64,
) -> Result<Option<Row>, &'static str> {
    authenticate(memory, base, air_base, engine_native_parts::cair_base_layout::VTABLE_RVA)?;
    if air_base_handle == 0
        || air_base_handle as u32 != engine_native_parts::cair_base_layout::TYPE
        || field::<u64>(memory, air_base, engine_native_parts::cair_base_layout::HANDLE)? != air_base_handle
    {
        return Err("map air base handle mismatch");
    }
    if pointer(memory, air_base, engine_native_air::cair_base::STATE)? != 0 {
        return Ok(Some(Row::default()));
    }
    let carrier = field::<i32>(memory, air_base, engine_native_air::cair_base::KIND)? != 1;

    let ship_handle = field::<u64>(memory, air_base, engine_native_parts::cair_base_layout::HOST_REFERENCE)?;
    if ship_handle == 0 {
        return Ok(None);
    }
    if ship_handle as u32 != engine_native::cship::HANDLE_TYPE {
        return Err("map air carrier host handle invalid");
    }
    let Some(ship) = engine_native_parts::creference_registry::resolve(memory, base, ship_handle)? else {
        return Ok(None);
    };
    authenticate(memory, base, ship, engine_native::cship::VTABLE_RVA)?;
    if field::<u64>(memory, ship, engine_native::cship::HANDLE)? != ship_handle
        || (carrier && pointer(memory, ship, engine_native::cship::CARRIER_AIR_BASE)? != air_base)
    {
        return Err("map air carrier ship backlink mismatch");
    }

    let task = pointer(memory, ship, engine_native::cship::TASK_FORCE)?;
    if task == 0 {
        return Ok(Some(Row::default()));
    }
    authenticate(memory, base, task, engine_native::ctask_force::VTABLE_RVA)?;
    if field::<u64>(memory, task, engine_native::ctask_force::SECONDARY_BASE)?
        != site(base, engine_native::ctask_force::SECONDARY_VTABLE_RVA)? as u64
    {
        return Err("map air carrier task secondary identity mismatch");
    }
    if field::<u64>(memory, task, engine_native::cunit::HANDLE)? == 0 {
        return Err("map air carrier task handle absent");
    }
    let carrier_stance = carrier
        .then(|| field::<i32>(memory, task, CARRIER_STANCE))
        .transpose()?;
    let combats = field::<i32>(memory, task, engine_native::cunit::COMBAT_REFERENCE_COUNT)?;
    let row = Row {
        carrier_stance,
        task_force_combats: Some(combats),
        task_force_region: if combats > 0 {
            location_region(memory, base, task)?
        } else {
            None
        },
        ..Row::default()
    };
    let fleet = pointer(memory, task, engine_native::ctask_force::FLEET)?;
    if fleet == 0 {
        return Ok(Some(row));
    }
    authenticate(memory, base, fleet, engine_native::cfleet::VTABLE_RVA)?;
    let fleet_handle = field::<u64>(memory, fleet, engine_native::cfleet::HANDLE)?;
    if fleet_handle == 0 || fleet_handle as u32 != engine_native::cfleet::HANDLE_TYPE {
        return Err("map air carrier fleet handle invalid");
    }
    let leader_handle = field::<u64>(memory, fleet, FLEET_LEADER_REFERENCE)?;
    let leader_modifier = if leader_handle == 0 {
        engine_native_memory::modifier_cache::Value::default()
    } else if let Some(leader) = engine_native_parts::creference_registry::resolve(memory, base, leader_handle)? {
        authenticate(memory, base, leader, engine_native_parts::cnavy_leader::VTABLE_RVA)?;
        if engine_native::cunit_leader::handle(memory, base, leader)? != leader_handle {
            return Err("map air carrier leader handle mismatch");
        }
        if field::<i32>(memory, leader, LEADER_MODIFIER_COOLDOWN)? != 0 {
            engine_native_memory::modifier_cache::Value::default()
        } else {
            modifier_accumulator::lookup_at(
                memory,
                at(leader, LEADER_MODIFIERS)?,
                SORTIE_EFFICIENCY_KEY,
            )?
        }
    } else {
        engine_native_memory::modifier_cache::Value::default()
    };
    Ok(Some(Row {
        leader_modifier,
        ..row
    }))
}

fn location_region(
    memory: &impl Memory,
    base: usize,
    task: usize,
) -> Result<Option<u32>, &'static str> {
    let province = pointer(memory, task, engine_native::cunit::LOCATION)?;
    authenticate(memory, base, province, engine_native::cprovince::VTABLE_RVA)?;
    let region = pointer(memory, province, engine_native::cprovince::STRATEGIC_REGION)?;
    if region == 0 {
        return Ok(None);
    }
    authenticate(memory, base, region, engine_native_parts::cstrategic_region::VTABLE_RVA)?;
    field(memory, region, engine_native_parts::cstrategic_region::ID).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_parts::{
        creference_registry as registry,
        reference_bucket as bucket,
    };
    use std::collections::BTreeMap;

    const BASE: usize = 0;
    const AIR_BASE: usize = 0x1000;
    const SHIP: usize = 0x2000;
    const TASK: usize = 0x3000;
    const FLEET: usize = 0x4000;
    const LEADER: usize = 0x5000;
    const SHIP_TABLE: usize = 0x6000;
    const LEADER_TABLE: usize = 0x7000;
    const SHIP_BUCKETS: usize = 0x8000;
    const LEADER_BUCKETS: usize = 0x9000;
    const MODIFIER_ENTRIES: usize = 0xa000;
    const PROVINCE: usize = 0xb000;
    const REGION: usize = 0xc000;
    const AIR_BASE_HANDLE: u64 = (4_u64 << 32) | engine_native_parts::cair_base_layout::TYPE as u64;
    const SHIP_HANDLE: u64 = (5_u64 << 32) | engine_native::cship::HANDLE_TYPE as u64;
    const LEADER_HANDLE: u64 = (6_u64 << 32) | engine_native::cunit_leader::REFERENCE_TYPE as u64;

    #[derive(Default)]
    struct Fixture {
        bytes: BTreeMap<usize, u8>,
    }

    impl Memory for Fixture {
        fn copy(&self, pointer: usize, target: &mut [u8]) -> bool {
            for (index, byte) in target.iter_mut().enumerate() {
                let Some(value) = self.bytes.get(&(pointer + index)) else {
                    return false;
                };
                *byte = *value;
            }
            true
        }
    }

    impl Fixture {
        fn put(&mut self, address: usize, bytes: &[u8]) {
            self.bytes.extend(
                bytes
                    .iter()
                    .enumerate()
                    .map(|(index, &b)| (address + index, b)),
            );
        }

        fn q(&mut self, address: usize, value: u64) {
            self.put(address, &value.to_le_bytes());
        }

        fn d(&mut self, address: usize, value: u32) {
            self.put(address, &value.to_le_bytes());
        }

        fn registry(&mut self, handle: u64, object: usize, table: usize, buckets: usize) {
            let root = if (handle as u32) < registry::LOW_ROOT_COUNT {
                registry::LOW_ROOTS_RVA.0 + (handle as u32 as usize) * 8
            } else {
                registry::HIGH_ROOT_RVA.0
            };
            self.q(root, table as u64);
            self.q(table + registry::ENTRIES, buckets as u64);
            self.d(table + registry::COUNT, 1);
            self.d(table + registry::MASK, 7);
            self.put(table + registry::OVERFLOW_DISTANCE, &[5]);
            self.put(buckets, &[0; 14 * bucket::STRIDE]);
            self.put(buckets + 13 * bucket::STRIDE + bucket::DISTANCE, &[255]);
            let hash = handle
                .rotate_left(32)
                .wrapping_mul(registry::HASH_MULTIPLIER);
            let slot = (((hash >> 32) as u32 ^ hash as u32) & 7) as usize;
            self.put(buckets + slot * bucket::STRIDE + bucket::DISTANCE, &[1]);
            self.q(buckets + slot * bucket::STRIDE + bucket::TYPE, handle);
            self.q(
                buckets + slot * bucket::STRIDE + bucket::OBJECT,
                object as u64,
            );
        }

        fn full() -> Self {
            let mut memory = Self::default();
            memory.q(AIR_BASE, engine_native_parts::cair_base_layout::VTABLE_RVA.0 as u64);
            memory.q(AIR_BASE + engine_native_parts::cair_base_layout::HANDLE, AIR_BASE_HANDLE);
            memory.q(AIR_BASE + engine_native_air::cair_base::STATE, 0);
            memory.d(AIR_BASE + engine_native_air::cair_base::KIND, 0);
            memory.q(AIR_BASE + engine_native_parts::cair_base_layout::HOST_REFERENCE, SHIP_HANDLE);
            memory.registry(SHIP_HANDLE, SHIP, SHIP_TABLE, SHIP_BUCKETS);
            memory.q(SHIP, engine_native::cship::VTABLE_RVA.0 as u64);
            memory.q(SHIP + engine_native::cship::HANDLE, SHIP_HANDLE);
            memory.q(SHIP + engine_native::cship::CARRIER_AIR_BASE, AIR_BASE as u64);
            memory.q(SHIP + engine_native::cship::TASK_FORCE, TASK as u64);
            memory.q(TASK, engine_native::ctask_force::VTABLE_RVA.0 as u64);
            memory.q(
                TASK + engine_native::ctask_force::SECONDARY_BASE,
                engine_native::ctask_force::SECONDARY_VTABLE_RVA.0 as u64,
            );
            memory.q(TASK + engine_native::cunit::HANDLE, (7_u64 << 32) | 61);
            memory.q(TASK + engine_native::ctask_force::FLEET, FLEET as u64);
            memory.d(TASK + CARRIER_STANCE, 2);
            memory.d(TASK + engine_native::cunit::COMBAT_REFERENCE_COUNT, 0);
            memory.q(FLEET, engine_native::cfleet::VTABLE_RVA.0 as u64);
            memory.q(FLEET + engine_native::cfleet::HANDLE, (8_u64 << 32) | 61);
            memory.q(FLEET + FLEET_LEADER_REFERENCE, LEADER_HANDLE);
            memory.registry(LEADER_HANDLE, LEADER, LEADER_TABLE, LEADER_BUCKETS);
            memory.q(LEADER, engine_native_parts::cnavy_leader::VTABLE_RVA.0 as u64);
            memory.q(LEADER + engine_native::cunit_leader::TYPE, LEADER_HANDLE);
            memory.d(LEADER + LEADER_MODIFIER_COOLDOWN, 0);
            memory.q(LEADER + LEADER_MODIFIERS, MODIFIER_ENTRIES as u64);
            memory.d(LEADER + LEADER_MODIFIERS + 8, 1);
            memory.d(LEADER + LEADER_MODIFIERS + 12, 1);
            memory.d(MODIFIER_ENTRIES, SORTIE_EFFICIENCY_KEY);
            memory.d(MODIFIER_ENTRIES + 4, 0);
            memory.q(MODIFIER_ENTRIES + 8, (-27_500_i64) as u64);
            memory
        }
    }

    #[test]
    fn state_base_stops_before_unused_host_graph() {
        let mut memory = Fixture::default();
        memory.q(AIR_BASE, engine_native_parts::cair_base_layout::VTABLE_RVA.0 as u64);
        memory.q(AIR_BASE + engine_native_parts::cair_base_layout::HANDLE, AIR_BASE_HANDLE);
        memory.q(AIR_BASE + engine_native_air::cair_base::STATE, 0xbeef);
        assert_eq!(
            read(&memory, BASE, AIR_BASE, AIR_BASE_HANDLE),
            Ok(Some(Row::default()))
        );
    }

    #[test]
    fn host_base_reads_raw_stance_and_eligible_leader_modifier() {
        let memory = Fixture::full();
        assert_eq!(
            read(&memory, BASE, AIR_BASE, AIR_BASE_HANDLE),
            Ok(Some(Row {
                carrier_stance: Some(2),
                leader_modifier: engine_native_memory::modifier_cache::Value {
                    raw: -27_500,
                    present: true,
                },
                task_force_combats: Some(0),
                task_force_region: None,
            }))
        );
    }

    #[test]
    fn rocket_host_keeps_the_leader_modifier_without_stance_or_carrier_backlink() {
        let mut memory = Fixture::full();
        memory.d(AIR_BASE + engine_native_air::cair_base::KIND, 1);
        memory.q(SHIP + engine_native::cship::CARRIER_AIR_BASE, 0);
        let row = read(&memory, BASE, AIR_BASE, AIR_BASE_HANDLE)
            .unwrap()
            .unwrap();
        assert_eq!(row.carrier_stance, None);
        assert_eq!(row.leader_modifier.raw, -27_500);
    }

    #[test]
    fn no_leader_veto_and_absent_key_are_distinct_complete_inputs() {
        let mut memory = Fixture::full();
        memory.q(TASK + engine_native::ctask_force::FLEET, 0);
        assert_eq!(
            read(&memory, BASE, AIR_BASE, AIR_BASE_HANDLE),
            Ok(Some(Row {
                carrier_stance: Some(2),
                leader_modifier: engine_native_memory::modifier_cache::Value::default(),
                task_force_combats: Some(0),
                task_force_region: None,
            }))
        );
        memory.q(TASK + engine_native::ctask_force::FLEET, FLEET as u64);
        memory.q(FLEET + FLEET_LEADER_REFERENCE, 0);
        assert_eq!(
            read(&memory, BASE, AIR_BASE, AIR_BASE_HANDLE),
            Ok(Some(Row {
                carrier_stance: Some(2),
                leader_modifier: engine_native_memory::modifier_cache::Value::default(),
                task_force_combats: Some(0),
                task_force_region: None,
            }))
        );
        memory.q(FLEET + FLEET_LEADER_REFERENCE, LEADER_HANDLE);
        memory.d(LEADER + LEADER_MODIFIER_COOLDOWN, 1);
        memory.bytes.remove(&(LEADER + LEADER_MODIFIERS));
        assert_eq!(
            read(&memory, BASE, AIR_BASE, AIR_BASE_HANDLE)
                .unwrap()
                .unwrap()
                .leader_modifier,
            engine_native_memory::modifier_cache::Value::default()
        );
        memory.d(LEADER + LEADER_MODIFIER_COOLDOWN, 0);
        memory.q(LEADER + LEADER_MODIFIERS, MODIFIER_ENTRIES as u64);
        memory.d(MODIFIER_ENTRIES, SORTIE_EFFICIENCY_KEY + 1);
        assert_eq!(
            read(&memory, BASE, AIR_BASE, AIR_BASE_HANDLE)
                .unwrap()
                .unwrap()
                .leader_modifier,
            engine_native_memory::modifier_cache::Value::default()
        );
    }

    #[test]
    fn a_null_task_force_has_no_host_operands_and_combat_reads_the_location_region() {
        let mut memory = Fixture::full();
        memory.q(SHIP + engine_native::cship::TASK_FORCE, 0);
        assert_eq!(
            read(&memory, BASE, AIR_BASE, AIR_BASE_HANDLE),
            Ok(Some(Row::default()))
        );
        memory.q(SHIP + engine_native::cship::TASK_FORCE, TASK as u64);
        memory.d(TASK + engine_native::cunit::COMBAT_REFERENCE_COUNT, 2);
        memory.q(TASK + engine_native::cunit::LOCATION, PROVINCE as u64);
        memory.q(PROVINCE, engine_native::cprovince::VTABLE_RVA.0 as u64);
        memory.q(PROVINCE + engine_native::cprovince::STRATEGIC_REGION, 0);
        let row = read(&memory, BASE, AIR_BASE, AIR_BASE_HANDLE)
            .unwrap()
            .unwrap();
        assert_eq!(
            (row.task_force_combats, row.task_force_region),
            (Some(2), None)
        );
        memory.q(PROVINCE + engine_native::cprovince::STRATEGIC_REGION, REGION as u64);
        memory.q(REGION, engine_native_parts::cstrategic_region::VTABLE_RVA.0 as u64);
        memory.d(REGION + engine_native_parts::cstrategic_region::ID, 90);
        let row = read(&memory, BASE, AIR_BASE, AIR_BASE_HANDLE)
            .unwrap()
            .unwrap();
        assert_eq!(row.task_force_region, Some(90));
        memory.q(REGION, 0);
        assert!(read(&memory, BASE, AIR_BASE, AIR_BASE_HANDLE).is_err());
    }

    #[test]
    fn a_stale_host_is_omitted_not_an_error() {
        let mut memory = Fixture::full();
        memory.q(
            registry::LOW_ROOTS_RVA.0 + (SHIP_HANDLE as u32 as usize) * 8,
            0,
        );
        assert_eq!(read(&memory, BASE, AIR_BASE, AIR_BASE_HANDLE), Ok(None));
        memory.q(AIR_BASE + engine_native_parts::cair_base_layout::HOST_REFERENCE, 0);
        assert_eq!(read(&memory, BASE, AIR_BASE, AIR_BASE_HANDLE), Ok(None));
    }

    #[test]
    fn graph_aliases_and_short_reads_fail_closed() {
        for (fault, expected) in [
            (0, "map air base handle mismatch"),
            (1, "map air carrier ship backlink mismatch"),
            (2, "map air carrier task secondary identity mismatch"),
            (3, "map air carrier fleet handle invalid"),
            (4, "map air carrier leader handle mismatch"),
            (5, "map air carrier field unreadable"),
            (6, "modifier accumulator entry unreadable"),
        ] {
            let mut memory = Fixture::full();
            match fault {
                0 => memory.q(AIR_BASE + engine_native_parts::cair_base_layout::HANDLE, SHIP_HANDLE),
                1 => memory.q(SHIP + engine_native::cship::CARRIER_AIR_BASE, 0xbeef),
                2 => memory.q(TASK + engine_native::ctask_force::SECONDARY_BASE, 0xbeef),
                3 => memory.q(FLEET + engine_native::cfleet::HANDLE, 0),
                4 => memory.q(
                    LEADER + engine_native::cunit_leader::TYPE,
                    (9_u64 << 32) | engine_native::cunit_leader::REFERENCE_TYPE as u64,
                ),
                5 => {
                    memory.bytes.remove(&(TASK + CARRIER_STANCE + 3));
                }
                6 => {
                    memory.bytes.remove(&(MODIFIER_ENTRIES + 15));
                }
                _ => unreachable!(),
            }
            assert_eq!(
                read(&memory, BASE, AIR_BASE, AIR_BASE_HANDLE),
                Err(expected),
                "fault {fault}"
            );
        }
    }
}
