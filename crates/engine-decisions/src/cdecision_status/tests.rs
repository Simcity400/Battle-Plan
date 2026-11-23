use super::*;
use engine_native_memory::memory::SliceMemory;

#[test]
fn decision_random_items_preserve_full_store_and_signed_words() {
    let mut world = ccountry::synthetic::World::new(IMAGE, 3, 0x1000);
    for country in 1..=3 {
        let owner = world.object(country);
        let status = world.alloc(RANDOM_ITEMS + 16);
        let items = world.alloc(RANDOM_ITEM_STRIDE * 2);
        let definition = world.alloc(cdecision::TOKEN + 32);
        world.u64(owner + ccountry::DECISION_STATUS, status as u64);
        world.u64(status, (IMAGE + VTABLE_RVA.0) as u64);
        world.u64(status + COUNTRY, owner as u64);
        world.array(status + RANDOM_ITEMS, items, 2);
        world.u64(definition, (IMAGE + cdecision::VTABLE_RVA.0) as u64);
        world.put(definition + cdecision::TOKEN, b"test\xff");
        world.u64(definition + cdecision::TOKEN + 16, 5);
        world.u64(definition + cdecision::TOKEN + 24, 15);
        for (ordinal, (count, target)) in [(i32::MIN, i32::MAX), (17, -1)].into_iter().enumerate() {
            let entry = items + ordinal * RANDOM_ITEM_STRIDE;
            world.u64(entry, (IMAGE + RANDOM_ITEM_VTABLE_RVA.0) as u64);
            world.u64(entry + 8, definition as u64);
            world.put(entry + 16, &count.to_le_bytes());
            world.put(entry + 20, &target.to_le_bytes());
        }
    }
    assert_eq!(
        random_item_demand(&world.memory(), IMAGE, world.game_state()),
        Ok(RandomItemDemand {
            rows: 6,
            bytes: 30
        })
    );
    let mut rows = vec![RandomItemRow::default(); 1];
    let mut tokens = vec![0; 1];
    let walk = walk_random_items(
        &world.memory(),
        IMAGE,
        world.game_state(),
        &mut rows,
        &mut tokens,
    );
    assert_eq!(walk.outcome, Outcome::Rows { len: 6 });
    for (index, row) in rows[..walk.outcome.len()].iter().enumerate() {
        assert_eq!(
            (row.country, row.ordinal),
            (index as u32 / 2 + 1, index as u32 % 2)
        );
        assert_eq!(
            (row.count, row.target),
            [(i32::MIN, i32::MAX), (17, -1)][index % 2]
        );
        assert_eq!(
            &tokens[row.token_start..row.token_start + row.token_len],
            b"test\xff"
        );
    }
    let status = engine_native_memory::read::<u64>(&world.memory(), world.object(1) + ccountry::DECISION_STATUS)
        .unwrap() as usize;
    let items = engine_native_memory::read::<u64>(&world.memory(), status + RANDOM_ITEMS).unwrap() as usize;
    world.u64(items, 0);
    assert_eq!(
        walk_random_items(
            &world.memory(),
            IMAGE,
            world.game_state(),
            &mut rows,
            &mut tokens
        )
        .outcome,
        Outcome::Unavailable {
            reason: "decision random-item vtable mismatch"
        }
    );
}

const IMAGE: usize = 0x0001_4000_0000;
const COUNTRY_ADDRESS: usize = 0x1000;
const STATUS_ADDRESS: usize = 0x2200;
const DEFINITION_ADDRESS: usize = 0x3000;
const RECORD_ADDRESS: usize = 0x3800;
const VECTOR_ADDRESS: usize = 0x3900;

fn put(bytes: &mut [u8], address: usize, value: &[u8]) {
    let at = address - COUNTRY_ADDRESS;
    bytes[at..at + value.len()].copy_from_slice(value);
}

