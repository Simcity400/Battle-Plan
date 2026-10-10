use super::{AirHistoryCopyContext, AirHistoryCopyReport, AirHistoryJob, Capacities, Captured};
use engine::{Memory, memory::SliceMemory, walk::Outcome};

pub(crate) const BASE: usize = 0;
pub(crate) const GAME_STATE: usize = 0x10000;
const CTAB: usize = 0x11000;
const LTAB: usize = 0x12000;
const C0: usize = 0x20000;
const C1: usize = 0x21000;
pub(crate) const MANAGER: usize = 0x30000;
const AIR_TAB: usize = 0x30800;
const AIR0: usize = 0x31000;
const RVEC: usize = 0x31800;
pub(crate) const R0: usize = 0x32000;
const FVEC: usize = 0x32800;
const SVEC2: usize = 0x32A00;
// Records live inline in their vectors (stride 0x98): two in FVEC, one
// in SVEC2. Damage entries live inline in theirs (stride 0x18).
pub(crate) const REC0: usize = 0x32800;
const REC1: usize = 0x32898;
const REC2: usize = 0x32A00;
const DV0: usize = 0x34000;
const DV1: usize = 0x34200;
pub(crate) const E0: usize = 0x34000;

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

pub(crate) fn fixture() -> Vec<u8> {
    use engine::{
        cair_region_combat_data, ccountry, cgame_state, cstrategic_air, cstrategic_air_manager,
        sair_wing_combat_data, sdamage_entry,
    };
    let mut bytes = vec![0; 0x40000];
    header(&mut bytes, GAME_STATE + cgame_state::COUNTRIES, CTAB, 3);
    p64(&mut bytes, CTAB, 0);
    p64(&mut bytes, CTAB + 8, C0 as u64);
    p64(&mut bytes, CTAB + 16, C1 as u64);
    header(&mut bytes, GAME_STATE + cgame_state::COUNTRY_LINKS, LTAB, 4);
    p32(&mut bytes, LTAB, 0);
    p32(&mut bytes, LTAB + 4, 1);
    p32(&mut bytes, LTAB + 8, 2);
    p32(&mut bytes, LTAB + 12, 0);
    for (country, id) in [(C0, 1u32), (C1, 2u32)] {
        p64(&mut bytes, country, ccountry::VTABLE_RVA as u64);
        p32(&mut bytes, country + ccountry::ID, id);
    }
    p64(
        &mut bytes,
        GAME_STATE + cgame_state::STRATEGIC_AIR_MANAGER,
        MANAGER as u64,
    );
    p64(
        &mut bytes,
        MANAGER,
        cstrategic_air_manager::VTABLE_RVA as u64,
    );
    header(
        &mut bytes,
        MANAGER + cstrategic_air_manager::COUNTRY_AIR,
        AIR_TAB,
        2,
    );
    p64(&mut bytes, AIR_TAB, 0);
    p64(&mut bytes, AIR_TAB + 8, AIR0 as u64);
    p64(&mut bytes, AIR0, cstrategic_air::VTABLE_RVA as u64);
    p32(&mut bytes, AIR0 + cstrategic_air::COUNTRY, 1);
    header(&mut bytes, AIR0 + cstrategic_air::REGION_COMBAT, RVEC, 2);
    p64(&mut bytes, RVEC, R0 as u64);
    p64(&mut bytes, RVEC + 8, 0);
    p64(&mut bytes, R0, cair_region_combat_data::VTABLE_RVA as u64);
    p32(&mut bytes, R0 + cair_region_combat_data::OWNER, 7);
    header(
        &mut bytes,
        R0 + cair_region_combat_data::FIRST_RECORDS,
        FVEC,
        2,
    );
    p64(&mut bytes, FVEC, REC0 as u64);
    p64(&mut bytes, FVEC + 8, REC1 as u64);
    header(
        &mut bytes,
        R0 + cair_region_combat_data::SECOND_RECORDS,
        SVEC2,
        1,
    );
    p64(&mut bytes, SVEC2, REC2 as u64);
    // Three records inline: REC0/REC1 in the first vector, REC2 in the
    // second. REC0 carries two first-channel damage entries, REC1 one
    // second-channel entry, REC2 none.
    record(
        &mut bytes,
        REC0,
        0x1111,
        100,
        0xF0,
        1,
        1,
        11,
        0,
        Some((DV0, 2)),
        None,
    );
    record(
        &mut bytes,
        REC1,
        0x2222,
        -50,
        0x0F,
        0,
        0,
        12,
        1,
        None,
        Some((DV1, 1)),
    );
    record(&mut bytes, REC2, 0x3333, 0, 0, 1, 0, 13, 0, None, None);
    damage(&mut bytes, E0, 5, 1000);
    damage(&mut bytes, E0 + sdamage_entry::STRIDE, 6, -2000);
    damage(&mut bytes, DV1, 7, 3000);
    let _ = sdamage_entry::STRIDE;
    let _ = sair_wing_combat_data::STRIDE;
    bytes
}

