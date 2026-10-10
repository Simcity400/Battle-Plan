use super::*;
use crate::memory::{RecordLayout, RecordSources, SliceMemory};
use std::cell::{Cell, RefCell};
const COUNTRY: usize = 0x100;
const STATUS: usize = 0x10000;
const SLOT: usize = 0x12000;
const TECH: usize = 0x13000;
const TEMPLATE: usize = 0x14000;
const BONUS: usize = 0x15000;
const SLOTS: usize = 0x17000;
const IDS: usize = 0x18000;
const BONUSES: usize = 0x18100;

fn put<const N: usize>(bytes: &mut [u8], at: usize, value: [u8; N]) {
    bytes[at..at + N].copy_from_slice(&value);
}
fn array(bytes: &mut [u8], at: usize, data: usize, count: u32) {
    put(bytes, at, (data as u64).to_le_bytes());
    put(bytes, at + 8, count.to_le_bytes());
    put(bytes, at + 12, count.to_le_bytes());
}
fn text(bytes: &mut [u8], at: usize, token: &[u8]) {
    bytes[at..at + token.len()].copy_from_slice(token);
    put(bytes, at + 16, (token.len() as u64).to_le_bytes());
    put(bytes, at + 24, 15_u64.to_le_bytes());
}
fn fixture(active: bool, applied: bool) -> Vec<u8> {
    let mut b = vec![0; 0x20000];
    put(
        &mut b,
        COUNTRY + crate::ccountry::TECHNOLOGY_STATUS,
        (STATUS as u64).to_le_bytes(),
    );
    put(
        &mut b,
        STATUS,
        (ctechnology_status::VTABLE_RVA as u64).to_le_bytes(),
    );
    put(
        &mut b,
        STATUS + ctechnology_status::COUNTRY,
        (COUNTRY as u64).to_le_bytes(),
    );
    array(
        &mut b,
        STATUS + ctechnology_status::RESEARCH_SLOTS_DATA,
        SLOTS,
        1,
    );
    put(&mut b, SLOTS, (SLOT as u64).to_le_bytes());
    put(
        &mut b,
        SLOT,
        (cresearch_slot::VTABLE_RVA as u64).to_le_bytes(),
    );
    put(
        &mut b,
        SLOT + cresearch_slot::STATUS,
        (STATUS as u64).to_le_bytes(),
    );
    put(
        &mut b,
        SLOT + cresearch_slot::TECHNOLOGY_HANDLE,
        (if active {
            7
        } else {
            cresearch_slot::IDLE_HANDLE
        })
        .to_le_bytes(),
    );
    put(
        &mut b,
        SLOT + cresearch_slot::RESEARCH_SPEED,
        150_000_i64.to_le_bytes(),
    );
    if active {
        put(
            &mut b,
            SLOT + cresearch_slot::TECHNOLOGY,
            (TECH as u64).to_le_bytes(),
        );
        put(&mut b, TECH, (ctechnology::VTABLE_RVA as u64).to_le_bytes());
        put(
            &mut b,
            TECH + ctechnology::DEFINITION_INDEX,
            7_u32.to_le_bytes(),
        );
        put(
            &mut b,
            TECH + ctechnology::STATUS,
            (STATUS as u64).to_le_bytes(),
        );
        put(
            &mut b,
            TECH + ctechnology::TEMPLATE,
            (TEMPLATE as u64).to_le_bytes(),
        );
        put(
            &mut b,
            TECH + ctechnology::CLASSIFIER,
            ctechnology::CLASSIFIER_ACTIVE.to_le_bytes(),
        );
        put(
            &mut b,
            TECH + ctechnology::LIVE_PROGRESS,
            500_000_i64.to_le_bytes(),
        );
        put(
            &mut b,
            TEMPLATE + ctechnology_template::DATABASE_INDEX,
            1_i32.to_le_bytes(),
        );
        put(
            &mut b,
            TEMPLATE + ctechnology_template::TECHNOLOGY_HANDLE,
            7_u32.to_le_bytes(),
        );
        put(
            &mut b,
            TEMPLATE + ctechnology_template::RESEARCH_COST,
            2_000_000_i64.to_le_bytes(),
        );
        text(&mut b, TEMPLATE + ctechnology_template::ID, b"radio");
        array(
            &mut b,
            TECH + ctechnology::APPLIED_BONUS_IDS,
            IDS,
            u32::from(applied),
        );
    }
    if applied {
        put(&mut b, IDS, (-7_i32).to_le_bytes());
        array(
            &mut b,
            STATUS + ctechnology_status::LIMITED_BONUSES,
            BONUSES,
            1,
        );
        put(&mut b, BONUSES, (BONUS as u64).to_le_bytes());
        put(&mut b, BONUS, (bonus::VTABLE_RVA as u64).to_le_bytes());
        put(&mut b, BONUS + bonus::ID, (-7_i32).to_le_bytes());
        text(&mut b, BONUS + bonus::NAME, b"bonus_radio");
        put(
            &mut b,
            BONUS + bonus::SPEED_BONUS,
            100_000_i64.to_le_bytes(),
        );
        put(
            &mut b,
            BONUS + bonus::AHEAD_REDUCTION,
            200_000_i64.to_le_bytes(),
        );
    }
    b
}

