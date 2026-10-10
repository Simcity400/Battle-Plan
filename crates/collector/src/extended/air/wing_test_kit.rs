use super::{AirWingCopyContext, AirWingCopyReport, AirWingJob, Capacities, Captured};
use engine::{Memory, memory::SliceMemory, walk::Outcome};

pub(crate) const BASE: usize = 0;
pub(crate) const GAME_STATE: usize = 0x10000;
const CTAB: usize = 0x11000;
const LTAB: usize = 0x12000;
const C0: usize = 0x20000;
const MANAGER: usize = 0x30000;
const OWNER_AIR: usize = 0x31000;
pub(crate) const POOL: usize = 0x32000;
pub(crate) const WING: usize = 0x33000;
const WING2: usize = 0x34000;
pub(crate) const ENTRIES: usize = 0x35000;
const VARIANT: usize = 0x36000;
const VARIANT2: usize = 0x37000;

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
        cair_wing, ccountry, cequipment_variant, cequipment_variant_pool, equipment_entry,
    };
    let mut bytes = vec![0; 0x40000];
    header(
        &mut bytes,
        GAME_STATE + engine::cgame_state::COUNTRIES,
        CTAB,
        3,
    );
    p64(&mut bytes, CTAB, 0);
    p64(&mut bytes, CTAB + 8, C0 as u64);
    p64(&mut bytes, CTAB + 16, C0 as u64);
    header(
        &mut bytes,
        GAME_STATE + engine::cgame_state::COUNTRY_LINKS,
        LTAB,
        3,
    );
    p32(&mut bytes, LTAB, 0);
    p32(&mut bytes, LTAB + 4, 1);
    p32(&mut bytes, LTAB + 8, 1);
    p64(&mut bytes, C0, ccountry::VTABLE_RVA as u64);
    p32(&mut bytes, C0 + ccountry::ID, 1);
    p64(
        &mut bytes,
        GAME_STATE + engine::cgame_state::STRATEGIC_AIR_MANAGER,
        MANAGER as u64,
    );
    p64(
        &mut bytes,
        MANAGER,
        engine::cstrategic_air_manager::VTABLE_RVA as u64,
    );
    header(
        &mut bytes,
        MANAGER + engine::cstrategic_air_manager::COUNTRY_AIR,
        0x38000,
        2,
    );
    p64(&mut bytes, 0x38000, 0);
    p64(&mut bytes, 0x38000 + 8, OWNER_AIR as u64);
    p64(
        &mut bytes,
        OWNER_AIR,
        engine::cstrategic_air::VTABLE_RVA as u64,
    );
    p32(&mut bytes, OWNER_AIR + engine::cstrategic_air::COUNTRY, 1);
    header(
        &mut bytes,
        OWNER_AIR + engine::cstrategic_air::POOLS,
        0x38100,
        1,
    );
    p64(&mut bytes, 0x38100, POOL as u64);
    p64(&mut bytes, POOL, engine::cair_wing_pool::VTABLE_RVA as u64);
    p64(
        &mut bytes,
        POOL + engine::cair_wing_pool::OWNER,
        OWNER_AIR as u64,
    );
    p64(
        &mut bytes,
        POOL + engine::cair_wing_pool::AIR_BASE_REFERENCE,
        0x1234u64,
    );
    header(&mut bytes, POOL + engine::cair_wing_pool::WINGS, 0x38200, 2);
    p64(&mut bytes, 0x38200, WING as u64);
    p64(&mut bytes, 0x38200 + 8, WING2 as u64);
    for (wing, id, accidents, exp) in [
        (WING, 0x4500_0001u64, 3i32, 1000i64),
        (WING2, 0x4500_0002u64, -5i32, i64::MAX),
    ] {
        p64(&mut bytes, wing, cair_wing::VTABLE_RVA as u64);
        p64(
            &mut bytes,
            wing + cair_wing::SECONDARY,
            cair_wing::SECONDARY_VTABLE_RVA as u64,
        );
        p64(&mut bytes, wing + cair_wing::SECONDARY + 8, id);
        p64(&mut bytes, wing + cair_wing::POOL, POOL as u64);
        p64(&mut bytes, wing + cair_wing::BASE_OVERRIDE, 0);
        p32(&mut bytes, wing + cair_wing::PLANE_COUNT, 10);
        p32(
            &mut bytes,
            wing + cair_wing::MISSION + engine::air_mission::ENABLED_MASK,
            0xff,
        );
        p64(
            &mut bytes,
            wing + cair_wing::MISSION + engine::air_mission::REGION,
            0,
        );
        p32(&mut bytes, wing + cair_wing::COUNTRY_OVERRIDE, 0);
        p32(
            &mut bytes,
            wing + cair_wing::AIR_ACCIDENTS,
            accidents as u32,
        );
        p64(&mut bytes, wing + cair_wing::EXPERIENCE, exp as u64);
        // Equipment pool: two entries on WING, one on WING2.
        let (items, entries) = if wing == WING {
            (0x39000usize, vec![(VARIANT, 100i64), (VARIANT2, -50i64)])
        } else {
            (0x39100usize, vec![(VARIANT, 7i64)])
        };
        let pool = wing + cair_wing::EQUIPMENT_POOL;
        p64(&mut bytes, pool, cequipment_variant_pool::VTABLE_RVA as u64);
        header(
            &mut bytes,
            pool + cequipment_variant_pool::ITEMS,
            items,
            entries.len() as u32,
        );
        for (index, (variant, amount)) in entries.into_iter().enumerate() {
            p64(
                &mut bytes,
                items + index * equipment_entry::STRIDE,
                variant as u64,
            );
            p64(
                &mut bytes,
                items + index * equipment_entry::STRIDE + 8,
                amount as u64,
            );
        }
    }
    for (variant, id) in [(VARIANT, 11u32), (VARIANT2, 12u32)] {
        p64(&mut bytes, variant, cequipment_variant::VTABLE_RVA as u64);
        p64(
            &mut bytes,
            variant + 8,
            ((id as u64) << 32) | cequipment_variant::REFERENCE_TYPE as u64,
        );
    }
    // Alias ENTRIES to the first wing's item vector for the fault test.
    let _ = ENTRIES;
    bytes
}

