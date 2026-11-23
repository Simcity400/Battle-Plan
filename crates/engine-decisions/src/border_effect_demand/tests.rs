use super::*;
use engine_native_memory::read_census::ReadCensus;
use engine_native_parts::reference_bucket as bucket;
use std::collections::BTreeMap;

const GS: usize = 0x1000;
const COMBAT: usize = 0x3000;
const EVENT: usize = 0x9000;
const CONDITIONAL: usize = 0xd000;
const FIRST: usize = 0xf000;
const SECOND: usize = 0x13000;

#[derive(Default)]
struct Fixture(BTreeMap<usize, u8>);

impl Memory for Fixture {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        for (index, byte) in destination.iter_mut().enumerate() {
            let Some(value) = self.0.get(&(address + index)) else {
                return false;
            };
            *byte = *value;
        }
        true
    }
}

impl Fixture {
    fn put(&mut self, address: usize, bytes: &[u8]) {
        self.0.extend(
            bytes
                .iter()
                .enumerate()
                .map(|(index, byte)| (address + index, *byte)),
        );
    }

    fn u64(&mut self, address: usize, value: usize) {
        self.put(address, &(value as u64).to_le_bytes());
    }
    fn u32(&mut self, address: usize, value: u32) {
        self.put(address, &value.to_le_bytes());
    }
    fn array(&mut self, address: usize, data: usize, count: u32) {
        self.u64(address, data);
        self.u32(address + 8, count);
        self.u32(address + 12, count);
    }

    fn text(&mut self, address: usize, bytes: &[u8]) {
        assert!(bytes.len() <= 15);
        self.put(address, &[0; 32]);
        self.put(address, bytes);
        self.u64(address + 16, bytes.len());
        self.u64(address + 24, 15);
    }

    fn effect(&mut self, address: usize, vtable: usize, children: &[usize]) {
        self.u64(address, vtable);
        self.array(address + 8, address + 0x400, children.len() as u32);
        for (index, child) in children.iter().enumerate() {
            self.u64(address + 0x400 + index * 8, *child);
        }
    }

    fn event_fixture() -> Self {
        let mut f = Self::default();
        f.array(GS + cgame_state::COMBATS + 8, 0x2000, 1);
        f.u64(0x2000, COMBAT);
        f.u64(COMBAT, combat::VTABLE_RVA.0);
        f.u64(COMBAT + cland_combat::BODY, combat::BODY_VTABLE_RVA.0);
        f.u64(
            COMBAT + cland_combat::HANDLE,
            (42_u64 << 32 | 4714) as usize,
        );
        for (index, address) in [0x4000, 0x5000].into_iter().enumerate() {
            f.u64(COMBAT + cland_combat::ATTACKER + index * 8, address);
            f.u64(address, side::VTABLE_RVA.0);
            f.u64(address + cland_combatant::COMBAT, COMBAT);
            f.put(address + cland_combatant::IS_ATTACKER, &[(1 - index) as u8]);
            f.text(address + side::WIN_EVENT, b"");
            f.text(address + side::LOSE_EVENT, b"");
        }
        f.text(0x4000 + side::WIN_EVENT, b"Test.7");
        f.array(namespace::TABLE_RVA.0, 0x6000, 1);
        f.text(0x6000, b"TEST");
        let hash = b"test".iter().fold(0x811c_9dc5_u32, |hash, byte| {
            (hash ^ u32::from(*byte)).wrapping_mul(0x0100_0193)
        });
        f.u32(0x6000 + namespace::NAME_HASH, hash);
        let id = 1_000_007_u32;
        let handle = (u64::from(id) << 32) | u64::from(EVENT_TYPE);
        f.u64(
            creference_registry::LOW_ROOTS_RVA.0 + EVENT_TYPE as usize * 8,
            0x7000,
        );
        f.u64(0x7000 + creference_registry::ENTRIES, 0x8000);
        f.u32(0x7000 + creference_registry::COUNT, 1);
        f.u32(0x7000 + creference_registry::MASK, 7);
        f.put(0x7000 + creference_registry::OVERFLOW_DISTANCE, &[5]);
        f.put(0x8000, &[0; 14 * bucket::STRIDE]);
        f.put(0x8000 + 13 * bucket::STRIDE + bucket::DISTANCE, &[255]);
        let hash = handle
            .rotate_left(32)
            .wrapping_mul(creference_registry::HASH_MULTIPLIER);
        let index = (((hash >> 32) as u32 ^ hash as u32) & 7) as usize;
        f.put(0x8000 + index * bucket::STRIDE + bucket::DISTANCE, &[1]);
        f.u64(
            0x8000 + index * bucket::STRIDE + bucket::TYPE,
            handle as usize,
        );
        f.u64(0x8000 + index * bucket::STRIDE + bucket::OBJECT, EVENT);
        f.u64(EVENT, EVENT_VTABLE.0);
        f.u64(EVENT + 8, handle as usize);
        f.put(EVENT + cevent::OPERATIVE_CHROME, &[0]);
        f.array(EVENT + cevent::OPTIONS, 0xb000, 1);
        f.u64(0xb000, 0xc000);
        f.u64(EVENT + IMMEDIATE, OPTION_VTABLE.0);
        f.effect(
            EVENT + IMMEDIATE + OPTION_EFFECT,
            EFFECT_VTABLE.0,
            &[CONDITIONAL],
        );
        f.u64(0xc000, OPTION_VTABLE.0);
        f.effect(0xc000 + OPTION_EFFECT, EFFECT_VTABLE.0, &[SECOND]);
        f.effect(CONDITIONAL, IF_VTABLE.0, &[FIRST]);
        f.u64(CONDITIONAL + ELSE, 0x11000);
        f.effect(0x11000, IF_VTABLE.0, &[SECOND]);
        f.u64(0x11000 + ELSE, 0);
        f.effect(FIRST, REMOVE_VTABLE.0, &[]);
        f.text(FIRST + DECISION_NAME, b"first_decision");
        f.effect(SECOND, REMOVE_VTABLE.0, &[]);
        f.text(SECOND + DECISION_NAME, b"second_decision");
        f
    }
}