struct MemoryTrace<'a> {
    source: SliceMemory<'a>,
    calls: Cell<usize>,
    reads: RefCell<Vec<(usize, usize)>>,
    fault: Option<usize>,
}
impl<'a> MemoryTrace<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self {
            source: SliceMemory { base: 0, bytes },
            calls: Cell::new(0),
            reads: RefCell::new(vec![]),
            fault: None,
        }
    }
    fn field(&self, address: usize, out: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((address, out.len()));
        if self
            .fault
            .is_some_and(|at| address <= at && at < address.saturating_add(out.len()))
        {
            if !out.is_empty() {
                out[0] = 0xff;
            }
            return false;
        }
        self.source.copy(address, out)
    }
}
impl Memory for MemoryTrace<'_> {
    fn copy(&self, address: usize, out: &mut [u8]) -> bool {
        self.calls.set(self.calls.get() + 1);
        self.field(address, out)
    }
    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        out: &mut [&mut [u8]; N],
    ) -> usize {
        self.calls.set(self.calls.get() + 1);
        for (i, (&at, span)) in addresses.iter().zip(out).enumerate() {
            if !span.is_empty() && !self.field(at, span) {
                return i;
            }
        }
        N
    }
    fn copy_records(
        &self,
        sources: RecordSources<'_>,
        layout: &RecordLayout<'_>,
        out: &mut [u8],
    ) -> usize {
        // Let the established implementation enumerate exact spans; count this
        // as one native guarded batch, not as one guard per decoded field.
        struct Fields<'a, 'b>(&'a MemoryTrace<'b>);
        impl Memory for Fields<'_, '_> {
            fn copy(&self, at: usize, out: &mut [u8]) -> bool {
                self.0.field(at, out)
            }
        }
        self.calls.set(self.calls.get() + 1);
        Fields(self).copy_records(sources, layout, out)
    }
}
fn capture(memory: &impl Memory, rows: &mut [Row], bonuses: &mut [BonusRow]) -> (Outcome, Outcome) {
    walk_country(
        memory,
        0,
        COUNTRY,
        11,
        Ok(Context {
            cost_scale_raw: 100_000,
            cost_debug: 0,
        }),
        rows,
        bonuses,
    )
}