fn qword(bytes: &mut [u8], address: usize, value: usize) {
    put(bytes, address, &(value as u64).to_le_bytes());
}

fn word(bytes: &mut [u8], address: usize, value: i32) {
    put(bytes, address, &value.to_le_bytes());
}

fn empty() -> Vec<u8> {
    let mut bytes = vec![0; 0x4000];
    qword(&mut bytes, COUNTRY_ADDRESS, IMAGE + ccountry::VTABLE_RVA.0);
    qword(
        &mut bytes,
        COUNTRY_ADDRESS + ccountry::DECISION_STATUS,
        STATUS_ADDRESS,
    );
    qword(&mut bytes, STATUS_ADDRESS, IMAGE + VTABLE_RVA.0);
    qword(&mut bytes, STATUS_ADDRESS + COUNTRY, COUNTRY_ADDRESS);
    qword(
        &mut bytes,
        DEFINITION_ADDRESS,
        IMAGE + cdecision::VTABLE_RVA.0,
    );
    let token = DEFINITION_ADDRESS + cdecision::TOKEN;
    put(&mut bytes, token, b"test\xff");
    qword(&mut bytes, token + 16, 5);
    qword(&mut bytes, token + 24, 15);
    bytes
}

fn vector(bytes: &mut [u8], offset: usize, count: i32) {
    qword(bytes, STATUS_ADDRESS + offset, VECTOR_ADDRESS);
    word(bytes, STATUS_ADDRESS + offset + 8, count);
    word(bytes, STATUS_ADDRESS + offset + 12, count);
    qword(bytes, VECTOR_ADDRESS, RECORD_ADDRESS);
}

fn run(bytes: &[u8], output: &mut Vec<Row>, tokens: &mut Vec<u8>) -> Walk {
    walk(
        &SliceMemory {
            base: COUNTRY_ADDRESS,
            bytes,
        },
        IMAGE,
        COUNTRY_ADDRESS,
        output,
        tokens,
    )
}

#[test]
fn every_decision_container_and_token_arena_grows_from_original_mod_sized_headers() {
    let mut bytes = empty();
    let sources = [RE_ENABLE, REMOVE, ACTIVE_TIMED, TARGETED, TARGETED_TIMED];
    for index in 0..sources.len() {
        let record = RECORD_ADDRESS + index * 0x80;
        let (vtable, definition, days) = match index {
            0 | 1 => (
                cdecision_cooldown::VTABLE_RVA,
                cdecision_cooldown::DEFINITION,
                cdecision_cooldown::DAYS,
            ),
            2 => (
                ctimed_decision::VTABLE_RVA,
                ctimed_decision::DEFINITION,
                ctimed_decision::DAYS,
            ),
            _ => (
                ctargeted_decision::VTABLE_RVA,
                ctargeted_decision::DEFINITION,
                ctargeted_decision::DAYS,
            ),
        };
        qword(&mut bytes, record, IMAGE + vtable.0);
        qword(&mut bytes, record + definition, DEFINITION_ADDRESS);
        word(&mut bytes, record + days, i32::MIN + index as i32);
        if index == 2 {
            word(&mut bytes, record + ctimed_decision::STATE, i32::MAX - 2);
        } else if index >= 3 {
            word(
                &mut bytes,
                record + ctargeted_decision::STATE,
                i32::MAX - index as i32,
            );
            word(&mut bytes, record + ctargeted_decision::NUMERIC_TARGET, -1);
            put(&mut bytes, record + ctargeted_decision::IGNORE, &[1]);
        }
    }
    let install = |bytes: &mut Vec<u8>, count: usize| {
        let base = 0x8000;
        bytes.resize(base + sources.len() * count * 8 - COUNTRY_ADDRESS, 0);
        for (index, &source) in sources.iter().enumerate() {
            let data = base + index * count * 8;
            qword(bytes, STATUS_ADDRESS + source, data);
            word(bytes, STATUS_ADDRESS + source + 8, count as i32);
            word(bytes, STATUS_ADDRESS + source + 12, count as i32);
            for ordinal in 0..count {
                qword(bytes, data + ordinal * 8, RECORD_ADDRESS + index * 0x80);
            }
        }
    };
    install(&mut bytes, 2);
    let mut rows = Vec::new();
    let mut tokens = Vec::new();
    let first = walk(
        &SliceMemory {
            base: COUNTRY_ADDRESS,
            bytes: &bytes,
        },
        IMAGE,
        COUNTRY_ADDRESS,
        &mut rows,
        &mut tokens,
    );
    assert_eq!(first.outcome, Outcome::Rows { len: 10 });
    assert_eq!((first.bytes_len, first.required_bytes), (50, 50));
    install(&mut bytes, 5_000);
    let later = walk(
        &SliceMemory {
            base: COUNTRY_ADDRESS,
            bytes: &bytes,
        },
        IMAGE,
        COUNTRY_ADDRESS,
        &mut rows,
        &mut tokens,
    );
    assert_eq!(later.outcome, Outcome::Rows { len: 25_000 });
    assert_eq!((later.bytes_len, later.required_bytes), (125_000, 125_000));
    assert_eq!(
        later.counts,
        Some(Counts {
            re_enable: 5_000,
            remove: 5_000,
            timed: 5_000,
            targeted: 5_000,
            targeted_timed: 5_000,
            taken: 0
        })
    );
    for (index, row) in rows[..25_000].iter().enumerate() {
        let container = index / 5_000;
        assert_eq!(row.container as usize, container);
        assert_eq!(row.ordinal as usize, index % 5_000);
        assert_eq!(row.days, i32::MIN + container as i32);
        assert_eq!(
            &tokens[row.token_start..row.token_start + row.token_len],
            b"test\xff"
        );
    }
}