#[test]
fn zero_border_combats_reads_only_the_manager_header() {
    let mut f = Fixture::default();
    f.array(GS + cgame_state::COMBATS + 8, 0, 0);
    let census = ReadCensus::new(&f);
    assert_eq!(
        walk(&census, 0, GS, &mut Vec::new(), &mut Vec::new(), &mut vec![0; 8]).outcome,
        Outcome::Rows { len: 0 }
    );
    assert_eq!((census.calls(), census.bytes()), (1, 16));
    f.array(GS + cgame_state::COMBATS + 8, 0x2000, 1);
    f.u64(0x2000, COMBAT);
    f.u64(COMBAT, cland_combat::VTABLE_RVA.0);
    assert_eq!(
        walk(&f, 0, GS, &mut Vec::new(), &mut Vec::new(), &mut vec![0; 8]).outcome,
        Outcome::Rows { len: 0 }
    );
}

#[test]
fn native_event_resolution_unions_immediate_option_and_attached_else() {
    let f = Fixture::event_fixture();
    let mut tokens = vec![Token::default(); 4];
    let mut stack = vec![Frame::default(); 8];
    let result = walk(&f, 0, GS, &mut tokens, &mut stack, &mut vec![0; 8]);
    assert_eq!(result.outcome, Outcome::Rows { len: 3 });
    assert_eq!((result.required_rows, result.required_stack), (3, 4));
    assert!(result.complete(&tokens, &stack));
    assert!(matches(&f, tokens[0], b"FIRST_DECISION").unwrap());
    assert!(matches(&f, tokens[1], b"second_decision").unwrap());
    assert_eq!(tokens[1], tokens[2]);
    assert!(!matches(&f, tokens[0], b"other").unwrap());
}

#[test]
fn faults_and_cycles_cannot_become_no_demand() {
    let mut f = Fixture::event_fixture();
    let result = walk(
        &f,
        0,
        GS,
        &mut vec![Token::default(); 1],
        &mut vec![Frame::default(); 1],
        &mut vec![0; 8],
    );
    assert_eq!(result.outcome, Outcome::Rows { len: 3 });
    assert_eq!((result.required_rows, result.required_stack), (3, 4));
    f.u64(FIRST + 8, FIRST + 0x400);
    f.u32(FIRST + 16, 1);
    f.u32(FIRST + 20, 1);
    f.u64(FIRST + 0x400, CONDITIONAL);
    assert_eq!(
        walk(
            &f,
            0,
            GS,
            &mut vec![Token::default(); 8],
            &mut vec![Frame::default(); 8],
            &mut vec![0; 8]
        )
        .outcome,
        Outcome::Unavailable {
            reason: "border demand effect cycle"
        }
    );
    f.0.remove(&(COMBAT + cland_combat::BODY));
    assert!(matches!(
        walk(&f, 0, GS, &mut Vec::new(), &mut Vec::new(), &mut vec![0; 8]).outcome,
        Outcome::Unavailable { .. }
    ));
}

#[test]
fn multiple_options_operative_and_refused_classes_do_not_demand_seed_reads() {
    for kind in 0..3 {
        let mut f = Fixture::event_fixture();
        match kind {
            0 => f.array(EVENT + cevent::OPTIONS, 0xb000, 2),
            1 => f.put(EVENT + cevent::OPERATIVE_CHROME, &[1]),
            _ => {
                f.u64(CONDITIONAL, 0x0299_ffff);
                f.u64(SECOND, 0x0299_ffff);
            }
        }
        let result = walk(&f, 0, GS, &mut Vec::new(), &mut vec![Frame::default(); 8], &mut vec![0; 8]);
        assert_eq!(result.outcome, Outcome::Rows { len: 0 });
    }
}

