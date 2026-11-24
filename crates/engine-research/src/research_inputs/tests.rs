use super::*;
use engine_native_memory::memory::{RecordLayout, RecordSources, SliceMemory};
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
        COUNTRY + engine_native::ccountry::TECHNOLOGY_STATUS,
        (STATUS as u64).to_le_bytes(),
    );
    put(
        &mut b,
        STATUS,
        (ctechnology_status::VTABLE_RVA.0 as u64).to_le_bytes(),
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
        (cresearch_slot::VTABLE_RVA.0 as u64).to_le_bytes(),
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
        put(
            &mut b,
            TECH,
            (ctechnology::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
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
        put(&mut b, BONUS, (bonus::VTABLE_RVA.0 as u64).to_le_bytes());
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
fn capture(memory: &impl Memory, rows: &mut Vec<Row>, bonuses: &mut Vec<BonusRow>) -> (Outcome, Outcome) {
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
        let result = walk_country(
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
}

#[test]
fn research_names_and_applied_bonus_populations_have_no_inline_limit() {
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
        walk_country(
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
    let mut exact_rows = vec![Row::default()];
    let mut exact_bonuses = vec![BonusRow::default(); 5_000];
    assert_eq!(
        walk_country_buffer(
            &memory,
            0,
            COUNTRY,
            11,
            Ok(Context {
                cost_scale_raw: 100_000,
                cost_debug: 0
            }),
            engine_native_memory::walk::Buffer::Exact(&mut exact_rows),
            engine_native_memory::walk::Buffer::Exact(&mut exact_bonuses),
        ),
        (Outcome::Rows { len: 1 }, Outcome::Rows { len: 5_000 })
    );
    assert_eq!(exact_rows[..], rows[..1]);
    assert_eq!(exact_bonuses[..], bonuses[..5_000]);
}

#[test]
fn applied_ids_join_exact_names_and_raw_values() {
    let b = fixture(true, true);
    let m = MemoryTrace::new(&b);
    let mut rows = vec![Row::default()];
    let mut bonuses = vec![BonusRow::default()];
    assert_eq!(
        capture(&m, &mut rows, &mut bonuses),
        (Outcome::Rows { len: 1 }, Outcome::Rows { len: 1 })
    );
    assert_eq!(m.calls.get(), 13);
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
    assert_eq!(m.calls.get(), 14);
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
fn count_probes_are_exact_and_bonus_faults_do_not_discard_progress() {
    let b = fixture(true, true);
    let m = MemoryTrace::new(&b);
    let mut rows = vec![Row::default()];
    let context = Ok(Context {
        cost_scale_raw: 100_000,
        cost_debug: 0,
    });
    assert_eq!(
        walk_country_buffer(
            &m,
            0,
            COUNTRY,
            11,
            context,
            engine_native_memory::walk::Buffer::Growing(&mut rows),
            engine_native_memory::walk::Buffer::Count,
        ),
        (
            Outcome::Rows { len: 1 },
            Outcome::Truncated {
                len: 0,
                unread_rows: 1
            }
        )
    );
    assert!(!m.reads.borrow().iter().any(|&(at, _)| at == BONUSES));
    let result = walk_country_buffer(
        &m,
        0,
        COUNTRY,
        11,
        context,
        engine_native_memory::walk::Buffer::Count,
        engine_native_memory::walk::Buffer::Count,
    );
    assert_eq!(
        result.0,
        Outcome::Truncated {
            len: 0,
            unread_rows: 1
        }
    );
    assert!(matches!(result.1, Outcome::Unavailable { .. }));
    let mut bonuses = vec![BonusRow::default()];
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
        COUNTRY + engine_native::ccountry::TECHNOLOGY_STATUS,
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
    let mut rows = vec![Row::default()];
    let mut bonuses = vec![BonusRow::default()];
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
    put(&mut b, pointer, (cost::VTABLE_RVA.0 as u64).to_le_bytes());
    put(&mut b, pointer + cost::ID, 99_i32.to_le_bytes());
    text(&mut b, pointer + cost::NAME, b"doctrine_bonus");
    put(&mut b, pointer + cost::REDUCTION, 25_000_i64.to_le_bytes());
    let mut rows = vec![Row::default()];
    let mut bonuses = vec![BonusRow::default()];
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
fn governance_batch_research_inputs_serial_and_batched_equality() {
    use engine_governance_tests::{Trace, World};
    let world = World::new(3);
    let serial_memory = Trace::new(&world, "ResearchInputs/ResearchAppliedBonuses serial");
    let memory = Trace::new(&world, "ResearchInputs/ResearchAppliedBonuses batch");
    let context = Context::read(&world, 0);
    for id in 1..=3 {
        let mut before = vec![Row::default(); 3];
        let mut after = vec![Row::default(); 3];
        let mut old_bonuses = vec![BonusRow::default(); 2];
        let mut new_bonuses = vec![BonusRow::default(); 2];
        let old = super::serial::walk_country_buffer(
            &serial_memory,
            0,
            World::country(id),
            id,
            context,
            engine_native_memory::walk::Buffer::Growing(&mut before),
            engine_native_memory::walk::Buffer::Growing(&mut old_bonuses),
        );
        let new = walk_country(
            &memory,
            0,
            World::country(id),
            id,
            context,
            &mut after,
            &mut new_bonuses,
        );
        assert_eq!(old, (Outcome::Rows { len: 3 }, Outcome::Rows { len: 2 }));
        assert_eq!(new, old);
        assert_eq!(after, before);
        assert_eq!(new_bonuses, old_bonuses);
        assert_eq!((serial_memory.count(), memory.count()), (20, 14));
        assert!(memory.fallbacks.witnesses().is_empty());
        serial_memory.clear();
        memory.clear();
    }
}

#[test]
fn governance_batch_research_inputs_persistent_and_one_shot_every_serial_operand() {
    use engine_governance_tests::{
        SLOT, SLOTS, TECH, TEMPLATE, Trace, World, addresses, never_harsher,
    };
    let world = World::new(1);
    let context = Context::read(&world, 0);
    let capture = |memory: &Trace<'_, World>, serial: bool| {
        let mut rows = vec![Row::default(); 3];
        let mut bonuses = vec![BonusRow::default(); 2];
        let outcome = if serial {
            super::serial::walk_country_buffer(
                memory,
                0,
                World::country(1),
                1,
                context,
                engine_native_memory::walk::Buffer::Growing(&mut rows),
                engine_native_memory::walk::Buffer::Growing(&mut bonuses),
            )
        } else {
            walk_country(
                memory,
                0,
                World::country(1),
                1,
                context,
                &mut rows,
                &mut bonuses,
            )
        };
        rows.truncate(outcome.0.len());
        bonuses.truncate(outcome.1.len());
        (outcome, rows, bonuses)
    };
    let country = World::country(1);
    for (offset, reader) in [
        (SLOTS, "slot_pointers"),
        (SLOT + crate::cresearch_slot::STATUS, "slots"),
        (
            TECH + crate::ctechnology::LIVE_PROGRESS,
            "assigned_technologies",
        ),
        (
            TECH + crate::ctechnology::APPLIED_SPEED_BONUS,
            "assigned_technologies",
        ),
        (
            TEMPLATE + crate::ctechnology_template::DATABASE_INDEX,
            "assigned_templates",
        ),
    ] {
        for once in [false, true] {
            let before = Trace::fault(&world, "ResearchInputs serial", country + offset, once);
            let after = Trace::fault(&world, "ResearchInputs batch", country + offset, once);
            let old = capture(&before, true);
            let new = capture(&after, false);
            if once {
                never_harsher(old.0.0, new.0.0);
                never_harsher(old.0.1, new.0.1);
            } else {
                assert_eq!(new, old);
            }
            assert_eq!(
                &*after.fallbacks.witnesses(),
                &[reader],
                "{reader}, once={once}"
            );
        }
    }
    let census = Trace::new(&world, "ResearchInputs serial census");
    capture(&census, true);
    for address in addresses(&census) {
        let before = Trace::fault(&world, "ResearchInputs serial", address, false);
        let after = Trace::fault(&world, "ResearchInputs batch", address, false);
        assert_eq!(
            capture(&after, false),
            capture(&before, true),
            "persistent {address:#x}"
        );
        let fallbacks = after.fallbacks.witnesses();
        for reader in [
            "slot_pointers",
            "slots",
            "assigned_technologies",
            "assigned_templates",
        ] {
            assert!(
                fallbacks.iter().filter(|&&r| r == reader).count() <= 1,
                "unbounded fallback {reader}"
            );
        }
        drop(fallbacks);
        let before = Trace::fault(&world, "ResearchInputs serial one-shot", address, true);
        let after = Trace::fault(&world, "ResearchInputs batch one-shot", address, true);
        let old = capture(&before, true).0;
        let new = capture(&after, false).0;
        never_harsher(old.0, new.0);
        never_harsher(old.1, new.1);
    }
}

#[test]
fn text_handles_inline_and_long_owned_tokens() {
    let mut mem = vec![0u8; 512];
    let short = b"electronic_mechanical_engineering";
    let long = [b'a'; 128];
    mem[64..64 + short.len()].copy_from_slice(short);
    mem[256..256 + long.len()].copy_from_slice(&long);
    let memory = SliceMemory {
        base: 0,
        bytes: &mem,
    };
    let short_text = Text::from_source(&memory, 64, short.len(), None).unwrap();
    assert_eq!(short_text.as_bytes(), short);
    assert_eq!(short_text.allocated_bytes(), 0);
    let long_text = Text::from_source(&memory, 256, long.len(), None).unwrap();
    assert_eq!(long_text.as_bytes(), &long);
    assert!(long_text.allocated_bytes() >= long.len());
    assert_eq!(long_text.clone().as_bytes(), &long);
}