#[test]
fn authenticated_empty_state_differs_from_missing_or_corrupt_containers() {
    let mut bytes = empty();
    let result = run(&bytes, &mut Vec::new(), &mut Vec::new());
    assert_eq!(result.outcome, Outcome::Rows { len: 0 });
    assert_eq!(result.counts, Some(Counts::default()));
    qword(&mut bytes, COUNTRY_ADDRESS + ccountry::DECISION_STATUS, 0);
    assert!(matches!(
        run(&bytes, &mut Vec::new(), &mut Vec::new()).outcome,
        Outcome::Unavailable { .. }
    ));
    qword(
        &mut bytes,
        COUNTRY_ADDRESS + ccountry::DECISION_STATUS,
        STATUS_ADDRESS,
    );
    word(&mut bytes, STATUS_ADDRESS + TARGETED + 12, 1);
    let invalid = run(&bytes, &mut Vec::new(), &mut Vec::new());
    assert!(matches!(invalid.outcome, Outcome::Unavailable { .. }));
    assert_eq!(invalid.counts, None);
}

#[test]
fn timed_rows_keep_nonactive_states_and_full_signed_fields() {
    let mut bytes = empty();
    vector(&mut bytes, ACTIVE_TIMED, 1);
    qword(
        &mut bytes,
        RECORD_ADDRESS,
        IMAGE + ctimed_decision::VTABLE_RVA.0,
    );
    qword(
        &mut bytes,
        RECORD_ADDRESS + ctimed_decision::DEFINITION,
        DEFINITION_ADDRESS,
    );
    word(&mut bytes, RECORD_ADDRESS + ctimed_decision::DAYS, i32::MIN);
    word(
        &mut bytes,
        RECORD_ADDRESS + ctimed_decision::STATE,
        i32::MAX,
    );
    let mut output = vec![Row::default(); 1];
    let mut tokens = vec![0; 5];
    let result = run(&bytes, &mut output, &mut tokens);
    assert_eq!(result.outcome, Outcome::Rows { len: 1 });
    assert_eq!(output[0].container, Container::Timed);
    assert_eq!(output[0].days, i32::MIN);
    assert_eq!(output[0].lifecycle, i32::MAX);
    assert_eq!(&tokens, b"test\xff");
    assert_eq!(result.bytes_len, 5);
}

