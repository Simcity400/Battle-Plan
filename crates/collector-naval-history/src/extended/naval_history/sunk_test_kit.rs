use super::{Capacities, Captured, SunkCopyContext, SunkCopyReport, SunkJob};
use engine_native_memory::{Memory, memory::SliceMemory, walk::Outcome};

pub const BASE: usize = 0;
pub const GAME_STATE: usize = 0x100;
pub const HOUR: i32 = 100;
const VECTOR: usize = 0x800;
pub const FIRST: usize = 0x900;
const SECOND: usize = 0xb00;
pub const CLASS: usize = 0xd00;
const NULL_CLASS: usize = 0xe00;
const MAP_PROVINCE: usize = 0xf00;
const HEAP_NAMES: usize = 0x1000;

fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
    bytes[at..at + value.len()].copy_from_slice(value);
}

fn short(bytes: &mut [u8], at: usize, value: &[u8]) {
    put(bytes, at, value);
    put(
        bytes,
        at + msvc_string::LENGTH,
        &(value.len() as u64).to_le_bytes(),
    );
    put(bytes, at + msvc_string::CAPACITY, &15_u64.to_le_bytes());
}

use engine_native_memory::msvc_string;

pub fn fixture() -> Vec<u8> {
    use engine_native_memory::cgame_date;
    use engine_native::{cgame_state, cprovince};
    use engine_native_parts::csub_unit_definition;
    use engine_naval_history::csunk_ship_info;
    let mut bytes = vec![0_u8; 0x2000];
    put(
        &mut bytes,
        GAME_STATE + cgame_state::SUNK_SHIP_HISTORY,
        &(VECTOR as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        GAME_STATE + cgame_state::SUNK_SHIP_HISTORY_CAPACITY,
        &2_u32.to_le_bytes(),
    );
    put(
        &mut bytes,
        GAME_STATE + cgame_state::SUNK_SHIP_HISTORY_COUNT,
        &2_u32.to_le_bytes(),
    );
    put(&mut bytes, VECTOR, &(FIRST as u64).to_le_bytes());
    put(&mut bytes, VECTOR + 8, &(SECOND as u64).to_le_bytes());
    for (record, hour, class, attribution, province, victim, attacker) in [
        (FIRST, 90u32, CLASS, NULL_CLASS, MAP_PROVINCE, 3u32, 4u32),
        (SECOND, 100u32, CLASS, 0usize, 0usize, 5u32, 6u32),
    ] {
        put(
            &mut bytes,
            record,
            &(csunk_ship_info::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            record + csunk_ship_info::SINK_DATE,
            &(cgame_date::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            record + csunk_ship_info::SINK_DATE_PERSISTENT,
            &(cgame_date::PERSISTENT_VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            record + csunk_ship_info::SINK_HOUR,
            &hour.to_le_bytes(),
        );
        put(
            &mut bytes,
            record + csunk_ship_info::SHIP_CLASS,
            &(class as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            record + csunk_ship_info::ATTRIBUTION_CLASS,
            &(attribution as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            record + csunk_ship_info::VICTIM_COUNTRY,
            &victim.to_le_bytes(),
        );
        put(
            &mut bytes,
            record + csunk_ship_info::ATTACKER_COUNTRY,
            &attacker.to_le_bytes(),
        );
        put(
            &mut bytes,
            record + csunk_ship_info::MODEL_LEVEL,
            &2_i32.to_le_bytes(),
        );
        put(
            &mut bytes,
            record + csunk_ship_info::VICTIM_EQUIPMENT_VARIANT,
            &0x1234_5678_9abc_def0u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            record + csunk_ship_info::DAMAGE_SOURCE_REFERENCE,
            &0x0fed_cba9_8765_4321u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            record + csunk_ship_info::PROVINCE,
            &(province as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            record + csunk_ship_info::BATTLE,
            &0x5555_6666_7777_8888u64.to_le_bytes(),
        );
        bytes[record + csunk_ship_info::CAUSE_RECORD] = 1;
        bytes[record + csunk_ship_info::ASSISTED_KILL] = 0;
    }
    short(&mut bytes, FIRST + csunk_ship_info::SHIP_NAME, b"Ship");
    short(
        &mut bytes,
        FIRST + csunk_ship_info::ATTRIBUTION_SOURCE_NAME,
        b"Src",
    );
    short(
        &mut bytes,
        SECOND + csunk_ship_info::SHIP_NAME,
        b"SecondShip",
    );
    put(
        &mut bytes,
        SECOND + csunk_ship_info::ATTRIBUTION_SOURCE_NAME + msvc_string::DATA,
        &(HEAP_NAMES as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        SECOND + csunk_ship_info::ATTRIBUTION_SOURCE_NAME + msvc_string::LENGTH,
        &4_u64.to_le_bytes(),
    );
    put(
        &mut bytes,
        SECOND + csunk_ship_info::ATTRIBUTION_SOURCE_NAME + msvc_string::CAPACITY,
        &16_u64.to_le_bytes(),
    );
    put(&mut bytes, HEAP_NAMES, b"Heap");
    bytes[HEAP_NAMES + 4] = 0;
    for (class, token, vtable) in [
        (CLASS, 11u32, csub_unit_definition::VTABLE_RVA),
        (NULL_CLASS, 0u32, csub_unit_definition::NULL_VTABLE_RVA),
    ] {
        put(&mut bytes, class, &(vtable.0 as u64).to_le_bytes());
        put(
            &mut bytes,
            class + csub_unit_definition::TOKEN,
            &token.to_le_bytes(),
        );
        short(&mut bytes, class + csub_unit_definition::NAME, b"Cls");
    }
    put(
        &mut bytes,
        MAP_PROVINCE,
        &(cprovince::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    put(
        &mut bytes,
        MAP_PROVINCE + cprovince::ID,
        &77_u32.to_le_bytes(),
    );
    bytes
}

pub fn memory(bytes: &[u8]) -> SliceMemory<'_> {
    SliceMemory { base: 0, bytes }
}

pub fn mod_fixture(count: usize) -> Vec<u8> {
    let mut bytes = fixture();
    let data = bytes.len();
    bytes.resize(data + count * 8, 0);
    put(
        &mut bytes,
        GAME_STATE + engine_native::cgame_state::SUNK_SHIP_HISTORY,
        &(data as u64).to_le_bytes(),
    );
    for offset in [
        engine_native::cgame_state::SUNK_SHIP_HISTORY_CAPACITY,
        engine_native::cgame_state::SUNK_SHIP_HISTORY_COUNT,
    ] {
        put(
            &mut bytes,
            GAME_STATE + offset,
            &(count as u32).to_le_bytes(),
        );
    }
    for index in 0..count {
        put(&mut bytes, data + index * 8, &(FIRST as u64).to_le_bytes());
    }
    bytes
}

pub struct Split {
    pub outcome: Outcome,
    pub rows: Vec<engine_naval_history::csunk_ship_info::Record>,
    pub bytes_len: usize,
    pub required: usize,
}

fn collect_typed(memory: &impl Memory, rows_cap: usize, names_cap: usize) -> Split {
    use engine_naval_history::csunk_ship_info;
    let mut capacities = Capacities::fixture();
    capacities.rows[13] = rows_cap;
    capacities.sunk_names = names_cap;
    let mut captured = Captured::with_capacities(&capacities);
    let walked = csunk_ship_info::walk(
        memory,
        BASE,
        GAME_STATE,
        HOUR,
        &mut captured.sunk.rows,
        &mut captured.sunk_names.first,
    );
    captured.sunk.outcome = super::bounded(walked.outcome, captured.sunk.rows.len());
    captured.sunk_names.len = walked.bytes_len;
    captured.sunk_names.required = walked.required_bytes;
    if !super::valid_sunk(
        captured.sunk.outcome,
        &captured.sunk.rows,
        &captured.sunk_names.first,
        walked.bytes_len,
        walked.required_bytes,
    ) {
        captured.sunk.outcome = super::invalid_names();
    }
    Split {
        outcome: captured.sunk.outcome,
        rows: captured.sunk.rows.clone(),
        bytes_len: captured.sunk_names.len,
        required: captured.sunk_names.required,
    }
}

fn collect_split(
    memory: &impl Memory,
    rows_cap: usize,
    names_cap: usize,
) -> (Split, SunkCopyReport, SunkJob) {
    let ctx = SunkCopyContext {
        force_full: true,
        base: BASE,
        game_state: GAME_STATE,
        hour: HOUR,
    };
    let mut capacities = Capacities::fixture();
    capacities.rows[13] = rows_cap;
    capacities.sunk_names = names_cap;
    let mut captured = Captured::with_capacities(&capacities);
    let mut first = SunkJob::fixture();
    let report = super::copy_sunk_refresh(memory, ctx, &mut first);
    assert!(report.raw_ok);
    let mut ticks = [0u64; 1];
    captured.decode_sunk_payload(BASE, &first, &mut ticks);
    (
        Split {
            outcome: captured.sunk.outcome,
            rows: captured.sunk.rows.clone(),
            bytes_len: captured.sunk_names.len,
            required: captured.sunk_names.required,
        },
        report,
        first,
    )
}

pub fn direct(memory: &impl Memory) -> Split {
    collect_typed(memory, 2048, 256 * 1024)
}

pub fn split(memory: &impl Memory) -> (Split, SunkCopyReport, SunkJob) {
    collect_split(memory, 2048, 256 * 1024)
}

pub fn direct_with(memory: &impl Memory, rows: usize, names: usize) -> Split {
    collect_typed(memory, rows, names)
}

pub fn split_with(
    memory: &impl Memory,
    rows: usize,
    names: usize,
) -> (Split, SunkCopyReport, SunkJob) {
    collect_split(memory, rows, names)
}

pub fn assert_matches_direct(split: &Split, direct: &Split) {
    assert_eq!(split.outcome, direct.outcome, "sunk outcome");
    assert_eq!(split.rows, direct.rows, "sunk rows");
    assert_eq!(split.bytes_len, direct.bytes_len, "sunk bytes len");
    assert_eq!(split.required, direct.required, "sunk required");
}
