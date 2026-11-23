use engine_native_memory::{Memory, memory::SliceMemory, walk::Outcome};
use engine_native::{
    ccountry,
    cgame_state,
    crelation::{self, status as crelation_status},
    cwar_relation,
};

pub const BASE: usize = 0;
pub const GAME_STATE: usize = 0x10000;
pub const CTAB: usize = 0x11000;
pub const LTAB: usize = 0x12000;
pub const C0: usize = 0x20000;
const C: [usize; 3] = [0x20000, 0x21000, 0x22000];
pub const D0: usize = 0x30000;
const D: [usize; 3] = [0x30000, 0x31000, 0x32000];
pub const M0: usize = 0x33000;
const M: [usize; 3] = [0x33000, 0x34000, 0x35000];
pub const O0: usize = 0x36000;
const O: [usize; 3] = [0x36000, 0x37000, 0x38000];
pub const WAR1: usize = 0x70000;
pub const WAR2: usize = 0x71000;
pub const OTHER: usize = 0x72000;
pub const RETIRED: usize = 0x73000;

pub fn status(owner: usize, target: usize) -> usize {
    0x40000 + owner * 0x10000 + target * 0xa00
}

fn p32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn p64(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

fn header(bytes: &mut [u8], at: usize, data: usize, count: u32) {
    p64(bytes, at, data as u64);
    p32(bytes, at + 8, count);
    p32(bytes, at + 12, count);
}

fn entries64(bytes: &mut [u8], data: usize, values: &[u64]) {
    for (index, &value) in values.iter().enumerate() {
        p64(bytes, data + index * 8, value);
    }
}

fn entries32(bytes: &mut [u8], data: usize, values: &[u32]) {
    for (index, &value) in values.iter().enumerate() {
        p32(bytes, data + index * 4, value);
    }
}

fn status_init(bytes: &mut [u8], owner: usize, target: usize, raw_target: u32) {
    let at = status(owner, target);
    p64(bytes, at, crelation_status::VTABLE_RVA.0 as u64);
    p64(
        bytes,
        at + crelation_status::DIPLOMACY_STATUS,
        D[owner] as u64,
    );
    p32(bytes, at + crelation_status::TARGET_COUNTRY, raw_target);
    p64(bytes, at + crelation_status::WAR_RELATION, 0);
    header(
        &mut *bytes,
        at + crelation_status::SUBRELATIONS,
        at + 0x400,
        0,
    );
    header(
        &mut *bytes,
        at + crelation_status::SERIALIZABLE_RELATIONS,
        at + 0x420,
        0,
    );
}

pub fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0x80000];
    header(&mut bytes, GAME_STATE + cgame_state::COUNTRIES, CTAB, 5);
    entries64(
        &mut bytes,
        CTAB,
        &[0, C[0] as u64, C[1] as u64, C[2] as u64, C[0] as u64],
    );
    header(&mut bytes, GAME_STATE + cgame_state::COUNTRY_LINKS, LTAB, 5);
    entries32(&mut bytes, LTAB, &[0, 1, 2, 3, 1]);
    for owner in 0..3 {
        p64(&mut bytes, C[owner], ccountry::VTABLE_RVA.0 as u64);
        p32(&mut bytes, C[owner] + ccountry::ID, (owner + 1) as u32);
        p64(
            &mut bytes,
            C[owner] + ccountry::DIPLOMACY_STATUS,
            D[owner] as u64,
        );
        p64(&mut bytes, D[owner], engine_native_parts::cdiplomacy_status_layout::VTABLE_RVA.0 as u64);
        p64(
            &mut bytes,
            D[owner] + engine_native_parts::cdiplomacy_status_layout::COUNTRY,
            C[owner] as u64,
        );
        let slots: Vec<u64> = (0..5).map(|target| status(owner, target) as u64).collect();
        header(
            &mut bytes,
            D[owner] + engine_native_parts::cdiplomacy_status_layout::RELATIONS,
            M[owner],
            5,
        );
        entries64(&mut bytes, M[owner], &slots);
        for target in 0..5 {
            let raw = if (owner, target) == (0, 1) {
                4
            } else {
                target as u32
            };
            status_init(&mut bytes, owner, target, raw);
        }
    }
    header(
        &mut bytes,
        D[0] + engine_native_parts::cdiplomacy_status_layout::OWNED_RELATIONS,
        O[0],
        3,
    );
    entries64(&mut bytes, O[0], &[WAR1 as u64, OTHER as u64, WAR2 as u64]);
    header(
        &mut bytes,
        D[1] + engine_native_parts::cdiplomacy_status_layout::OWNED_RELATIONS,
        O[1],
        2,
    );
    entries64(&mut bytes, O[1], &[WAR1 as u64, RETIRED as u64]);
    header(
        &mut bytes,
        D[2] + engine_native_parts::cdiplomacy_status_layout::OWNED_RELATIONS,
        O[2],
        1,
    );
    entries64(&mut bytes, O[2], &[WAR2 as u64]);
    p32(&mut bytes, OTHER + crelation::TYPE_ID, 0x28da);
    p64(&mut bytes, RETIRED, cwar_relation::VTABLE_RVA.0 as u64);
    p32(
        &mut bytes,
        RETIRED + crelation::TYPE_ID,
        cwar_relation::TYPE,
    );
    bytes[RETIRED + crelation::RETIRED] = 1;
    p32(&mut bytes, RETIRED + crelation::FIRST_COUNTRY, 1);
    p32(&mut bytes, RETIRED + crelation::SECOND_COUNTRY, 2);
    for (war, second, start, end, cancel, cas, unk, instigator) in [
        (
            WAR1,
            2u32,
            i32::MIN,
            i32::MAX,
            7u8,
            [i64::MAX, i64::MIN],
            [0x1_0000_0001i64, -0x1_0000_0001i64],
            255u8,
        ),
        (WAR2, 1u32, 5, 6, 0u8, [11, 22], [33, 44], 0u8),
    ] {
        let first = if war == WAR1 { 1u32 } else { 3u32 };
        p64(&mut bytes, war, cwar_relation::VTABLE_RVA.0 as u64);
        p32(&mut bytes, war + crelation::TYPE_ID, cwar_relation::TYPE);
        p32(&mut bytes, war + crelation::FIRST_COUNTRY, first);
        p32(&mut bytes, war + crelation::SECOND_COUNTRY, second);
        p32(&mut bytes, war + crelation::START_HOUR, start as u32);
        p32(&mut bytes, war + crelation::END_HOUR, end as u32);
        bytes[war + crelation::CANCEL] = cancel;
        bytes[war + cwar_relation::FIRST_WAS_INSTIGATOR] = instigator;
        p64(
            &mut bytes,
            war + cwar_relation::FIRST_CASUALTIES,
            cas[0] as u64,
        );
        p64(
            &mut bytes,
            war + cwar_relation::SECOND_CASUALTIES,
            cas[1] as u64,
        );
        p64(
            &mut bytes,
            war + cwar_relation::FIRST_UNKNOWN_CASUALTIES,
            unk[0] as u64,
        );
        p64(
            &mut bytes,
            war + cwar_relation::SECOND_UNKNOWN_CASUALTIES,
            unk[1] as u64,
        );
    }
    for (owner, target, war) in [(0, 2, WAR1), (1, 1, WAR1), (2, 1, WAR2), (0, 3, WAR2)] {
        let at = status(owner, target);
        p64(&mut bytes, at + crelation_status::WAR_RELATION, war as u64);
        let sub = &[war as u64][..];
        header(
            &mut bytes,
            at + crelation_status::SUBRELATIONS,
            at + 0x400,
            1,
        );
        entries64(&mut bytes, at + 0x400, sub);
        let serializable = (owner, target) == (0, 2) || (owner, target) == (2, 1);
        header(
            &mut bytes,
            at + crelation_status::SERIALIZABLE_RELATIONS,
            at + 0x420,
            u32::from(serializable),
        );
        if serializable {
            entries64(&mut bytes, at + 0x420, sub);
        }
    }
    bytes
}

pub fn memory(bytes: &[u8]) -> SliceMemory<'_> {
    SliceMemory { base: 0, bytes }
}

pub struct Capture {
    pub owned_outcome: Outcome,
    pub owned: Vec<cwar_relation::Row>,
}

pub fn direct(memory: &impl Memory, len: usize) -> Capture {
    let mut owned = vec![cwar_relation::Row::default(); len];
    let owned_outcome = cwar_relation::walk(memory, BASE, GAME_STATE, &mut owned);
    owned.truncate(owned_outcome.len());
    Capture {
        owned_outcome,
        owned,
    }
}