#[test]
fn targeted_container_labels_match_native_serializer_sources() {
    let mut bytes = empty();
    vector(&mut bytes, 0xe8, 1);
    vector(&mut bytes, 0x100, 1);
    qword(
        &mut bytes,
        RECORD_ADDRESS,
        IMAGE + ctargeted_decision::VTABLE_RVA.0,
    );
    qword(
        &mut bytes,
        RECORD_ADDRESS + ctargeted_decision::DEFINITION,
        DEFINITION_ADDRESS,
    );
    let mut output = vec![Row::default(); 2];
    let result = run(&bytes, &mut output, &mut vec![0; 10]);
    assert_eq!(result.outcome, Outcome::Rows { len: 2 });
    assert_eq!(output[0].container, Container::Targeted);
    assert_eq!(output[0].container as u8, 3);
    assert_eq!(output[1].container, Container::TargetedTimed);
    assert_eq!(output[1].container as u8, 4);
    assert_eq!(result.counts.unwrap().targeted, 1);
    assert_eq!(result.counts.unwrap().targeted_timed, 1);
}

#[test]
fn targeted_rows_retain_both_native_slots_and_validate_the_bool_only() {
    let mut bytes = empty();
    vector(&mut bytes, TARGETED, 1);
    qword(
        &mut bytes,
        RECORD_ADDRESS,
        IMAGE + ctargeted_decision::VTABLE_RVA.0,
    );
    qword(
        &mut bytes,
        RECORD_ADDRESS + ctargeted_decision::DEFINITION,
        DEFINITION_ADDRESS,
    );
    word(
        &mut bytes,
        RECORD_ADDRESS + ctargeted_decision::COUNTRY_TARGET,
        -1,
    );
    word(
        &mut bytes,
        RECORD_ADDRESS + ctargeted_decision::NUMERIC_TARGET,
        i32::MIN,
    );
    word(&mut bytes, RECORD_ADDRESS + ctargeted_decision::DAYS, -471);
    word(&mut bytes, RECORD_ADDRESS + ctargeted_decision::STATE, -7);
    put(
        &mut bytes,
        RECORD_ADDRESS + ctargeted_decision::IGNORE,
        &[1],
    );
    let mut output = vec![Row::default(); 1];
    assert_eq!(
        run(&bytes, &mut output, &mut vec![0; 5]).outcome,
        Outcome::Rows { len: 1 }
    );
    assert_eq!(output[0].target_kind, TargetKind::Numeric);
    assert_eq!(output[0].country_target, u32::MAX);
    assert_eq!(output[0].numeric_target, 0x8000_0000);
    assert_eq!(output[0].lifecycle, -7);
    assert_eq!(output[0].days, -471);
    assert_eq!(output[0].ignore, 1);
    for (country, numeric, kind) in [
        (0, 0, TargetKind::Numeric),
        (9, 0, TargetKind::Country),
        (0, 7, TargetKind::Numeric),
    ] {
        word(
            &mut bytes,
            RECORD_ADDRESS + ctargeted_decision::COUNTRY_TARGET,
            country,
        );
        word(
            &mut bytes,
            RECORD_ADDRESS + ctargeted_decision::NUMERIC_TARGET,
            numeric,
        );
        assert_eq!(
            run(&bytes, &mut output, &mut vec![0; 5]).outcome,
            Outcome::Rows { len: 1 }
        );
        assert_eq!(output[0].target_kind, kind);
    }
    put(
        &mut bytes,
        RECORD_ADDRESS + ctargeted_decision::IGNORE,
        &[2],
    );
    assert_eq!(
        run(&bytes, &mut output, &mut vec![0; 5]).outcome,
        Outcome::Unavailable {
            reason: "targeted decision ignore flag invalid"
        }
    );
}