#[test]
fn growing_research_slots_and_bonus_rows_follow_later_original_headers() {
    const POINTERS: usize = 0x20000;
    const OBJECTS: usize = 0x30000;
    let mut bytes = fixture(true, true);
    bytes.resize(OBJECTS + 7_000 * 0x50, 0);
    let source = bytes[SLOT..SLOT + 0x44].to_vec();
    let mut rows = Vec::new();
    let mut bonuses = Vec::new();
    for count in [5_000_u32, 7_000] {
        array(
            &mut bytes,
            STATUS + ctechnology_status::RESEARCH_SLOTS_DATA,
            POINTERS,
            count,
        );
        for index in 0..count as usize {
            let slot = OBJECTS + index * 0x50;
            bytes[slot..slot + source.len()].copy_from_slice(&source);
            put(
                &mut bytes,
                slot + cresearch_slot::INDEX,
                (index as u32).to_le_bytes(),
            );
            put(
                &mut bytes,
                POINTERS + index * 8,
                (slot as u64).to_le_bytes(),
            );
        }
        let memory = MemoryTrace::new(&bytes);
        let result = walk_country_growing(
            &memory,
            0,
            COUNTRY,
            11,
            Ok(Context {
                cost_scale_raw: 100_000,
                cost_debug: 0,
            }),
            &mut rows,
            &mut bonuses,
        );
        assert_eq!(
            result,
            (
                Outcome::Rows {
                    len: count as usize
                },
                Outcome::Rows {
                    len: count as usize
                }
            )
        );
        assert_eq!(rows[count as usize - 1].slot.index, count - 1);
        assert!(
            bonuses[..count as usize]
                .iter()
                .all(|bonus| bonus.resolved == 1)
        );
        assert_eq!(
            memory
                .reads
                .borrow()
                .iter()
                .filter(|&&(at, _)| at == STATUS + ctechnology_status::RESEARCH_SLOTS_DATA)
                .count(),
            1
        );
    }
    let mut short = [Row::default()];
    assert!(matches!(
        capture(&MemoryTrace::new(&bytes), &mut short, &mut []),
        (Outcome::Truncated { .. }, _)
    ));
}

#[test]
fn growing_research_names_and_applied_bonus_populations_have_no_inline_limit() {
    const IDS_LIVE: usize = 0x20000;
    const PROVIDERS: usize = 0x26000;
    const TECH_TEXT: usize = 0x28000;
    const BONUS_TEXT: usize = 0x2a000;
    let mut bytes = fixture(true, true);
    bytes.resize(0x30000, 0);
    for (descriptor, source, len, fill) in [
        (
            TEMPLATE + ctechnology_template::ID,
            TECH_TEXT,
            2_048_usize,
            b't',
        ),
        (BONUS + bonus::NAME, BONUS_TEXT, 4_097_usize, b'b'),
    ] {
        put(&mut bytes, descriptor, (source as u64).to_le_bytes());
        put(&mut bytes, descriptor + 16, (len as u64).to_le_bytes());
        put(&mut bytes, descriptor + 24, (len as u64).to_le_bytes());
        bytes[source..source + len].fill(fill);
    }
    array(
        &mut bytes,
        TECH + ctechnology::APPLIED_BONUS_IDS,
        IDS_LIVE,
        5_000,
    );
    for index in 0..5_000 {
        put(&mut bytes, IDS_LIVE + index * 4, (-7_i32).to_le_bytes());
    }
    array(
        &mut bytes,
        STATUS + ctechnology_status::LIMITED_BONUSES,
        PROVIDERS,
        5_000,
    );
    for index in 0..64 {
        put(
            &mut bytes,
            PROVIDERS + index * 8,
            (BONUS as u64).to_le_bytes(),
        );
    }
    let memory = MemoryTrace::new(&bytes);
    let mut rows = Vec::new();
    let mut bonuses = Vec::new();
    assert_eq!(
        walk_country_growing(
            &memory,
            0,
            COUNTRY,
            11,
            Ok(Context {
                cost_scale_raw: 100_000,
                cost_debug: 0
            }),
            &mut rows,
            &mut bonuses
        ),
        (Outcome::Rows { len: 1 }, Outcome::Rows { len: 5_000 })
    );
    assert_eq!(rows[0].token.as_bytes(), vec![b't'; 2_048]);
    assert!(
        bonuses[..5_000]
            .iter()
            .all(|row| row.value.name.as_bytes() == vec![b'b'; 4_097])
    );
    assert_eq!(
        capture(&MemoryTrace::new(&bytes), &mut [Row::default()], &mut []),
        (
            Outcome::Unavailable {
                reason: "technology token too long"
            },
            Outcome::Unavailable {
                reason: "technology token too long"
            }
        )
    );
}

