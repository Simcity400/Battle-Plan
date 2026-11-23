
use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    memory::{RecordField, RecordLayout, RecordSources, read_slice},
    read,
    walk::{Buffer, Outcome},
};
use engine_native_parts::cbuilding;
use engine_native::{
    cprovince::{buildings as cbuilding_status, self},
    cgame_state,
    cstate,
};
use engine_native_air::{craid_system, craid_target};

pub const TARGETS: usize = 0x08;
pub const SCOPE: usize = 0x08;
pub const VALID: usize = 0x31;
pub const DETECTED: usize = 0x38;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub ordinal: u32,
    pub valid: u8,
    pub scope: craid_target::Row,
    pub detected_start: usize,
    pub detected_len: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Walk {
    pub outcome: Outcome,
    pub detected_len: usize,
    pub required_rows: usize,
    pub required_detected: usize,
    pub refused_leaders: u32,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Demand {
    pub rows: usize,
    pub detected: usize,
}

pub fn demand(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
) -> Result<Demand, &'static str> {
    let system = game_state
        .checked_add(cgame_state::RAID_SYSTEM)
        .and_then(|at| read::<u64>(memory, at))
        .and_then(|p| usize::try_from(p).ok())
        .filter(|&p| p != 0)
        .ok_or("raid system unreadable or null")?;
    let expected =
        engine_sites::at(base, craid_system::VTABLE_RVA).ok_or("raid system vtable mismatch")?;
    if read::<u64>(memory, system) != Some(expected as u64) {
        return Err("raid system vtable mismatch");
    }
    let targets = system
        .checked_add(TARGETS)
        .and_then(|at| Array::read(memory, at, 8))
        .ok_or("raid target array unreadable or invalid")?;
    let mut demand = Demand {
        rows: targets.count as usize,
        detected: 0,
    };
    engine_native_memory::cpdx_array::pointers(
        memory,
        targets,
        "raid target pointers unreadable",
        |_, pointer| {
            if pointer == 0 {
                return Err("raid target pointer null or invalid");
            }
            let bits = pointer
                .checked_add(DETECTED)
                .and_then(|at| Array::read(memory, at, 1))
                .ok_or("raid target detection vector invalid")?;
            demand.detected = demand
                .detected
                .checked_add(bits.count as usize)
                .ok_or("raid target detection demand overflow")?;
            Ok(())
        },
    )?;
    Ok(demand)
}

const fn field(source_offset: usize, destination_offset: usize, len: usize) -> RecordField {
    RecordField {
        source_offset,
        destination_offset,
        len,
    }
}
const TARGET_FIELDS: [RecordField; 3] = [
    field(SCOPE, 0, 0x28),
    field(VALID, 0x28, 1),
    field(DETECTED, 0x29, 16),
];
const TARGET_STRIDE: usize = 0x39;
const BUILDING_FIELDS: [RecordField; 3] = [
    field(0, 0, 8),
    field(cbuilding::TYPE_ID, 8, 4),
    field(cbuilding::BUILDING_STATUS, 12, 8),
];
const STATUS_FIELDS: [RecordField; 4] = [
    field(0, 0, 8),
    field(cbuilding_status::FLAGS, 8, 4),
    field(cbuilding_status::ID, 12, 4),
    field(cbuilding_status::COUNTRY, 16, 4),
];
const PROVINCE_FIELDS: [RecordField; 2] = [field(0, 0, 8), field(cprovince::ID, 8, 4)];
const STATE_FIELDS: [RecordField; 2] = [field(0, 0, 8), field(cstate::ID, 8, 4)];
const TARGET_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&TARGET_FIELDS, TARGET_STRIDE).expect("valid target layout");
const BUILDING_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&BUILDING_FIELDS, 20).expect("valid building layout");
const STATUS_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&STATUS_FIELDS, 20).expect("valid status layout");
const PROVINCE_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&PROVINCE_FIELDS, 12).expect("valid province layout");
const STATE_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&STATE_FIELDS, 12).expect("valid state layout");
const BATCH: usize = 64;

fn word64(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap())
}
fn word32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