#[test]
fn cooldown_rows_and_token_storage_grow_to_the_full_demand() {
    let mut bytes = empty();
    vector(&mut bytes, RE_ENABLE, 2);
    qword(&mut bytes, VECTOR_ADDRESS + 8, RECORD_ADDRESS);
    qword(
        &mut bytes,
        RECORD_ADDRESS,
        IMAGE + cdecision_cooldown::VTABLE_RVA.0,
    );
    qword(
        &mut bytes,
        RECORD_ADDRESS + cdecision_cooldown::DEFINITION,
        DEFINITION_ADDRESS,
    );
    word(
        &mut bytes,
        RECORD_ADDRESS + cdecision_cooldown::DAYS,
        i32::MIN,
    );
    let mut output = vec![Row::default(); 1];
    let result = run(&bytes, &mut output, &mut vec![0; 9]);
    assert_eq!(result.outcome, Outcome::Rows { len: 2 });
    assert_eq!((result.bytes_len, result.required_bytes), (10, 10));
    assert_eq!(output[0].days, i32::MIN);
    assert_eq!(output[1].ordinal, 1);
}

#[test]
fn wrong_owners_definitions_and_heap_strings_never_become_empty_rows() {
    let mut bytes = empty();
    vector(&mut bytes, REMOVE, 1);
    qword(
        &mut bytes,
        RECORD_ADDRESS,
        IMAGE + cdecision_cooldown::VTABLE_RVA.0,
    );
    qword(
        &mut bytes,
        RECORD_ADDRESS + cdecision_cooldown::DEFINITION,
        DEFINITION_ADDRESS,
    );
    let token = DEFINITION_ADDRESS + cdecision::TOKEN;
    qword(&mut bytes, token, 0x4000);
    qword(&mut bytes, token + 16, 17);
    qword(&mut bytes, token + 24, 31);
    put(&mut bytes, 0x4000, b"a_long_definition");
    let mut output = vec![Row::default(); 1];
    let mut tokens = vec![0; 17];
    assert_eq!(
        run(&bytes, &mut output, &mut tokens).outcome,
        Outcome::Rows { len: 1 }
    );
    assert_eq!(&tokens, b"a_long_definition");
    put(&mut bytes, 0x4011, &[9]);
    assert_eq!(
        run(&bytes, &mut output, &mut tokens).outcome,
        Outcome::Unavailable {
            reason: "decision token terminator unreadable or invalid"
        }
    );
    put(&mut bytes, 0x4011, &[0]);
    qword(&mut bytes, DEFINITION_ADDRESS, 0);
    assert_eq!(
        run(&bytes, &mut output, &mut tokens).outcome,
        Outcome::Unavailable {
            reason: "decision definition vtable mismatch"
        }
    );
    qword(&mut bytes, STATUS_ADDRESS + COUNTRY, COUNTRY_ADDRESS + 1);
    assert_eq!(
        run(&bytes, &mut output, &mut tokens).outcome,
        Outcome::Unavailable {
            reason: "decision status owner mismatch"
        }
    );
}