#[test]
fn idle_and_unbonused_slots_skip_bonus_roster_and_catalogue() {
    for (active, expected_calls) in [(false, 5), (true, 8)] {
        let b = fixture(active, false);
        let m = MemoryTrace::new(&b);
        let mut rows = [Row::default()];
        assert_eq!(
            capture(&m, &mut rows, &mut []),
            (Outcome::Rows { len: 1 }, Outcome::Rows { len: 0 })
        );
        assert_eq!(m.calls.get(), expected_calls);
        assert!(!m.reads.borrow().iter().any(|&(at, _)| at
            == STATUS + ctechnology_status::LIMITED_BONUSES
            || at == STATUS + ctechnology_status::TECHNOLOGIES_DATA));
        if active {
            assert_eq!(rows[0].token.as_bytes(), b"radio");
            assert_eq!(rows[0].progress_raw, 500_000);
        }
    }
}

#[test]
fn applied_ids_join_exact_names_and_raw_values() {
    let b = fixture(true, true);
    let m = MemoryTrace::new(&b);
    let mut rows = [Row::default()];
    let mut bonuses = [BonusRow::default()];
    assert_eq!(
        capture(&m, &mut rows, &mut bonuses),
        (Outcome::Rows { len: 1 }, Outcome::Rows { len: 1 })
    );
    assert_eq!(m.calls.get(), 14);
    assert_eq!(bonuses[0].value.id, -7);
    assert_eq!(bonuses[0].value.name.as_bytes(), b"bonus_radio");
    assert_eq!(bonuses[0].value.speed_bonus_raw, 100_000);
    assert_eq!(bonuses[0].value.ahead_reduction_raw, 200_000);
    assert_eq!(bonuses[0].resolved, 1);
}

#[test]
fn native_slot_indices_survive_permutation_and_shared_bonuses_are_read_once() {
    let mut b = fixture(true, true);
    let second = SLOT + 0x100;
    b.copy_within(SLOT..SLOT + 0x48, second);
    put(&mut b, SLOT + cresearch_slot::INDEX, 1_u32.to_le_bytes());
    array(
        &mut b,
        STATUS + ctechnology_status::RESEARCH_SLOTS_DATA,
        SLOTS,
        2,
    );
    put(&mut b, SLOTS + 8, (second as u64).to_le_bytes());
    let m = MemoryTrace::new(&b);
    let mut rows = vec![Row::default(); 2];
    let mut bonuses = vec![BonusRow::default(); 2];
    assert_eq!(
        capture(&m, &mut rows, &mut bonuses),
        (Outcome::Rows { len: 2 }, Outcome::Rows { len: 2 })
    );
    assert_eq!(
        rows.iter().map(|r| r.slot.index).collect::<Vec<_>>(),
        [0, 1]
    );
    assert_eq!(bonuses.iter().map(|r| r.index).collect::<Vec<_>>(), [0, 1]);
    assert_eq!(m.calls.get(), 19);
    for address in [
        STATUS + ctechnology_status::LIMITED_BONUSES,
        BONUS + bonus::NAME,
    ] {
        assert_eq!(
            m.reads
                .borrow()
                .iter()
                .filter(|&&(at, _)| at == address)
                .count(),
            1
        );
    }
    put(&mut b, SLOT + cresearch_slot::INDEX, 0_u32.to_le_bytes());
    assert!(matches!(
        capture(&MemoryTrace::new(&b), &mut rows, &mut bonuses).0,
        Outcome::Unavailable { .. }
    ));
}