#[allow(clippy::too_many_arguments)]
fn record(
    bytes: &mut [u8],
    at: usize,
    wing: u32,
    planes: i32,
    missions: u32,
    selection: u8,
    classification: u8,
    attributed: u32,
    copied: u8,
    first: Option<(usize, u32)>,
    second: Option<(usize, u32)>,
) {
    use engine::{sair_wing_combat_data, sdamage_entry};
    p64(bytes, at, sair_wing_combat_data::VTABLE_RVA as u64);
    p32(bytes, at + sair_wing_combat_data::WING_ID, wing);
    p32(bytes, at + sair_wing_combat_data::DATE, 1000);
    p32(
        bytes,
        at + sair_wing_combat_data::PLANE_COUNT,
        planes as u32,
    );
    p32(bytes, at + sair_wing_combat_data::MISSION_MASK, missions);
    bytes[at + sair_wing_combat_data::SELECTION_FLAG] = selection;
    bytes[at + sair_wing_combat_data::CLASSIFICATION_FLAG] = classification;
    p32(
        bytes,
        at + sair_wing_combat_data::ATTRIBUTED_COUNTRY,
        attributed,
    );
    bytes[at + sair_wing_combat_data::COPIED_FLAG] = copied;
    match first {
        Some((data, count)) => header(bytes, at + sair_wing_combat_data::FIRST_DAMAGE, data, count),
        None => header(bytes, at + sair_wing_combat_data::FIRST_DAMAGE, 0, 0),
    }
    match second {
        Some((data, count)) => header(
            bytes,
            at + sair_wing_combat_data::SECOND_DAMAGE,
            data,
            count,
        ),
        None => header(bytes, at + sair_wing_combat_data::SECOND_DAMAGE, 0, 0),
    }
    let _ = sdamage_entry::STRIDE;
}

fn damage(bytes: &mut [u8], at: usize, cause: u32, amount: i64) {
    use engine::sdamage_entry;
    p64(bytes, at, sdamage_entry::VTABLE_RVA as u64);
    p32(bytes, at + sdamage_entry::CAUSE_TYPE, cause);
    bytes[at + sdamage_entry::AMOUNT..at + sdamage_entry::AMOUNT + 8]
        .copy_from_slice(&amount.to_le_bytes());
}

pub(crate) fn memory(bytes: &[u8]) -> SliceMemory<'_> {
    SliceMemory { base: 0, bytes }
}

pub(crate) fn mod_fixture(count: usize) -> Vec<u8> {
    let mut bytes = fixture();
    let record = bytes[REC0..REC0 + engine::sair_wing_combat_data::STRIDE].to_vec();
    let data = bytes.len();
    bytes.resize(data + count * record.len(), 0);
    header(
        &mut bytes,
        R0 + engine::cair_region_combat_data::FIRST_RECORDS,
        data,
        count as u32,
    );
    for row in bytes[data..].chunks_exact_mut(record.len()) {
        row.copy_from_slice(&record);
    }
    bytes
}