#[test]
fn description_seed_reads_only_demanded_targeted_rows_and_preserves_first_match() {
    let mut bytes = empty();
    put(&mut bytes, DEFINITION_ADDRESS + cdecision::TOKEN, b"testx");
    vector(&mut bytes, TARGETED, 1);
    qword(
        &mut bytes,
        RECORD_ADDRESS,
        IMAGE + ctargeted_decision::VTABLE_RVA.0,
    );
    qword(
        &mut bytes,
        RECORD_ADDRESS + ctargeted_decision::DEFINITION,
        DEFINITION_ADDRESS,
    );
    word(
        &mut bytes,
        RECORD_ADDRESS + ctargeted_decision::NUMERIC_TARGET,
        -17,
    );
    let seed_data = 0x3c00;
    qword(&mut bytes, STATUS_ADDRESS + RANDOM_ITEMS, seed_data);
    word(&mut bytes, STATUS_ADDRESS + RANDOM_ITEMS + 8, 2);
    word(&mut bytes, STATUS_ADDRESS + RANDOM_ITEMS + 12, 2);
    for (ordinal, seed) in [(0, i32::MIN), (1, 21)] {
        let at = seed_data + ordinal * RANDOM_ITEM_STRIDE;
        qword(&mut bytes, at, IMAGE + RANDOM_ITEM_VTABLE_RVA.0);
        qword(&mut bytes, at + RANDOM_ITEM_DEFINITION, DEFINITION_ADDRESS);
        word(&mut bytes, at + RANDOM_ITEM_SEED, seed);
        word(&mut bytes, at + RANDOM_ITEM_TARGET, -17);
    }
    let demand = [crate::border_effect_demand::Name { start: 0, len: 5 }];
    let run = |bytes: &[u8], demand: &[crate::border_effect_demand::Name]| {
        let mut output = vec![Row::default()];
        let result = walk_with_description_demand_buffer(&SliceMemory {
                base: COUNTRY_ADDRESS,
                bytes,
            }, IMAGE, 0, COUNTRY_ADDRESS, engine_native_memory::walk::Buffer::Growing(&mut output), engine_native_memory::walk::Buffer::Growing(&mut vec![0; 5]), crate::border_effect_demand::Demand {
                names: demand,
                bytes: b"testx",
            });
        (result.outcome, output[0].description_seed)
    };
    assert_eq!(
        run(&bytes, &demand),
        (Outcome::Rows { len: 1 }, Some(Some(i32::MIN)))
    );
    word(
        &mut bytes,
        RECORD_ADDRESS + ctargeted_decision::NUMERIC_TARGET,
        -18,
    );
    assert_eq!(run(&bytes, &demand), (Outcome::Rows { len: 1 }, Some(None)));
    word(&mut bytes, STATUS_ADDRESS + RANDOM_ITEMS + 12, 3);
    assert_eq!(run(&bytes, &[]), (Outcome::Rows { len: 1 }, None));
    assert!(matches!(
        run(&bytes, &demand).0,
        Outcome::Unavailable {
            reason: "decision seed header unreadable or invalid"
        }
    ));
    put(
        &mut bytes,
        DEFINITION_ADDRESS + cdecision::FIXED_RANDOM_SEED,
        &[1],
    );
    assert_eq!(run(&bytes, &demand), (Outcome::Rows { len: 1 }, None));
}