#[test]
fn truncation_is_exact_and_bonus_faults_do_not_discard_progress() {
    let b = fixture(true, true);
    let m = MemoryTrace::new(&b);
    let mut rows = [Row::default()];
    assert_eq!(
        capture(&m, &mut rows, &mut []),
        (
            Outcome::Rows { len: 1 },
            Outcome::Truncated {
                len: 0,
                unread_rows: 1
            }
        )
    );
    assert!(!m.reads.borrow().iter().any(|&(at, _)| at == BONUSES));
    let result = capture(&m, &mut [], &mut []);
    assert_eq!(
        result.0,
        Outcome::Truncated {
            len: 0,
            unread_rows: 1
        }
    );
    assert!(matches!(result.1, Outcome::Unavailable { .. }));
    let mut bonuses = [BonusRow::default()];
    for fault in [
        IDS,
        BONUSES,
        BONUS,
        BONUS + bonus::NAME,
        BONUS + bonus::SPEED_BONUS,
    ] {
        let mut m = MemoryTrace::new(&b);
        m.fault = Some(fault);
        let result = capture(&m, &mut rows, &mut bonuses);
        assert_eq!(result.0, Outcome::Rows { len: 1 });
        assert!(matches!(result.1, Outcome::Unavailable { .. }));
    }
    for fault in [
        COUNTRY + crate::ccountry::TECHNOLOGY_STATUS,
        STATUS,
        SLOTS,
        SLOT,
        TECH,
        TEMPLATE + ctechnology_template::ID,
        TECH + ctechnology::APPLIED_SPEED_BONUS,
    ] {
        let mut m = MemoryTrace::new(&b);
        m.fault = Some(fault);
        assert!(matches!(
            capture(&m, &mut rows, &mut bonuses).0,
            Outcome::Unavailable { .. }
        ));
    }
}

#[test]
fn unresolved_ids_are_retained_and_wrong_owners_are_rejected() {
    let mut b = fixture(true, true);
    put(&mut b, IDS, 99_i32.to_le_bytes());
    let mut rows = [Row::default()];
    let mut bonuses = [BonusRow::default()];
    assert_eq!(
        capture(&MemoryTrace::new(&b), &mut rows, &mut bonuses).1,
        Outcome::Rows { len: 1 }
    );
    assert_eq!(bonuses[0].value.id, 99);
    assert_eq!(bonuses[0].resolved, 0);
    put(&mut b, SLOT + cresearch_slot::STATUS, 0_u64.to_le_bytes());
    assert!(matches!(
        capture(&MemoryTrace::new(&b), &mut rows, &mut bonuses).0,
        Outcome::Unavailable { .. }
    ));
}

#[test]
fn cost_reduction_ids_use_their_own_layout_and_only_unresolved_ids_need_that_roster() {
    use crate::climited_use_tech_cost_reduction as cost;
    let mut b = fixture(true, true);
    let pointer = BONUS + 0x200;
    let roster = BONUSES + 0x100;
    put(&mut b, IDS, 99_i32.to_le_bytes());
    array(
        &mut b,
        STATUS + ctechnology_status::COST_REDUCTIONS,
        roster,
        1,
    );
    put(&mut b, roster, (pointer as u64).to_le_bytes());
    put(&mut b, pointer, (cost::VTABLE_RVA as u64).to_le_bytes());
    put(&mut b, pointer + cost::ID, 99_i32.to_le_bytes());
    text(&mut b, pointer + cost::NAME, b"doctrine_bonus");
    put(&mut b, pointer + cost::REDUCTION, 25_000_i64.to_le_bytes());
    let mut rows = [Row::default()];
    let mut bonuses = [BonusRow::default()];
    assert_eq!(
        capture(&MemoryTrace::new(&b), &mut rows, &mut bonuses).1,
        Outcome::Rows { len: 1 }
    );
    assert_eq!(bonuses[0].value.name.as_bytes(), b"doctrine_bonus");
    assert_eq!(bonuses[0].cost_reduction_raw, Some(25_000));
    assert_eq!(bonuses[0].value.speed_bonus_raw, 0);
    put(&mut b, IDS, (-7_i32).to_le_bytes());
    let mut m = MemoryTrace::new(&b);
    m.fault = Some(STATUS + ctechnology_status::COST_REDUCTIONS);
    assert_eq!(
        capture(&m, &mut rows, &mut bonuses).1,
        Outcome::Rows { len: 1 }
    );
    assert_eq!(bonuses[0].cost_reduction_raw, None);
    assert!(
        !m.reads
            .borrow()
            .iter()
            .any(|&(at, _)| at == STATUS + ctechnology_status::COST_REDUCTIONS)
    );
}