#[test]
fn name_matching_preserves_case_nul_and_locale_refusal() {
    let mut f = Fixture::default();
    f.text(0x1000, b"A\0ignored");
    let name = token(&f, 0x1000).unwrap();
    assert_eq!(matches(&f, name, b"a\0different"), Ok(true));
    f.text(0x1000, &[0x80]);
    assert_eq!(
        matches(&f, token(&f, 0x1000).unwrap(), &[0x80]),
        Err("border demand name comparison locale unavailable")
    );
}

#[test]
fn unknown_combat_classes_are_outside_border_demand() {
    let mut f = Fixture::default();
    f.array(GS + cgame_state::COMBATS + 8, 0x2000, 1);
    f.u64(0x2000, COMBAT);
    f.u64(COMBAT, 0x0123_4567);
    assert_eq!(
        walk(&f, 0, GS, &mut Vec::new(), &mut Vec::new(), &mut Vec::new()).outcome,
        Outcome::Rows { len: 0 }
    );
}

#[test]
fn repeated_resolved_events_walk_their_effect_trees_once() {
    use std::cell::Cell;
    let mut f = Fixture::event_fixture();
    f.array(GS + cgame_state::COMBATS + 8, 0x2000, 2);
    f.u64(0x2008, COMBAT);
    for combatant in [0x4000, 0x5000] {
        for offset in [side::WIN_EVENT, side::LOSE_EVENT] {
            f.text(combatant + offset, b"TEST.7");
        }
    }
    struct Probe<'a> {
        fixture: &'a Fixture,
        event_reads: Cell<usize>,
    }
    impl Memory for Probe<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if address == EVENT {
                self.event_reads.set(self.event_reads.get() + 1);
            }
            self.fixture.copy(address, output)
        }
    }
    let memory = Probe {
        fixture: &f,
        event_reads: Cell::new(0),
    };
    let result = walk(
        &memory,
        0,
        GS,
        &mut vec![Token::default(); 4],
        &mut vec![Frame::default(); 8],
        &mut vec![0; 1],
    );
    assert_eq!(result.outcome, Outcome::Rows { len: 3 });
    assert_eq!(result.required_events, 1);
    assert_eq!(memory.event_reads.get(), 1);
}

#[test]
fn demand_names_are_copied_once_and_preserve_nul_case_and_locale_rules() {
    let mut f = Fixture::default();
    f.text(0x1000, b"A\0ignored");
    let native = [token(&f, 0x1000).unwrap()];
    let census = ReadCensus::new(&f);
    let mut names = [Name::default()];
    let mut bytes = [0; 9];
    assert_eq!(copy_names(&census, &native, &mut names, &mut bytes), Ok(9));
    assert_eq!((census.calls(), census.bytes()), (1, 9));
    f.0.clear();
    assert_eq!(matches_name(names[0], &bytes, b"a\0different"), Ok(true));
    assert_eq!(matches_name(names[0], &bytes, b"b"), Ok(false));
    assert_eq!(
        matches_name(Name { start: 0, len: 1 }, &[0x80], &[0x80]),
        Err("border demand name comparison locale unavailable")
    );
    assert_eq!(
        copy_names(&f, &native, &mut names, &mut [0; 8]),
        Err("border demand name capacity")
    );
}

#[test]
fn event_identifiers_use_bulk_reads_without_a_length_cap_and_keep_signed_suffix_wrapping() {
    let mut f = Fixture::default();
    let prefix = vec![b'a'; 180];
    let mut identifier = prefix.clone();
    identifier.extend_from_slice(b".\xff7");
    f.put(0x20000, &identifier);
    f.array(namespace::TABLE_RVA.0, 0x21000, 1);
    f.put(0x21000, &[0; 32]);
    f.u64(0x21000, 0x22000);
    f.u64(0x21000 + 16, prefix.len());
    f.u64(0x21000 + 24, prefix.len());
    f.put(0x22000, &prefix);
    f.put(0x22000 + prefix.len(), &[0]);
    let hash = prefix.iter().fold(0x811c_9dc5_u32, |hash, byte| {
        (hash ^ u32::from(*byte)).wrapping_mul(0x0100_0193)
    });
    f.u32(0x21000 + namespace::NAME_HASH, hash);
    let census = ReadCensus::new(&f);
    assert_eq!(
        event_id(
            &census,
            0,
            Token {
                address: 0x20000,
                len: identifier.len()
            }
        ),
        Ok(Some(999_517))
    );
    assert_eq!(census.calls(), 10);
}