fn identified<'b>(
    memory: &impl Memory,
    pointers: &[usize],
    layout: &RecordLayout<'_>,
    stride: usize,
    vtable: u64,
    out: &'b mut [u8],
    what: &'static str,
) -> Result<&'b [u8], &'static str> {
    let fields = layout.fields().len();
    let out = &mut out[..pointers.len() * stride];
    if memory.copy_records(RecordSources::Pointers(pointers), layout, out) < pointers.len() * fields
    {
        return Err(what);
    }
    if out
        .chunks_exact(stride)
        .any(|record| word64(record, 0) != vtable)
    {
        return Err(what);
    }
    Ok(out)
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    rows: &mut Vec<Row>,
    detected: &mut Vec<u8>,
) -> Walk {
    walk_buffer(
        memory,
        base,
        game_state,
        Buffer::Growing(rows),
        Buffer::Growing(detected),
    )
}

fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut rows: Buffer<'_, Row>,
    mut detected: Buffer<'_, u8>,
) -> Walk {
    let mut walk = Walk {
        outcome: Outcome::Rows { len: 0 },
        detected_len: 0,
        required_rows: 0,
        required_detected: 0,
        refused_leaders: 0,
    };
    let result = (|| {
        let system = game_state
            .checked_add(cgame_state::RAID_SYSTEM)
            .and_then(|at| read::<u64>(memory, at))
            .and_then(|pointer| usize::try_from(pointer).ok())
            .filter(|&pointer| pointer != 0)
            .ok_or("raid system unreadable or null")?;
        let vtable = |rva| {
            engine_sites::at(base, rva)
                .map(|address| address as u64)
                .ok_or("module address overflow")
        };
        if read::<u64>(memory, system) != Some(vtable(craid_system::VTABLE_RVA)?) {
            return Err("raid system vtable mismatch");
        }
        let targets = system
            .checked_add(TARGETS)
            .and_then(|at| Array::read(memory, at, 8))
            .ok_or("raid target array unreadable or invalid")?;
        walk.required_rows = targets.count as usize;
        rows.ensure(walk.required_rows)?;
        let mut slots = [0_u8; BATCH * 8];
        let mut pointers = [0_usize; BATCH];
        let mut records = [0_u8; BATCH * TARGET_STRIDE];
        let mut arms = [0_usize; BATCH];
        let mut owners = [0_usize; BATCH];
        let mut objects = [0_u8; BATCH * 20];
        let mut statuses = [0_u8; BATCH * 20];
        let (mut len, mut unread) = (0, 0_u32);
        let mut first = 0;
        while first < targets.count {
            let count = ((targets.count - first) as usize).min(BATCH);
            let address = targets
                .element(first, 8)
                .ok_or("raid target address overflow")?;
            if !read_slice(memory, address, &mut slots[..count * 8]) {
                return Err("raid target pointers unreadable");
            }
            for (pointer, chunk) in pointers.iter_mut().zip(slots[..count * 8].chunks_exact(8)) {
                *pointer = usize::try_from(u64::from_le_bytes(chunk.try_into().unwrap()))
                    .ok()
                    .filter(|&pointer| pointer != 0)
                    .ok_or("raid target pointer null or invalid")?;
            }
            let records = &mut records[..count * TARGET_STRIDE];
            if memory.copy_records(
                RecordSources::Pointers(&pointers[..count]),
                &TARGET_LAYOUT,
                records,
            ) < count * TARGET_FIELDS.len()
            {
                return Err("raid target fields unreadable");
            }
            let mut batch = [Row::default(); BATCH];
            for (index, record) in records.chunks_exact(TARGET_STRIDE).enumerate() {
                if record[0x28] > 1 {
                    return Err("raid target valid byte invalid");
                }
                batch[index].ordinal = first + index as u32;
                batch[index].valid = record[0x28];
                batch[index].scope.leader_reference = word64(record, 0x18);
            }
            let mut arm = |offset: usize,
                           layout: &RecordLayout<'_>,
                           stride: usize,
                           rva: engine_sites::Rva,
                           what: &'static str,
                           apply: &mut dyn FnMut(&mut Row, &[u8])|
             -> Result<(), &'static str> {
                let mut rows_of = [0_usize; BATCH];
                let mut named = 0;
                for (index, record) in records.chunks_exact(TARGET_STRIDE).enumerate() {
                    let pointer = usize::try_from(word64(record, offset))
                        .map_err(|_| "raid scope arm invalid")?;
                    if pointer != 0 {
                        arms[named] = pointer;
                        rows_of[named] = index;
                        named += 1;
                    }
                }
                let read = identified(
                    memory,
                    &arms[..named],
                    layout,
                    stride,
                    vtable(rva)?,
                    &mut objects,
                    what,
                )?;
                for (record, &index) in read.chunks_exact(stride).zip(&rows_of[..named]) {
                    apply(&mut batch[index], record);
                }
                Ok(())
            };
            arm(
                craid_target::PROVINCE_ARM,
                &PROVINCE_LAYOUT,
                12,
                cprovince::VTABLE_RVA,
                "raid target province arm is not a province",
                &mut |row, record| row.scope.province = word32(record, 8),
            )?;
            arm(
                craid_target::LEADER_PROVINCE_ARM,
                &PROVINCE_LAYOUT,
                12,
                cprovince::VTABLE_RVA,
                "raid target leader province is not a province",
                &mut |row, record| row.scope.leader_province = word32(record, 8),
            )?;
            arm(
                craid_target::STATE_ARM,
                &STATE_LAYOUT,
                12,
                cstate::VTABLE_RVA,
                "raid target state arm is not a state",
                &mut |row, record| row.scope.state = word32(record, 8),
            )?;
            let mut named = 0;
            let mut building_rows = [0_usize; BATCH];
            arm(
                craid_target::BUILDING,
                &BUILDING_LAYOUT,
                20,
                cbuilding::VTABLE_RVA,
                "raid target building arm is not a building",
                &mut |row, record| {
                    row.scope.building_type = word32(record, 8);
                    owners[named] = word64(record, 12) as usize;
                    building_rows[named] = row.ordinal as usize - first as usize;
                    named += 1;
                },
            )?;
            if owners[..named].contains(&0) {
                return Err("raid target building status null");
            }
            if building_rows[..named]
                .iter()
                .any(|&index| batch[index].scope.building_type == 0)
            {
                return Err("raid target building type token zero");
            }
            let read = identified(
                memory,
                &owners[..named],
                &STATUS_LAYOUT,
                20,
                vtable(cbuilding_status::VTABLE_RVA)?,
                &mut statuses,
                "raid target building status unreadable",
            )?;
            for (status, &index) in read.chunks_exact(20).zip(&building_rows[..named]) {
                let scope = &mut batch[index].scope;
                scope.building_flags = word32(status, 8);
                scope.building_location = word32(status, 12);
                scope.building_country = word32(status, 16) as i32;
            }
            for (index, record) in records.chunks_exact(TARGET_STRIDE).enumerate() {
                let mut row = batch[index];
                let scope = &row.scope;
                if craid_target::reads_leader(scope.building_type != 0, scope.province, scope.state)
                {
                    match craid_target::leader_captor(memory, base, scope.leader_reference) {
                        Ok(captor) => row.scope.leader_country = captor,
                        Err(()) => {
                            unread += 1;
                            walk.refused_leaders += 1;
                            continue;
                        }
                    }
                }
                let bits = Array::from_header(record[0x29..0x39].try_into().unwrap(), 1)
                    .ok_or("raid target detection vector invalid")?;
                let size = bits.count as usize;
                walk.required_detected = walk
                    .required_detected
                    .checked_add(size)
                    .ok_or("raid target detection demand overflow")?;
                let end = walk
                    .detected_len
                    .checked_add(size)
                    .ok_or("raid target detection demand overflow")?;
                detected.ensure(end)?;
                if len == rows.len() || end > detected.len() {
                    unread += 1;
                    continue;
                }
                if size != 0 && !memory.copy(bits.data, &mut detected[walk.detected_len..end]) {
                    return Err("raid target detection bytes unreadable");
                }
                row.detected_start = walk.detected_len;
                row.detected_len = size;
                rows[len] = row;
                walk.detected_len = end;
                len += 1;
            }
            first += count as u32;
        }
        Ok(if unread == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated {
                len,
                unread_rows: unread,
            }
        })
    })();
    walk.outcome = result.unwrap_or_else(|reason| Outcome::Unavailable { reason });
    walk
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;

    const STATE: usize = 0x100;
    const SYSTEM: usize = 0x800;
    const POINTERS: usize = 0x1000;
    const TARGET: usize = 0x1100;
    const BUILDING_AT: usize = 0x1400;
    const PROVINCE_AT: usize = 0x1600;
    const STATUS_AT: usize = 0x1800;
    const BITS: usize = 0x1c00;

    fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
        bytes[at..at + value.len()].copy_from_slice(value);
    }
    fn header(bytes: &mut [u8], at: usize, data: usize, count: u32) {
        put(bytes, at, &(data as u64).to_le_bytes());
        put(bytes, at + 8, &count.to_le_bytes());
        put(bytes, at + 12, &count.to_le_bytes());
    }
    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x2000];
        put(
            &mut bytes,
            STATE + cgame_state::RAID_SYSTEM,
            &(SYSTEM as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            SYSTEM,
            &(craid_system::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        header(&mut bytes, SYSTEM + TARGETS, POINTERS, 2);
        for ordinal in 0..2 {
            let target = TARGET + ordinal * 0x80;
            put(
                &mut bytes,
                POINTERS + ordinal * 8,
                &(target as u64).to_le_bytes(),
            );
        }
        put(
            &mut bytes,
            TARGET + SCOPE + craid_target::PROVINCE_ARM,
            &(PROVINCE_AT as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            PROVINCE_AT,
            &(cprovince::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            PROVINCE_AT + cprovince::ID,
            &6521_u32.to_le_bytes(),
        );
        let named = TARGET + 0x80;
        put(&mut bytes, named + VALID, &[1]);
        put(
            &mut bytes,
            named + SCOPE + craid_target::BUILDING,
            &(BUILDING_AT as u64).to_le_bytes(),
        );
        header(&mut bytes, named + DETECTED, BITS, 2);
        put(&mut bytes, BITS, &[0x80, 1]);
        put(
            &mut bytes,
            BUILDING_AT,
            &(cbuilding::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            BUILDING_AT + cbuilding::TYPE_ID,
            &0x2fb6_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            BUILDING_AT + cbuilding::BUILDING_STATUS,
            &(STATUS_AT as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            STATUS_AT,
            &(cbuilding_status::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            STATUS_AT + cbuilding_status::FLAGS,
            &1_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            STATUS_AT + cbuilding_status::ID,
            &9421_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            STATUS_AT + cbuilding_status::COUNTRY,
            &7_u32.to_le_bytes(),
        );
        bytes
    }
    fn run(bytes: &[u8], rows: &mut Vec<Row>, detected: &mut Vec<u8>) -> Walk {
        walk(&SliceMemory { base: 0, bytes }, 0, STATE, rows, detected)
    }

    #[test]
    fn every_target_keeps_its_ordinal_scope_valid_byte_and_bits() {
        let mut rows = vec![Row::default(); 2];
        let mut detected = vec![0; 2];
        let result = run(&fixture(), &mut rows, &mut detected);
        assert_eq!(result.outcome, Outcome::Rows { len: 2 });
        assert_eq!((rows[0].valid, rows[0].scope.province), (0, 6521));
        assert_eq!(rows[0].detected_len, 0);
        assert_eq!(rows[1].ordinal, 1);
        assert_eq!(rows[1].valid, 1);
        assert_eq!(rows[1].scope.building_type, 0x2fb6);
        assert_eq!(rows[1].scope.building_location, 9421);
        assert_eq!(rows[1].scope.building_flags, 1);
        assert_eq!(rows[1].scope.building_country, 7);
        assert_eq!(rows[1].scope.leader_country, None);
        assert_eq!((rows[1].detected_start, rows[1].detected_len), (0, 2));
        assert_eq!(detected, [0x80, 1]);
    }

    #[test]
    fn row_and_byte_shortfalls_grow_to_the_demand() {
        let result = run(&fixture(), &mut vec![Row::default(); 1], &mut vec![0; 1]);
        assert_eq!(result.outcome, Outcome::Rows { len: 2 });
        assert_eq!((result.required_rows, result.required_detected), (2, 2));
    }

    #[test]
    fn a_foreign_arm_or_short_bits_fail_the_table() {
        let mut bytes = fixture();
        put(&mut bytes, STATUS_AT, &3_u64.to_le_bytes());
        let result = run(&bytes, &mut vec![Row::default(); 2], &mut vec![0; 2]);
        assert!(matches!(result.outcome, Outcome::Unavailable { .. }));
        let mut bytes = fixture();
        put(&mut bytes, PROVINCE_AT, &3_u64.to_le_bytes());
        let result = run(&bytes, &mut vec![Row::default(); 2], &mut vec![0; 2]);
        assert!(matches!(result.outcome, Outcome::Unavailable { .. }));
        let bytes = fixture();
        let result = run(&bytes[..BITS + 1], &mut vec![Row::default(); 2], &mut vec![0; 2]);
        assert!(matches!(result.outcome, Outcome::Unavailable { .. }));
    }

    fn scaled(targets: usize, countries: usize) -> Vec<u8> {
        let width = countries.div_ceil(8);
        let pointers = 0x1000;
        let records = pointers + targets * 8;
        let objects = records + targets * 0x80;
        let bits = objects + targets * 0x40;
        let mut bytes = vec![0; bits + targets.div_ceil(10) * width + 0x100];
        put(
            &mut bytes,
            STATE + cgame_state::RAID_SYSTEM,
            &(SYSTEM as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            SYSTEM,
            &(craid_system::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        header(&mut bytes, SYSTEM + TARGETS, pointers, targets as u32);
        let mut next_bits = bits;
        for ordinal in 0..targets {
            let target = records + ordinal * 0x80;
            let object = objects + ordinal * 0x40;
            put(
                &mut bytes,
                pointers + ordinal * 8,
                &(target as u64).to_le_bytes(),
            );
            put(&mut bytes, target + VALID, &[1]);
            if ordinal % 2 == 0 {
                put(
                    &mut bytes,
                    target + SCOPE + craid_target::PROVINCE_ARM,
                    &(object as u64).to_le_bytes(),
                );
                put(
                    &mut bytes,
                    object,
                    &(cprovince::VTABLE_RVA.0 as u64).to_le_bytes(),
                );
                put(
                    &mut bytes,
                    object + cprovince::ID,
                    &(ordinal as u32).to_le_bytes(),
                );
            } else {
                let status = object + 0x20;
                put(
                    &mut bytes,
                    target + SCOPE + craid_target::BUILDING,
                    &(object as u64).to_le_bytes(),
                );
                put(
                    &mut bytes,
                    object,
                    &(cbuilding::VTABLE_RVA.0 as u64).to_le_bytes(),
                );
                put(
                    &mut bytes,
                    object + cbuilding::TYPE_ID,
                    &0x2fb6_u32.to_le_bytes(),
                );
                put(
                    &mut bytes,
                    object + cbuilding::BUILDING_STATUS,
                    &(status as u64).to_le_bytes(),
                );
                put(
                    &mut bytes,
                    status,
                    &(cbuilding_status::VTABLE_RVA.0 as u64).to_le_bytes(),
                );
                put(
                    &mut bytes,
                    status + cbuilding_status::FLAGS,
                    &1_u32.to_le_bytes(),
                );
                put(
                    &mut bytes,
                    status + cbuilding_status::ID,
                    &(ordinal as u32).to_le_bytes(),
                );
            }
            if ordinal % 10 == 0 {
                header(&mut bytes, target + DETECTED, next_bits, width as u32);
                next_bits += width;
            }
        }
        bytes
    }

    #[test]
    #[ignore]
    fn bench_target_statuses_by_country_count() {
        for (countries, targets) in [(60, 2_500), (1_500, 10_000), (5_000, 25_000)] {
            let bytes = scaled(targets, countries);
            let memory = engine_native_memory::memory::CountingMemory::new(&bytes);
            let mut rows = vec![Row::default(); targets];
            let mut detected = vec![0; targets * countries.div_ceil(8)];
            memory.bench(
                &format!("RaidTargetStatuses {countries} countries {targets} targets"),
                20,
                |memory| {
                    let walk = walk(memory, 0, STATE, &mut rows, &mut detected);
                    assert_eq!(walk.outcome, Outcome::Rows { len: targets });
                    format!("{targets} rows, {} detection bytes", walk.detected_len)
                },
            );
        }
    }
}