#[test]
fn bonus_rosters_follow_native_population_and_stop_after_resolution() {
    let mut bytes = fixture(true, true);
    array(
        &mut bytes,
        STATUS + ctechnology_status::LIMITED_BONUSES,
        BONUSES,
        4097,
    );
    let memory = MemoryTrace::new(&bytes);
    let mut requested = BonusRow::default();
    requested.value.id = -7;
    assert_eq!(
        resolve_bonuses(&memory, 0, STATUS, &mut [requested.clone()], false),
        Err("research bonus pointer invalid")
    );
    assert_eq!(memory.calls.get(), 2);
    assert_eq!(memory.reads.borrow().len(), 2);

    array(
        &mut bytes,
        STATUS + ctechnology_status::LIMITED_BONUSES,
        BONUSES,
        5_000,
    );
    for index in 0..64 {
        put(
            &mut bytes,
            BONUSES + index * 8,
            (BONUS as u64).to_le_bytes(),
        );
    }
    let memory = MemoryTrace::new(&bytes);
    let mut requested = [requested];
    assert_eq!(
        resolve_bonuses(&memory, 0, STATUS, &mut requested, false),
        Ok(())
    );
    assert_eq!(requested[0].resolved, 1);
    assert!(
        !memory
            .reads
            .borrow()
            .iter()
            .any(|&(at, _)| at == BONUSES + 64 * 8)
    );
    assert!(memory.calls.get() < 10);
}

#[test]
fn fingerprint_ignores_daily_progress_and_follows_assignment_writers() {
    let fingerprint = |bytes: &[u8]| fingerprint_country(&MemoryTrace::new(bytes), 0, COUNTRY);
    let base = fixture(true, true);
    let reference = fingerprint(&base).unwrap();
    assert_eq!(fingerprint(&base).unwrap(), reference);
    // Daily-only writers: progress and banked idle days.
    for (at, value) in [
        (TECH + ctechnology::LIVE_PROGRESS, 900_000_i64),
        (SLOT + cresearch_slot::BANKED_DAYS, 300_000_i64),
    ] {
        let mut changed = base.clone();
        put(&mut changed, at, value.to_le_bytes());
        assert_eq!(
            fingerprint(&changed).unwrap(),
            reference,
            "daily field at {at:#x}"
        );
    }
    // Any-hour writers: assignment, speed, applied days, level, classifier,
    // applied bonuses and the slot roster.
    for (at, bytes) in [
        (
            SLOT + cresearch_slot::TECHNOLOGY_HANDLE,
            8_u32.to_le_bytes().to_vec(),
        ),
        (
            SLOT + cresearch_slot::RESEARCH_SPEED,
            170_000_i64.to_le_bytes().to_vec(),
        ),
        (
            SLOT + cresearch_slot::APPLIED_DAYS,
            100_000_i64.to_le_bytes().to_vec(),
        ),
        (TECH + ctechnology::LEVEL, 1_i32.to_le_bytes().to_vec()),
        (
            TECH + ctechnology::CLASSIFIER,
            ctechnology::CLASSIFIER_COMPLETED.to_le_bytes().to_vec(),
        ),
        (
            TECH + ctechnology::APPLIED_SPEED_BONUS,
            50_000_i64.to_le_bytes().to_vec(),
        ),
        (
            TECH + ctechnology::APPLIED_BONUS_IDS + 8,
            0_u32.to_le_bytes().to_vec(),
        ),
        (
            STATUS + ctechnology_status::RESEARCH_SLOTS_DATA + 8,
            2_u32.to_le_bytes().to_vec(),
        ),
    ] {
        let mut changed = base.clone();
        changed[at..at + bytes.len()].copy_from_slice(&bytes);
        assert_ne!(
            fingerprint(&changed).unwrap(),
            reference,
            "any-hour field at {at:#x}"
        );
    }
    // Cancelling the assignment changes it too; idle slots still fingerprint.
    let idle = fixture(false, false);
    assert_ne!(fingerprint(&idle).unwrap(), reference);
    // No readable status is an error, never an equal fingerprint.
    let mut broken = base.clone();
    put(
        &mut broken,
        COUNTRY + crate::ccountry::TECHNOLOGY_STATUS,
        0_u64.to_le_bytes(),
    );
    assert!(fingerprint(&broken).is_err());
}
