
use crate::table_capture::{Arena, grow_vec};
use collector_observers::retreat_ban_changes;
use engine_native_memory::{Memory, walk::Outcome};
use engine_native::carmy;
use std::collections::HashMap;

#[derive(Clone, Copy, Default)]
struct Row {
    division: u64,
    factor: Option<Result<(i64, u8), &'static str>>,
}

type Terms = HashMap<(usize, usize), ((u64, u64), i64)>;

pub struct Captured {
    rows: Arena<Row>,
    memo: carmy::DigInMemo,
    held: Terms,
    next: Terms,
    blocks: [u64; 3],
}

fn memo(divisions: usize) -> Option<carmy::DigInMemo> {
    carmy::DigInMemo::try_with_capacity(divisions)
        .map_err(crate::allocation::io)
        .ok()
}

impl Captured {
    pub fn new(rows: usize) -> Self {
        Self {
            rows: Arena::new(rows),
            memo: memo(rows).unwrap_or_else(carmy::DigInMemo::vacant),
            held: Terms::new(),
            next: Terms::new(),
            blocks: [0; 3],
        }
    }

    pub fn allocated_bytes(&self) -> usize {
        self.rows.allocated_bytes()
            + self.memo.allocated_bytes()
            + (self.held.capacity() + self.next.capacity())
                * (size_of::<((usize, usize), ((u64, u64), i64))>() + 1)
    }

    pub fn reserve_rows(&mut self, count: usize) {
        grow_vec(&mut self.rows.first, count);
        self.reserve_memo(count);
    }

    pub fn grow(&mut self, capacity: &mut usize) {
        self.rows.grow(capacity);
        self.reserve_memo(*capacity);
    }

    fn reserve_memo(&mut self, divisions: usize) {
        if self.memo.capacity() < divisions
            && let Some(memo) = memo(divisions)
        {
            self.memo = memo;
        }
    }