pub(crate) struct Split {
    pub history_outcome: Outcome,
    pub history: Vec<engine::sair_wing_combat_data::Row>,
    pub damage_outcome: Outcome,
    pub damage: Vec<engine::sdamage_entry::Row>,
}

fn collect_typed(memory: &impl Memory, history_cap: usize, damage_cap: usize) -> Split {
    use engine::cair_region_combat_data;
    let capacities = Capacities {
        rows: [
            8192,
            2048,
            4096,
            history_cap,
            damage_cap,
            1024,
            2048,
            4096,
            1024,
            64,
            0,
        ],
        ace_names: 65_536,
        wings: 8192,
        sorties: Default::default(),
        scores: Default::default(),
        transport: [0; 5],
    };
    let mut captured = Captured::with_capacities(&capacities);
    let walked = cair_region_combat_data::walk_world(
        memory,
        BASE,
        GAME_STATE,
        &mut captured.history.rows,
        &mut captured.damage.rows,
    );
    captured.history.outcome = super::bounded(walked.records, captured.history.rows.len());
    captured.damage.outcome = super::bounded(walked.damage, captured.damage.rows.len());
    Split {
        history_outcome: captured.history.outcome,
        history: captured.history.rows.clone(),
        damage_outcome: captured.damage.outcome,
        damage: captured.damage.rows.clone(),
    }
}

fn collect_split(
    memory: &impl Memory,
    history_cap: usize,
    damage_cap: usize,
) -> (Split, AirHistoryCopyReport, AirHistoryJob) {
    let ctx = AirHistoryCopyContext {
        hour: 0,
        force_full: true,
        base: BASE,
        game_state: GAME_STATE,

        history_limit: history_cap,
        damage_limit: damage_cap,
    };
    let capacities = Capacities {
        rows: [
            8192,
            2048,
            4096,
            history_cap,
            damage_cap,
            1024,
            2048,
            4096,
            1024,
            64,
            0,
        ],
        ace_names: 65_536,
        wings: 8192,
        sorties: Default::default(),
        scores: Default::default(),
        transport: [0; 5],
    };
    let mut captured = Captured::with_capacities(&capacities);
    let mut first = AirHistoryJob::fixture();
    let report = super::copy_air_history_refresh(memory, ctx, &mut first);
    assert!(report.raw_ok);
    let mut ticks = [0u64; 2];
    captured.decode_air_history_payload(&first, &mut ticks);
    (
        Split {
            history_outcome: captured.history.outcome,
            history: captured.history.rows.clone(),
            damage_outcome: captured.damage.outcome,
            damage: captured.damage.rows.clone(),
        },
        report,
        first,
    )
}

pub(crate) fn direct(memory: &impl Memory) -> Split {
    collect_typed(memory, 8192, 16_384)
}

pub(crate) fn split(memory: &impl Memory) -> (Split, AirHistoryCopyReport, AirHistoryJob) {
    collect_split(memory, 8192, 16_384)
}

pub(crate) fn direct_with(memory: &impl Memory, history: usize, damage: usize) -> Split {
    collect_typed(memory, history, damage)
}

pub(crate) fn split_with(
    memory: &impl Memory,
    history: usize,
    damage: usize,
) -> (Split, AirHistoryCopyReport, AirHistoryJob) {
    collect_split(memory, history, damage)
}

pub(crate) fn assert_matches_direct(split: &Split, direct: &Split) {
    assert_eq!(
        split.history_outcome, direct.history_outcome,
        "history outcome"
    );
    assert_eq!(
        split.history[..split.history_outcome.len()],
        direct.history[..direct.history_outcome.len()],
        "history rows"
    );
    assert_eq!(
        split.damage_outcome, direct.damage_outcome,
        "damage outcome"
    );
    assert_eq!(
        split.damage[..split.damage_outcome.len()],
        direct.damage[..direct.damage_outcome.len()],
        "damage rows"
    );
}