#[test]
fn country_description_seed_resolves_alias_and_matches_first_definition_key_in_batched_items() {
    use crate::border_effect_demand::Name;
    use engine_native::cgame_state;
    use std::cell::RefCell;
    const GS: usize = 0x4000;
    const COUNTRIES: usize = 0x4800;
    const LINKS: usize = 0x4880;
    const SEEDS: usize = 0x3c00;
    const ALIAS_DEFINITION: usize = 0x3e00;
    let mut bytes = empty();
    put(&mut bytes, DEFINITION_ADDRESS + cdecision::TOKEN, b"testx");
    vector(&mut bytes, TARGETED, 2);
    qword(&mut bytes, VECTOR_ADDRESS + 8, RECORD_ADDRESS);
    qword(
        &mut bytes,
        RECORD_ADDRESS,
        IMAGE + ctargeted_decision::VTABLE_RVA.0,
    );
    qword(
        &mut bytes,
        RECORD_ADDRESS + ctargeted_decision::DEFINITION,
        DEFINITION_ADDRESS,
    );
    word(
        &mut bytes,
        RECORD_ADDRESS + ctargeted_decision::COUNTRY_TARGET,
        3,
    );
    word(
        &mut bytes,
        RECORD_ADDRESS + ctargeted_decision::NUMERIC_TARGET,
        99,
    );
    word(&mut bytes, COUNTRY_ADDRESS + ccountry::ID, 3);
    for (offset, pointer) in [
        (cgame_state::COUNTRIES, COUNTRIES),
        (cgame_state::COUNTRY_LINKS, LINKS),
    ] {
        qword(&mut bytes, GS + offset, pointer);
        word(&mut bytes, GS + offset + 8, 4);
        word(&mut bytes, GS + offset + 12, 4);
    }
    qword(&mut bytes, COUNTRIES + 8, COUNTRY_ADDRESS);
    word(&mut bytes, LINKS + 4, 1);
    word(&mut bytes, LINKS + 12, 1);
    qword(&mut bytes, ALIAS_DEFINITION, IMAGE + cdecision::VTABLE_RVA.0);
    put(&mut bytes, ALIAS_DEFINITION + cdecision::TOKEN, b"TESTX");
    qword(&mut bytes, ALIAS_DEFINITION + cdecision::TOKEN + 16, 5);
    qword(&mut bytes, ALIAS_DEFINITION + cdecision::TOKEN + 24, 15);
    qword(&mut bytes, STATUS_ADDRESS + RANDOM_ITEMS, SEEDS);
    word(&mut bytes, STATUS_ADDRESS + RANDOM_ITEMS + 8, 3);
    word(&mut bytes, STATUS_ADDRESS + RANDOM_ITEMS + 12, 3);
    for (index, definition, target, seed) in [
        (0, ALIAS_DEFINITION, 99, 17),
        (1, ALIAS_DEFINITION, 1, i32::MIN),
        (2, DEFINITION_ADDRESS, 1, 21),
    ] {
        let at = SEEDS + index * RANDOM_ITEM_STRIDE;
        qword(&mut bytes, at, IMAGE + RANDOM_ITEM_VTABLE_RVA.0);
        qword(&mut bytes, at + RANDOM_ITEM_DEFINITION, definition);
        word(&mut bytes, at + RANDOM_ITEM_TARGET, target);
        word(&mut bytes, at + RANDOM_ITEM_SEED, seed);
    }
    struct Trace<'a> {
        memory: SliceMemory<'a>,
        reads: RefCell<Vec<(usize, usize)>>,
    }
    impl Memory for Trace<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.reads.borrow_mut().push((address, output.len()));
            self.memory.copy(address, output)
        }
    }
    let memory = Trace {
        memory: SliceMemory {
            base: COUNTRY_ADDRESS,
            bytes: &bytes,
        },
        reads: RefCell::new(Vec::new()),
    };
    let mut output = vec![Row::default(); 2];
    let demand = [Name { start: 0, len: 5 }];
    let result = walk_with_description_demand_buffer(&memory, IMAGE, GS, COUNTRY_ADDRESS, engine_native_memory::walk::Buffer::Growing(&mut output), engine_native_memory::walk::Buffer::Growing(&mut vec![0; 10]), crate::border_effect_demand::Demand {
            names: &demand,
            bytes: b"testx",
        });
    assert_eq!(result.outcome, Outcome::Rows { len: 2 });
    assert!(
        output
            .iter()
            .all(|row| row.country_target == 3 && row.description_seed == Some(Some(i32::MIN)))
    );
    let reads = memory.reads.borrow();
    assert_eq!(
        reads
            .iter()
            .filter(|(at, _)| *at == GS + cgame_state::COUNTRIES
                || *at == GS + cgame_state::COUNTRY_LINKS)
            .count(),
        2
    );
    assert_eq!(
        reads
            .iter()
            .filter(|(at, _)| (SEEDS..SEEDS + 3 * RANDOM_ITEM_STRIDE).contains(at))
            .map(|(_, len)| *len)
            .collect::<Vec<_>>(),
        [24, 24, 24, 24]
    );
}