pub(crate) fn memory(bytes: &[u8]) -> SliceMemory<'_> {
    SliceMemory { base: 0, bytes }
}

pub(crate) struct Split {
    pub equipment_outcome: Outcome,
    pub equipment: Vec<replay::rows::WingEquipmentRow>,
    pub cumulative_outcome: Outcome,
    pub cumulative: Vec<engine::cair_wing::CumulativeRow>,
}

fn collect_typed(memory: &impl Memory, capacity: &Capacities) -> Split {
    use engine::cair_wing;
    let mut captured = Captured::with_capacities(capacity);
    // Typed reference: wing walk with visitor into pooled buffers,
    // exactly like the game thread's Sources::air sequence.
    captured.begin_wings();
    // Re-walk collecting (pointer,row) for the visitor (typed, no raw).
    let mut collected: Vec<(usize, usize, cair_wing::Row)> = Vec::new();
    let mut wing_rows = vec![cair_wing::Row::default(); 8192];
    let outcome = cair_wing::walk_with(
        memory,
        BASE,
        GAME_STATE,
        &mut wing_rows,
        |pointer, pool, row| {
            collected.push((pointer, pool, *row));
        },
    );
    for (pointer, pool, row) in &collected {
        let source = super::WingSource {
            pointer: *pointer,
            pool: *pool,
            row,
        };
        captured.visit_wing(memory, BASE, source);
    }
    captured.finish_wings(outcome);
    let _ = outcome;
    Split {
        equipment_outcome: captured.equipment.outcome,
        equipment: captured.equipment.rows.clone(),
        cumulative_outcome: captured.cumulative.outcome,
        cumulative: captured.cumulative.rows.clone(),
    }
}

fn collect(memory: &impl Memory) -> Split {
    collect_typed(memory, &Capacities::fixture())
}

pub(crate) fn direct(memory: &impl Memory) -> Split {
    collect(memory)
}

pub(crate) fn split(memory: &impl Memory) -> (Split, AirWingCopyReport, AirWingJob) {
    let ctx = AirWingCopyContext {
        hour: 0,
        force_full: true,
        base: BASE,
        game_state: GAME_STATE,

        limits: [1024, 8192, 4096],
        naval_limit: 4096,
        game_score: false,
    };
    let mut first = AirWingJob::fixture();
    let report = super::copy_air_refresh(memory, ctx, &mut first);
    assert!(report.raw_ok);
    let mut captured = Captured::with_capacities(&Capacities::fixture());
    let mut ticks = [0u64; 2];
    super::decode_air_payload(&mut captured, &first, &mut ticks);
    (
        Split {
            equipment_outcome: captured.equipment.outcome,
            equipment: captured.equipment.rows.clone(),
            cumulative_outcome: captured.cumulative.outcome,
            cumulative: captured.cumulative.rows.clone(),
        },
        report,
        first,
    )
}