    pub fn reset(&mut self) {
        self.rows.reset();
    }

    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        seeds: &[carmy::DivisionSeed],
        outcome: Outcome,
    ) {
        self.capture_with(memory, base, seeds, outcome, &retreat_ban_changes::Observer);
    }

    fn capture_with(
        &mut self,
        memory: &impl Memory,
        base: usize,
        seeds: &[carmy::DivisionSeed],
        outcome: Outcome,
        changes: &dyn retreat_ban_changes::Changes,
    ) {
        self.reset();
        self.memo.clear();
        self.blocks = [0; 3];
        self.rows.required = outcome.len();
        if !grow_vec(&mut self.rows.first, outcome.len())
            || !matches!(outcome, Outcome::Rows { .. })
            || outcome.len() > seeds.len()
        {
            return;
        }
        self.next.clear();
        let (held, next, blocks) = (&self.held, &mut self.next, &mut self.blocks);
        for (index, seed) in seeds[..outcome.len()].iter().enumerate() {
            let leader = seed
                .selected
                .and_then(|selected| selected.leader)
                .and_then(Result::ok)
                .filter(|&leader| leader != 0);
            let factor =
                carmy::counter_dig_in_selection_with(memory, base, seed, &mut self.memo, |root| {
                    let Some((leader, certificate)) =
                        leader.and_then(|leader| Some((leader, changes.sequence(leader)?)))
                    else {
                        blocks[2] += 1;
                        return carmy::commander_entrenchment(memory, base, root);
                    };
                    let term = match held.get(&(leader, root)) {
                        Some(&(read_under, term)) if read_under == certificate => {
                            blocks[0] += 1;
                            Ok(term)
                        }
                        _ => {
                            blocks[1] += 1;
                            carmy::commander_entrenchment(memory, base, root)
                        }
                    };
                    if let Ok(term) = term
                        && engine_allocation::advisory(|| {
                            engine_allocation::reserve_map(&mut *next, 1)
                        })
                        .is_ok()
                    {
                        next.insert((leader, root), (certificate, term));
                    }
                    term
                });
            self.rows.first[index] = Row {
                division: seed.division,
                factor: Some(factor),
            };
            self.rows.len += 1;
        }
        std::mem::swap(&mut self.held, &mut self.next);
    }

    pub fn blocks(&self) -> [u64; 3] {
        self.blocks
    }

    pub fn settle(&mut self, outcome: Outcome) {
        self.rows.required = outcome.len();
        self.rows.len = if matches!(outcome, Outcome::Rows { .. }) && outcome.len() <= self.rows.len
        {
            outcome.len()
        } else {
            0
        };
    }

    pub fn factor(&self, division: u64, index: usize) -> Option<i64> {
        self.selection(division, index).map(|(factor, _)| factor)
    }

    pub fn commander_block(&self, division: u64, index: usize) -> Option<u8> {
        self.selection(division, index).map(|(_, block)| block)
    }

    fn selection(&self, division: u64, index: usize) -> Option<(i64, u8)> {
        let row = self.rows.first.get(index)?;
        if index >= self.rows.len || row.division != division {
            return None;
        }
        row.factor?.ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed(division: u64, orders_group: Option<usize>) -> carmy::DivisionSeed {
        carmy::DivisionSeed {
            division,
            pointer: 0x2000,
            orders_group,
            ..Default::default()
        }
    }

    #[test]
    fn each_division_keeps_its_own_factor_or_refusal() {
        let memory = engine_native_memory::memory::SliceMemory {
            base: 0x1000,
            bytes: &[],
        };
        let mut captured = Captured::new(0);
        captured.capture(
            &memory,
            0,
            &[seed(9, Some(0)), seed(11, None)],
            Outcome::Rows { len: 2 },
        );
        assert_eq!(captured.factor(9, 0), Some(100_000));
        assert_eq!(captured.factor(11, 1), None);
        assert_eq!(captured.factor(11, 0), None);
    }

    struct Commanders {
        bytes: Vec<u8>,
        reads: std::cell::RefCell<Vec<usize>>,
    }

    const GROUPS: [usize; 2] = [0x1000, 0x1400];
    const LEADERS: [usize; 2] = [0x2000, 0x3000];
    const MODIFIERS: [usize; 2] = [0x5000, 0x5100];
    const DEFINITIONS: usize = 0x3000_0000;

    impl Commanders {
        fn new(terms: [i64; 2]) -> Self {
            use engine_native::{carmy_leader, corders_group as group, cunit_leader};
            fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
                bytes[at..at + value.len()].copy_from_slice(value);
            }
            let mut bytes = vec![0; 0x9000];
            for index in 0..2 {
                let (group, leader) = (GROUPS[index], LEADERS[index]);
                put(&mut bytes, group, &(group::VTABLE_RVA.0 as u64).to_le_bytes());
                put(
                    &mut bytes,
                    group + group::HANDLE,
                    &group::REFERENCE_TYPE.to_le_bytes(),
                );
                put(
                    &mut bytes,
                    group + group::LEADER,
                    &(leader as u64).to_le_bytes(),
                );
                put(
                    &mut bytes,
                    leader,
                    &(carmy_leader::VTABLE_RVA.0 as u64).to_le_bytes(),
                );
                put(
                    &mut bytes,
                    leader + cunit_leader::TYPE,
                    &cunit_leader::REFERENCE_TYPE.to_le_bytes(),
                );
                let accumulator =
                    leader + carmy_leader::MOVEMENT_MODIFIER + carmy_leader::MODIFIER_ACCUMULATOR;
                put(
                    &mut bytes,
                    accumulator,
                    &(MODIFIERS[index] as u64).to_le_bytes(),
                );
                put(&mut bytes, accumulator + 8, &1_u32.to_le_bytes());
                put(&mut bytes, accumulator + 12, &1_u32.to_le_bytes());
                put(
                    &mut bytes,
                    MODIFIERS[index],
                    &engine_executable::modifier_keys::COMBAT_ENTRENCHMENT.to_le_bytes(),
                );
                put(
                    &mut bytes,
                    MODIFIERS[index] + 8,
                    &terms[index].to_le_bytes(),
                );
            }
            Self {
                bytes,
                reads: Default::default(),
            }
        }

        fn count(&self, address: usize) -> usize {
            self.reads
                .borrow()
                .iter()
                .filter(|&&read| read == address)
                .count()
        }
    }

    impl Memory for Commanders {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.reads.borrow_mut().push(address);
            if address == engine_native_parts::modifier_definition::TABLE_RVA.0 && output.len() == 8 {
                output.copy_from_slice(&(DEFINITIONS as u64).to_le_bytes());
                return true;
            }
            if address == engine_native_parts::modifier_definition::TRADE_FEATURE_FLAGS_RVA.0 {
                output.fill(0);
                return true;
            }
            if (DEFINITIONS..DEFINITIONS + 0x4_0000).contains(&address) {
                output.fill(0);
                return true;
            }
            engine_native_memory::memory::SliceMemory {
                base: 0,
                bytes: &self.bytes,
            }
            .copy(address, output)
        }
    }

    #[test]
    fn a_group_and_its_commander_are_read_once_for_all_their_divisions() {
        let mut seeds = Vec::new();
        for (index, group) in [0, 0, 1, 0, 1, 0, 0].into_iter().enumerate() {
            seeds.push(carmy::DivisionSeed {
                division: (index as u64 + 1) << 32,
                pointer: 0x8000 + index * 0x100,
                orders_group: Some(GROUPS[group]),
                class: Some(if index == 3 { 1 } else { 0 }),
                ..Default::default()
            });
        }
        seeds.push(seed(9 << 32, Some(0)));
        seeds.push(seed(10 << 32, None));
        let memory = Commanders::new([-150_000, 2_500]);
        let expected: Vec<_> = seeds
            .iter()
            .map(|seed| carmy::counter_dig_in_factor(&memory, 0, seed))
            .collect();
        assert_eq!(memory.count(GROUPS[0]), 5);
        assert_eq!(
            expected[..3],
            [Ok(-50_000), Ok(-50_000), Ok(102_500)],
            "the fixture selects each commander's term"
        );
        memory.reads.borrow_mut().clear();
        let _ = carmy::counter_dig_in_factor(&memory, 0, &seeds[0]);
        let block = memory.count(MODIFIERS[0]);
        assert_ne!(block, 0);
        let mut captured = Captured::new(seeds.len());
        for _ in 0..2 {
            memory.reads.borrow_mut().clear();
            captured.capture(&memory, 0, &seeds, Outcome::Rows { len: seeds.len() });
            for (index, (seed, expected)) in seeds.iter().zip(&expected).enumerate() {
                assert_eq!(
                    captured.rows.first[index]
                        .factor
                        .map(|factor| factor.map(|(f, _)| f)),
                    Some(*expected)
                );
                assert_eq!(
                    captured.factor(seed.division, index),
                    expected.ok(),
                    "{index}"
                );
                let block = captured.commander_block(seed.division, index);
                match seed.orders_group {
                    Some(0) => assert_eq!(block, Some(carmy::COMMANDER_BLOCK_NONE)),
                    None => assert_eq!(block, None),
                    Some(_) if expected.is_ok() => assert_ne!(block, None, "{index}"),
                    Some(_) => assert_eq!(block, None),
                }
            }
            for (group, leader) in GROUPS.into_iter().zip(LEADERS) {
                assert_eq!(memory.count(group), 1);
                assert_eq!(memory.count(leader), 1);
            }
            for modifiers in MODIFIERS {
                assert_eq!(memory.count(modifiers), block);
            }
            assert_eq!(
                memory.count(engine_native_parts::modifier_definition::TRADE_FEATURE_FLAGS_RVA.0),
                1
            );
        }
    }

    #[test]
    fn a_settled_census_capture_publishes_what_the_status_rows_capture_does() {
        let mut seeds = Vec::new();
        for (index, group) in [0, 1, 0, 1, 0].into_iter().enumerate() {
            seeds.push(carmy::DivisionSeed {
                division: (index as u64 + 1) << 32,
                pointer: 0x8000 + index * 0x100,
                orders_group: Some(GROUPS[group]),
                class: Some(0),
                ..Default::default()
            });
        }
        seeds.push(seed(9 << 32, Some(0)));
        seeds.push(seed(10 << 32, None));
        let memory = Commanders::new([-150_000, 2_500]);
        let census = Outcome::Rows { len: seeds.len() };
        let mut statuses: Vec<_> = (0..=seeds.len()).map(|len| Outcome::Rows { len }).collect();
        statuses.extend([
            Outcome::Rows {
                len: seeds.len() + 1,
            },
            Outcome::Truncated {
                len: 3,
                unread_rows: 4,
            },
            Outcome::Unavailable {
                reason: "status refused",
            },
        ]);
        for status in statuses {
            let mut expected = Captured::new(seeds.len());
            expected.capture(&memory, 0, &seeds, status);
            let mut settled = Captured::new(seeds.len());
            settled.capture(&memory, 0, &seeds, census);
            assert_eq!(settled.rows.len, seeds.len());
            settled.settle(status);
            assert_eq!(settled.rows.len, expected.rows.len, "{status:?}");
            assert_eq!(settled.rows.required, expected.rows.required, "{status:?}");
            for (index, seed) in seeds.iter().enumerate() {
                assert_eq!(
                    settled.factor(seed.division, index),
                    expected.factor(seed.division, index),
                    "{status:?} {index}"
                );
                if index < expected.rows.len {
                    let (settled, expected) =
                        (settled.rows.first[index], expected.rows.first[index]);
                    assert_eq!(settled.division, expected.division);
                    assert_eq!(settled.factor, expected.factor);
                }
            }
        }
    }

    struct Certificates(std::cell::RefCell<HashMap<usize, u64>>);
    impl retreat_ban_changes::Changes for Certificates {
        fn sequence(&self, leader: usize) -> Option<(u64, u64)> {
            self.0.borrow().get(&leader).map(|&sequence| (1, sequence))
        }
    }

    fn commanded(division: u64, index: usize) -> carmy::DivisionSeed {
        use engine_native::carmy_leader;
        let leader = LEADERS[index];
        carmy::DivisionSeed {
            division,
            pointer: 0x8000,
            orders_group: Some(GROUPS[index]),
            class: Some(0),
            selected: Some(carmy_leader::Selected {
                leader: Some(Ok(leader)),
                handle: None,
                group_identity: true,
                block: Ok(Some((
                    leader + carmy_leader::MOVEMENT_MODIFIER + carmy_leader::MODIFIER_ACCUMULATOR,
                    false,
                ))),
                training: None,
            }),
            ..Default::default()
        }
    }

    #[test]
    fn a_quiet_commander_block_is_carried_and_a_touched_one_read() {
        let seeds = [commanded(1, 0), commanded(2, 0), commanded(3, 1)];
        let outcome = Outcome::Rows { len: seeds.len() };
        let certificates = Certificates(std::cell::RefCell::new(HashMap::from([
            (LEADERS[0], 1),
            (LEADERS[1], 1),
        ])));
        let factors = |captured: &Captured| {
            [0, 1, 2].map(|index| captured.factor(seeds[index].division, index))
        };
        let mut captured = Captured::new(seeds.len());
        let memory = Commanders::new([-150_000, 2_500]);
        captured.capture_with(&memory, 0, &seeds, outcome, &certificates);
        assert_eq!(captured.blocks(), [0, 2, 0]);
        assert_eq!(
            factors(&captured),
            [Some(-50_000), Some(-50_000), Some(102_500)]
        );

        let memory = Commanders::new([-10_000, 7_000]);
        captured.capture_with(&memory, 0, &seeds, outcome, &certificates);
        assert_eq!(captured.blocks(), [2, 0, 0]);
        assert!(memory.reads.borrow().is_empty());
        assert_eq!(
            factors(&captured),
            [Some(-50_000), Some(-50_000), Some(102_500)]
        );
        assert_eq!(
            captured.commander_block(seeds[2].division, 2),
            Some(carmy::COMMANDER_BLOCK_ORDINARY)
        );

        certificates.0.borrow_mut().insert(LEADERS[0], 2);
        captured.capture_with(&memory, 0, &seeds, outcome, &certificates);
        assert_eq!(captured.blocks(), [1, 1, 0]);
        assert_eq!(
            factors(&captured),
            [Some(90_000), Some(90_000), Some(102_500)]
        );

        let uncertified = Certificates(Default::default());
        captured.capture_with(&memory, 0, &seeds, outcome, &uncertified);
        assert_eq!(captured.blocks(), [0, 0, 2]);
        assert_eq!(
            factors(&captured),
            [Some(90_000), Some(90_000), Some(107_000)]
        );
        captured.capture_with(&memory, 0, &seeds, outcome, &certificates);
        assert_eq!(captured.blocks(), [0, 2, 0]);
    }

    #[test]
    fn a_held_term_serves_only_its_own_leader() {
        let certificates = Certificates(std::cell::RefCell::new(HashMap::from([
            (LEADERS[0], 1),
            (LEADERS[1], 1),
        ])));
        let outcome = Outcome::Rows { len: 1 };
        let first = [commanded(1, 0)];
        let mut captured = Captured::new(1);
        let memory = Commanders::new([-150_000, 2_500]);
        captured.capture_with(&memory, 0, &first, outcome, &certificates);
        assert_eq!(captured.blocks(), [0, 1, 0]);
        let mut reused = commanded(1, 1);
        reused.selected.as_mut().unwrap().block = first[0].selected.unwrap().block;
        captured.capture_with(&memory, 0, &[reused], outcome, &certificates);
        assert_eq!(captured.blocks(), [0, 1, 0]);
    }

    #[test]
    fn incomplete_census_publishes_no_factor() {
        let memory = engine_native_memory::memory::SliceMemory {
            base: 0x1000,
            bytes: &[],
        };
        let mut captured = Captured::new(1);
        captured.capture(
            &memory,
            0,
            &[seed(9, Some(0))],
            Outcome::Truncated {
                len: 1,
                unread_rows: 1,
            },
        );
        assert_eq!(captured.factor(9, 0), None);
        assert_eq!(captured.rows.len, 0);
    }
}