pub(crate) fn direct_with(memory: &impl Memory, capacity: &Capacities) -> Split {
    collect_typed(memory, capacity)
}

pub(crate) fn split_with(
    memory: &impl Memory,
    capacity: &Capacities,
) -> (Split, AirWingCopyReport, AirWingJob) {
    let ctx = AirWingCopyContext {
        hour: 0,
        force_full: true,
        base: BASE,
        game_state: GAME_STATE,

        limits: [1024, capacity.rows[0], capacity.rows[2]],
        naval_limit: 4096,
        game_score: false,
    };
    let mut first = AirWingJob::fixture();
    let report = super::copy_air_refresh(memory, ctx, &mut first);
    assert!(report.raw_ok);
    let mut captured = Captured::with_capacities(capacity);
    let mut ticks = [0u64; 2];
    super::decode_air_payload(&mut captured, &first, &mut ticks);
    (
        Split {
            equipment_outcome: captured.equipment.outcome,
            equipment: captured.equipment.rows.clone(),
            cumulative_outcome: captured.cumulative.outcome,
            cumulative: captured.cumulative.rows.clone(),
        },
        report,
        first,
    )
}

pub(crate) fn direct_truncated(memory: &impl Memory) -> Split {
    let capacity = Capacities {
        rows: [1, 2048, 1, 8192, 16384, 1024, 2048, 4096, 1024, 64, 0],
        ace_names: 65536,
        wings: 8192,
        sorties: Default::default(),
        scores: Default::default(),
        transport: [0; 5],
    };
    collect_typed(memory, &capacity)
}

pub(crate) fn split_truncated(memory: &impl Memory) -> (Split, AirWingCopyReport, AirWingJob) {
    let ctx = AirWingCopyContext {
        hour: 0,
        force_full: true,
        base: BASE,
        game_state: GAME_STATE,

        limits: [1024, 1, 1],
        naval_limit: 4096,
        game_score: false,
    };
    let mut first = AirWingJob::fixture();
    let report = super::copy_air_refresh(memory, ctx, &mut first);
    assert!(report.raw_ok);
    let capacity = Capacities {
        rows: [1, 2048, 1, 8192, 16384, 1024, 2048, 4096, 1024, 64, 0],
        ace_names: 65536,
        wings: 8192,
        sorties: Default::default(),
        scores: Default::default(),
        transport: [0; 5],
    };
    let mut captured = Captured::with_capacities(&capacity);
    let mut ticks = [0u64; 2];
    super::decode_air_payload(&mut captured, &first, &mut ticks);
    (
        Split {
            equipment_outcome: captured.equipment.outcome,
            equipment: captured.equipment.rows.clone(),
            cumulative_outcome: captured.cumulative.outcome,
            cumulative: captured.cumulative.rows.clone(),
        },
        report,
        first,
    )
}

pub(crate) fn assert_matches_direct(split: &Split, direct: &Split) {
    assert_eq!(
        split.equipment_outcome, direct.equipment_outcome,
        "equipment outcome"
    );
    assert_eq!(
        split.equipment[..split.equipment_outcome.len()],
        direct.equipment[..direct.equipment_outcome.len()],
        "equipment rows"
    );
    assert_eq!(
        split.cumulative_outcome, direct.cumulative_outcome,
        "cumulative outcome"
    );
    assert_eq!(
        split.cumulative[..split.cumulative_outcome.len()],
        direct.cumulative[..direct.cumulative_outcome.len()],
        "cumulative rows"
    );
}

/// Decode-worker assertion against pooled buffers (same module, so
/// private rows are visible). Compares outcomes and rows with the typed
/// reference without cloning through public views.
pub(crate) fn assert_captured_matches_direct(captured: &super::Captured, direct: &Split) {
    assert_eq!(
        captured.equipment.outcome, direct.equipment_outcome,
        "equipment outcome"
    );
    assert_eq!(
        &captured.equipment.rows[..captured.equipment.outcome.len()],
        &direct.equipment[..direct.equipment_outcome.len()],
        "equipment rows"
    );
    assert_eq!(
        captured.cumulative.outcome, direct.cumulative_outcome,
        "cumulative outcome"
    );
    assert_eq!(
        &captured.cumulative.rows[..captured.cumulative.outcome.len()],
        &direct.cumulative[..direct.cumulative_outcome.len()],
        "cumulative rows"
    );
}
