use super::*;
use crate::{
    cattache_relation, cdocking_rights_relation, cgame_state, cguarantee_relation,
    cmarket_access_rights_relation, coffer_military_access_relation, memory::SliceMemory,
};

const BASE: usize = 0;
const GS: usize = 0x1000;
const COUNTRIES: [usize; 3] = [0x4000, 0x5000, 0x6000];
const DIPLOMACY: [usize; 3] = [0x8000, 0x9000, 0xa000];
const RELATION: [usize; 2] = [0x1c000, 0x1c800];

fn p32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}
fn p64(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}
fn array(bytes: &mut [u8], at: usize, data: usize, values: &[u64]) {
    p64(bytes, at, data as u64);
    p32(bytes, at + 8, values.len() as u32);
    p32(bytes, at + 12, values.len() as u32);
    for (index, value) in values.iter().enumerate() {
        p64(bytes, data + index * 8, *value);
    }
}
/// The owned-relation vector of owner index `owner`.
fn own(bytes: &mut [u8], owner: usize, relations: &[usize]) {
    let relations: Vec<u64> = relations.iter().map(|&r| r as u64).collect();
    array(
        bytes,
        DIPLOMACY[owner] + cdiplomacy_status::OWNED_RELATIONS,
        0x3400 + owner * 0x40,
        &relations,
    );
}
/// Status object for owner slot `owner`, target slot `target`.
fn status(owner: usize, target: usize) -> usize {
    0x10000 + owner * 0x4000 + target * 0x1000
}
fn matrix_data(owner: usize) -> usize {
    0x3200 + owner * 0x40
}
fn serializable_data(owner: usize, target: usize) -> usize {
    0x2800 + owner * 0x80 + target * 0x20
}

/// Three countries in slots 1..3, each with a complete four-slot matrix.
fn fixture() -> Vec<u8> {
    let mut bytes = vec![0_u8; 0x24000];
    array(
        &mut bytes,
        GS + cgame_state::COUNTRIES,
        0x3000,
        &[
            u64::MAX,
            COUNTRIES[0] as u64,
            COUNTRIES[1] as u64,
            COUNTRIES[2] as u64,
        ],
    );
    p64(&mut bytes, GS + cgame_state::COUNTRY_LINKS, 0x3100);
    p32(&mut bytes, GS + cgame_state::COUNTRY_LINKS + 8, 4);
    p32(&mut bytes, GS + cgame_state::COUNTRY_LINKS + 12, 4);
    for (index, value) in [0_u32, 1, 2, 3].into_iter().enumerate() {
        p32(&mut bytes, 0x3100 + index * 4, value);
    }
    for owner in 0..3 {
        let country = COUNTRIES[owner];
        let diplomacy = DIPLOMACY[owner];
        p64(&mut bytes, country, (BASE + ccountry::VTABLE_RVA) as u64);
        p32(&mut bytes, country + ccountry::ID, owner as u32 + 1);
        p64(
            &mut bytes,
            country + ccountry::DIPLOMACY_STATUS,
            diplomacy as u64,
        );
        p64(
            &mut bytes,
            diplomacy,
            (BASE + cdiplomacy_status::VTABLE_RVA) as u64,
        );
        p64(
            &mut bytes,
            diplomacy + cdiplomacy_status::COUNTRY,
            country as u64,
        );
        let slots: Vec<u64> = (0..4).map(|t| status(owner, t) as u64).collect();
        array(
            &mut bytes,
            diplomacy + cdiplomacy_status::RELATIONS,
            matrix_data(owner),
            &slots,
        );
        for target in 0..4 {
            let object = status(owner, target);
            p64(&mut bytes, object, (BASE + VTABLE_RVA) as u64);
            p64(&mut bytes, object + DIPLOMACY_STATUS, diplomacy as u64);
            p32(&mut bytes, object + TARGET_COUNTRY, target as u32);
            array(
                &mut bytes,
                object + SERIALIZABLE_RELATIONS,
                serializable_data(owner, target),
                &[],
            );
        }
        // Slot zero is the reserved sentinel; the walk never reaches it.
        p32(&mut bytes, status(owner, 0) + TARGET_COUNTRY, 1);
    }
    bytes
}

fn relation(bytes: &mut [u8], object: usize, vtable: usize, type_id: u32, first: u32, second: u32) {
    p64(bytes, object, (BASE + vtable) as u64);
    p32(bytes, object + crelation::TYPE_ID, type_id);
    p32(bytes, object + crelation::FIRST_COUNTRY, first);
    p32(bytes, object + crelation::SECOND_COUNTRY, second);
}

fn memory(bytes: &[u8]) -> SliceMemory<'_> {
    SliceMemory { base: 0, bytes }
}

fn country(index: usize) -> ccountry::Country {
    ccountry::Country {
        pointer: COUNTRIES[index],
        id: index as u32 + 1,
        canonical_slot: index as u32 + 1,
    }
}

#[test]
fn zero_relation_keeps_pointer_presence_and_raw_bytes_and_refuses_short_reads() {
    let mut bytes = fixture();
    let zero = status(0, 0);
    p32(&mut bytes, zero + TARGET_COUNTRY, 0);
    bytes[zero + CO_BELLIGERENT] = 1;
    assert_eq!(
        zero_relation(&memory(&bytes), BASE, country(0)),
        Ok(ZeroRelation {
            co_belligerent: 1,
            ..Default::default()
        })
    );
    p64(&mut bytes, zero + WAR_RELATION, RELATION[0] as u64);
    bytes[RELATION[0] + crelation::RETIRED] = 0x7f;
    assert_eq!(
        zero_relation(&memory(&bytes), BASE, country(0)),
        Ok(ZeroRelation {
            war_present: 1,
            war_retired: 0x7f,
            co_belligerent: 1
        })
    );
    assert!(
        zero_relation(
            &memory(&bytes[..RELATION[0] + crelation::RETIRED]),
            BASE,
            country(0)
        )
        .is_err()
    );
    p64(&mut bytes, matrix_data(0), 0);
    assert_eq!(
        zero_relation(&memory(&bytes), BASE, country(0)),
        Err("relation status missing")
    );
}

#[test]
fn co_belligerents_read_each_candidate_byte_in_both_vectors() {
    let mut bytes = fixture();
    links_vector(
        &mut bytes,
        DIPLOMACY[0] + cdiplomacy_status::WARTIME_ALLIES,
        0x3600,
        &[3, 0, 2],
    );
    links_vector(
        &mut bytes,
        DIPLOMACY[0] + cdiplomacy_status::ENEMY_COUNTRIES,
        0x3700,
        &[2],
    );
    bytes[status(0, 2) + CO_BELLIGERENT] = 1;
    bytes[status(0, 3) + CO_BELLIGERENT] = 1;
    let memory = memory(&bytes);
    let countries = ccountry::Reader::new(&memory, BASE, GS).unwrap();
    let mut slots = [0_u32; 4];
    assert_eq!(
        co_belligerents(&memory, BASE, &countries, country(0), &mut slots),
        Ok((2, 2))
    );
    assert_eq!(slots[..2], [2, 3]);
    // A byte left zero in either vector is not inferred from membership.
    bytes[status(0, 3) + CO_BELLIGERENT] = 0;
    let memory = self::memory(&bytes);
    let countries = ccountry::Reader::new(&memory, BASE, GS).unwrap();
    assert_eq!(
        co_belligerents(&memory, BASE, &countries, country(0), &mut slots),
        Ok((1, 1))
    );
    // Without storage the demand is an upper bound: repeats are not deduplicated.
    assert_eq!(
        co_belligerents(&memory, BASE, &countries, country(0), &mut []),
        Ok((0, 2))
    );
    p64(&mut bytes, status(0, 2) + DIPLOMACY_STATUS, 0);
    let memory = self::memory(&bytes);
    let countries = ccountry::Reader::new(&memory, BASE, GS).unwrap();
    assert_eq!(
        co_belligerents(&memory, BASE, &countries, country(0), &mut slots),
        Err("relation status identity mismatch")
    );
}

/// A named way to corrupt one synthetic byte span before a walk.
type Case<'a> = (&'a str, &'a dyn Fn(&mut Vec<u8>));

/// An observed walk without storage, one that records the baseline and
/// one that replays it (no writes observed) must agree; returns the first.
fn run(bytes: &[u8]) -> (Walk, Vec<OpinionRow>, Vec<TupleRow>) {
    let (mut resolver, mut baseline) = (Resolver::default(), Baseline::default());
    let passes: Vec<_> = (0..3)
        .map(|_| {
            let mut opinions = [OpinionRow::default(); 16];
            let mut tuples = [TupleRow::default(); 16];
            let result = walk_with(
                &memory(bytes),
                BASE,
                GS,
                &mut opinions,
                &mut tuples,
                [true; 2],
                &mut resolver,
                &mut baseline,
                Some(Observation {
                    token: (1, 1),
                    marks: Some(&[]),
                }),
                |_| true,
            );
            resolver.grow();
            baseline.grow();
            (
                result,
                opinions[..result.opinions.len()].to_vec(),
                tuples[..result.tuples.len()].to_vec(),
            )
        })
        .collect();
    assert_eq!(passes[0], passes[1]);
    assert_eq!(passes[0], passes[2]);
    passes[0].clone()
}

#[test]
fn links_come_from_owned_relations_without_walking_the_matrix() {
    let mut bytes = fixture();
    // An attache of country 1 in country 3: 3's status toward 1 holds +0x210.
    relation(
        &mut bytes,
        RELATION[0],
        cattache_relation::VTABLE_RVA,
        cattache_relation::TYPE,
        1,
        3,
    );
    // An unrelated guarantee in the same vectors is skipped.
    relation(
        &mut bytes,
        RELATION[1],
        cguarantee_relation::VTABLE_RVA,
        cguarantee_relation::TYPE,
        1,
        2,
    );
    for owner in [0, 2] {
        array(
            &mut bytes,
            DIPLOMACY[owner] + cdiplomacy_status::OWNED_RELATIONS,
            0x3400 + owner * 0x40,
            &[RELATION[0] as u64, RELATION[1] as u64],
        );
    }
    p64(&mut bytes, status(2, 1) + ATTACHE_LINK, RELATION[0] as u64);
    // A slot set in a status no owned relation names is never visited.
    p64(&mut bytes, status(1, 3) + AIR_BASE_ACCESS, 1);
    let mut rows = [LinkRow::default(); 4];
    let mut resolver = Resolver::default();
    let outcome = link_rows(&bytes, &mut rows, &mut resolver);
    assert_eq!(outcome, Outcome::Rows { len: 1 });
    assert_eq!(
        rows[0],
        LinkRow {
            country: 3,
            target: 1,
            links: LINK_ATTACHE,
        }
    );
    assert_eq!(
        link_rows(&bytes, &mut [], &mut resolver),
        Outcome::Truncated {
            len: 0,
            unread_rows: 1
        }
    );
    p64(&mut bytes, status(2, 1), 7);
    assert!(matches!(
        link_rows(&bytes, &mut rows, &mut resolver),
        Outcome::Unavailable { .. }
    ));
}

/// Two relations naming one pair give one row carrying both bits, and the
/// walk reuses the caller's resolver storage without reallocating it.
#[test]
fn a_pair_named_twice_is_one_row_and_the_walk_reuses_its_scratch() {
    let mut bytes = fixture();
    relation(
        &mut bytes,
        RELATION[0],
        cattache_relation::VTABLE_RVA,
        cattache_relation::TYPE,
        3,
        1,
    );
    relation(
        &mut bytes,
        RELATION[1],
        cair_base_access_relation::VTABLE_RVA,
        cair_base_access_relation::TYPE,
        1,
        3,
    );
    array(
        &mut bytes,
        DIPLOMACY[0] + cdiplomacy_status::OWNED_RELATIONS,
        0x3400,
        &[RELATION[0] as u64, RELATION[1] as u64],
    );
    p64(&mut bytes, status(0, 3) + ATTACHE_LINK, RELATION[0] as u64);
    p64(
        &mut bytes,
        status(0, 3) + AIR_BASE_ACCESS,
        RELATION[1] as u64,
    );
    let mut resolver = Resolver::with_capacity(8);
    let storage = (resolver.references.as_ptr(), resolver.capacity());
    let mut rows = [LinkRow::default(); 4];
    for _ in 0..2 {
        assert_eq!(
            link_rows(&bytes, &mut rows, &mut resolver),
            Outcome::Rows { len: 1 }
        );
        assert_eq!(
            rows[0],
            LinkRow {
                country: 1,
                target: 3,
                links: LINK_ATTACHE | LINK_AIR_BASE_ACCESS,
            }
        );
        assert_eq!((resolver.references.as_ptr(), resolver.capacity()), storage);
    }
    // A full output counts the pair once per naming relation: an upper bound.
    assert_eq!(
        link_rows(&bytes, &mut [], &mut resolver),
        Outcome::Truncated {
            len: 0,
            unread_rows: 2
        }
    );
}

fn link_rows(bytes: &[u8], rows: &mut [LinkRow], resolver: &mut Resolver) -> Outcome {
    let (mut wars, mut countries) = ([WarRow::default(); 4], [0_u32; 8]);
    links(
        &memory(bytes),
        BASE,
        GS,
        rows,
        WarOutput {
            rows: &mut wars,
            countries: &mut countries,
        },
        resolver,
    )
    .links
}

fn links_vector(bytes: &mut [u8], at: usize, data: usize, values: &[u32]) {
    p64(bytes, at, data as u64);
    p32(bytes, at + 8, values.len() as u32);
    p32(bytes, at + 12, values.len() as u32);
    for (index, value) in values.iter().enumerate() {
        p32(bytes, data + index * 4, *value);
    }
}

/// Countries 1 and 2 fight country 3; country 1 gave 2 military access.
fn war_fixture() -> Vec<u8> {
    let mut bytes = fixture();
    relation(
        &mut bytes,
        RELATION[0],
        cmilitary_access_relation::VTABLE_RVA,
        cmilitary_access_relation::TYPE,
        1,
        2,
    );
    for owner in [0, 1] {
        array(
            &mut bytes,
            DIPLOMACY[owner] + cdiplomacy_status::OWNED_RELATIONS,
            0x3400 + owner * 0x40,
            &[RELATION[0] as u64],
        );
    }
    p64(
        &mut bytes,
        status(0, 2) + MILITARY_ACCESS,
        RELATION[0] as u64,
    );
    links_vector(
        &mut bytes,
        DIPLOMACY[0] + cdiplomacy_status::ENEMY_COUNTRIES,
        0x3600,
        &[3],
    );
    links_vector(
        &mut bytes,
        DIPLOMACY[0] + cdiplomacy_status::WARTIME_ALLIES,
        0x3640,
        &[2, 2],
    );
    links_vector(
        &mut bytes,
        DIPLOMACY[2] + cdiplomacy_status::ENEMY_COUNTRIES,
        0x3680,
        &[1, 2],
    );
    bytes[COUNTRIES[0] + ccountry::CAN_OCCUPY_NON_WAR] = 1;
    bytes[COUNTRIES[2] + ccountry::CAN_OCCUPY_NON_WAR] = 0x80;
    bytes
}

fn walk_wars(bytes: &[u8], capacity: (usize, usize)) -> (LinkWalk, Vec<WarRow>, Vec<u32>) {
    let mut rows = [LinkRow::default(); 4];
    let mut wars = vec![WarRow::default(); capacity.0];
    let mut countries = vec![0_u32; capacity.1];
    let walk = links(
        &memory(bytes),
        BASE,
        GS,
        &mut rows,
        WarOutput {
            rows: &mut wars,
            countries: &mut countries,
        },
        &mut Resolver::default(),
    );
    (walk, wars, countries)
}

fn row(country: u32, rule: u8) -> WarRow {
    WarRow {
        country,
        can_occupy_non_war: rule,
        ..WarRow::default()
    }
}

/// Without a civil-war link or a retired war relation the enemy vectors
/// are derived by the reader and never read; the rule byte is recorded
/// wherever it is nonzero.
#[test]
fn war_rows_record_the_rule_and_leave_enemy_sets_to_derivation() {
    let bytes = war_fixture();
    let (walk, wars, _) = walk_wars(&bytes, (4, 8));
    assert_eq!(walk.links, Outcome::Rows { len: 1 });
    assert_eq!(walk.wars, Outcome::Rows { len: 2 });
    assert_eq!((walk.countries_len, walk.required_countries), (0, 0));
    assert_eq!(wars[..2], [row(1, 1), row(3, 0x80)]);
    // The access relation's slot is the link row's military access bit.
    let mut rows = [LinkRow::default(); 2];
    assert_eq!(
        link_rows(&bytes, &mut rows, &mut Resolver::default()),
        Outcome::Rows { len: 1 }
    );
    assert_eq!(rows[0].links, LINK_MILITARY_ACCESS);
}

/// A civil-war revolter's vector is read as stored, even with a zero rule.
#[test]
fn a_civil_war_revolter_has_its_enemy_vector_read() {
    let mut bytes = war_fixture();
    p32(&mut bytes, COUNTRIES[1] + ccountry::CIVIL_WAR_LINK, 1);
    links_vector(
        &mut bytes,
        DIPLOMACY[1] + cdiplomacy_status::ENEMY_COUNTRIES,
        0x36c0,
        &[3, 9],
    );
    let (walk, wars, countries) = walk_wars(&bytes, (4, 8));
    assert_eq!(walk.wars, Outcome::Rows { len: 3 });
    assert_eq!(
        wars[1],
        WarRow {
            country: 2,
            can_occupy_non_war: 0,
            enemies_read: true,
            enemies_start: 0,
            enemies_len: 2,
        }
    );
    assert_eq!(countries[..2], [3, 9]);
    // Too few links truncates that row and reports its demand; only the
    // war-link arena is short, so the link table stays complete.
    let (walk, _, _) = walk_wars(&bytes, (4, 1));
    assert_eq!(
        walk.wars,
        Outcome::Truncated {
            len: 2,
            unread_rows: 1
        }
    );
    assert_eq!((walk.countries_len, walk.required_countries), (0, 2));
    assert_eq!(walk.links, Outcome::Rows { len: 1 });
}

/// An owned war relation whose retired byte is set could linger in a side
/// list, so its owner's vector is read.
#[test]
fn a_retired_war_relation_makes_its_owner_read_the_vector() {
    let mut bytes = war_fixture();
    relation(
        &mut bytes,
        RELATION[1],
        cwar_relation::VTABLE_RVA,
        cwar_relation::TYPE,
        3,
        1,
    );
    bytes[RELATION[1] + crelation::RETIRED] = 1;
    array(
        &mut bytes,
        DIPLOMACY[2] + cdiplomacy_status::OWNED_RELATIONS,
        0x3480,
        &[RELATION[1] as u64],
    );
    let (walk, wars, countries) = walk_wars(&bytes, (4, 8));
    assert_eq!(walk.wars, Outcome::Rows { len: 2 });
    assert_eq!(wars[0], row(1, 1));
    assert_eq!(
        wars[1],
        WarRow {
            enemies_read: true,
            enemies_len: 2,
            ..row(3, 0x80)
        }
    );
    assert_eq!(countries[..2], [1, 2]);
    // An active war relation alone changes nothing.
    bytes[RELATION[1] + crelation::RETIRED] = 0;
    let (walk, wars, _) = walk_wars(&bytes, (4, 8));
    assert_eq!(walk.countries_len, 0);
    assert_eq!(wars[1], row(3, 0x80));
}

#[test]
fn access_and_docking_slots_come_from_their_owned_relations() {
    let mut bytes = fixture();
    // Docking rights of 1 at 3: 1's status toward 3 holds +0x2B0. Military access offered
    // by 1 to 3: 3's status toward 1 holds +0x2A0.
    relation(
        &mut bytes,
        RELATION[0],
        cdocking_rights_relation::VTABLE_RVA,
        cdocking_rights_relation::TYPE,
        1,
        3,
    );
    relation(
        &mut bytes,
        RELATION[1],
        coffer_military_access_relation::VTABLE_RVA,
        coffer_military_access_relation::TYPE,
        1,
        3,
    );
    for owner in [0, 2] {
        array(
            &mut bytes,
            DIPLOMACY[owner] + cdiplomacy_status::OWNED_RELATIONS,
            0x3400 + owner * 0x40,
            &[RELATION[0] as u64, RELATION[1] as u64],
        );
    }
    p64(
        &mut bytes,
        status(0, 3) + DOCKING_RIGHTS,
        RELATION[0] as u64,
    );
    p64(
        &mut bytes,
        status(2, 1) + MILITARY_ACCESS,
        RELATION[1] as u64,
    );
    let mut rows = [LinkRow::default(); 4];
    assert_eq!(
        link_rows(&bytes, &mut rows, &mut Resolver::default()),
        Outcome::Rows { len: 2 }
    );
    assert_eq!(
        rows[..2],
        [
            LinkRow {
                country: 1,
                target: 3,
                links: LINK_DOCKING_RIGHTS,
            },
            LinkRow {
                country: 3,
                target: 1,
                links: LINK_MILITARY_ACCESS,
            },
        ]
    );
}

#[test]
fn only_nonzero_opinions_and_serializable_relations_become_rows() {
    let mut bytes = fixture();
    p32(&mut bytes, status(0, 2) + OPINION, (-100_i32) as u32);
    p32(&mut bytes, status(1, 3) + OPINION, 60);
    // Slot zero is never serialized even with a nonzero opinion.
    p32(&mut bytes, status(2, 0) + OPINION, 7);
    relation(
        &mut bytes,
        RELATION[0],
        cmarket_access_rights_relation::VTABLE_RVA,
        cmarket_access_rights_relation::TYPE,
        1,
        3,
    );
    p32(&mut bytes, RELATION[0] + crelation::START_HOUR, 100);
    p32(
        &mut bytes,
        RELATION[0] + crelation::END_HOUR,
        (-5_i32) as u32,
    );
    bytes[RELATION[0] + crelation::CANCEL] = 0x80;
    bytes[RELATION[0] + crelation::RETIRED] = 0x02;
    relation(
        &mut bytes,
        RELATION[1],
        cguarantee_relation::VTABLE_RVA,
        cguarantee_relation::TYPE,
        1,
        3,
    );
    array(
        &mut bytes,
        status(0, 3) + SERIALIZABLE_RELATIONS,
        serializable_data(0, 3),
        &[RELATION[0] as u64, RELATION[1] as u64],
    );
    for owner in [0, 2] {
        own(&mut bytes, owner, &RELATION);
    }
    let (result, opinions, tuples) = run(&bytes);
    assert_eq!(
        dense::walk(
            &memory(&bytes),
            BASE,
            GS,
            &mut [OpinionRow::default(); 16],
            &mut [TupleRow::default(); 16],
            &mut Resolver::default(),
        ),
        result
    );
    assert_eq!(result.opinions, Outcome::Rows { len: 2 });
    assert_eq!(result.tuples, Outcome::Rows { len: 2 });
    assert_eq!(
        opinions,
        vec![
            OpinionRow {
                owner_country: 1,
                target_country: 2,
                opinion: -100,
            },
            OpinionRow {
                owner_country: 2,
                target_country: 3,
                opinion: 60,
            },
        ]
    );
    assert_eq!(
        tuples[0],
        TupleRow {
            owner_country: 1,
            target_country: 3,
            type_id: cmarket_access_rights_relation::TYPE,
            ordinal: 0,
            first_country: 1,
            second_country: 3,
            start_hour: 100,
            end_hour: -5,
            cancel: 0x80,
            retired: 0x02,
        }
    );
    assert_eq!(tuples[1].ordinal, 1);
    assert_eq!(tuples[1].type_id, cguarantee_relation::TYPE);
}

#[test]
fn war_and_subject_elements_are_skipped_but_still_consume_their_ordinal() {
    let mut bytes = fixture();
    // Native vector order: war, guarantee, subject, market access.
    relation(
        &mut bytes,
        RELATION[0],
        cwar_relation::VTABLE_RVA,
        cwar_relation::TYPE,
        1,
        3,
    );
    relation(
        &mut bytes,
        RELATION[1],
        cguarantee_relation::VTABLE_RVA,
        cguarantee_relation::TYPE,
        1,
        3,
    );
    let subject = 0x1d000;
    let market = 0x1d800;
    relation(
        &mut bytes,
        subject,
        csubject_relation::VTABLE_RVA,
        csubject_relation::TYPE_VALUE,
        1,
        3,
    );
    relation(
        &mut bytes,
        market,
        cmarket_access_rights_relation::VTABLE_RVA,
        cmarket_access_rights_relation::TYPE,
        1,
        3,
    );
    array(
        &mut bytes,
        status(0, 3) + SERIALIZABLE_RELATIONS,
        serializable_data(0, 3),
        &[
            RELATION[0] as u64,
            RELATION[1] as u64,
            subject as u64,
            market as u64,
        ],
    );
    for owner in [0, 2] {
        own(
            &mut bytes,
            owner,
            &[RELATION[0], RELATION[1], subject, market],
        );
    }
    let (result, _, tuples) = run(&bytes);
    assert_eq!(result.tuples, Outcome::Rows { len: 2 });
    assert_eq!(
        tuples
            .iter()
            .map(|row| (row.type_id, row.ordinal))
            .collect::<Vec<_>>(),
        vec![
            (cguarantee_relation::TYPE, 1),
            (cmarket_access_rights_relation::TYPE, 3),
        ]
    );
    // A skipped class is still authenticated: a contradictory type fails.
    let mut broken = bytes.clone();
    p32(&mut broken, RELATION[0] + crelation::TYPE_ID, 1);
    assert_eq!(
        run(&broken).0.tuples,
        Outcome::Unavailable {
            reason: "relation type word differs from its class"
        }
    );
}

#[test]
fn empty_matrices_are_a_complete_empty_pair_of_tables() {
    let (result, opinions, tuples) = run(&fixture());
    assert_eq!(result.opinions, Outcome::Rows { len: 0 });
    assert_eq!(result.tuples, Outcome::Rows { len: 0 });
    assert!(opinions.is_empty() && tuples.is_empty());
}

#[test]
fn capacity_truncates_each_table_independently_with_exact_missing_counts() {
    let mut bytes = fixture();
    for target in 1..4 {
        p32(&mut bytes, status(0, target) + OPINION, target as u32);
    }
    relation(
        &mut bytes,
        RELATION[0],
        cguarantee_relation::VTABLE_RVA,
        cguarantee_relation::TYPE,
        1,
        2,
    );
    array(
        &mut bytes,
        status(0, 2) + SERIALIZABLE_RELATIONS,
        serializable_data(0, 2),
        &[RELATION[0] as u64],
    );
    for owner in [0, 1] {
        own(&mut bytes, owner, &[RELATION[0]]);
    }
    let mut opinions = [OpinionRow::default(); 2];
    let result = walk(&memory(&bytes), BASE, GS, &mut opinions, &mut [], [true; 2]);
    assert_eq!(
        result.opinions,
        Outcome::Truncated {
            len: 2,
            unread_rows: 1
        }
    );
    assert_eq!(
        result.tuples,
        Outcome::Truncated {
            len: 0,
            unread_rows: 1
        }
    );
    assert_eq!(opinions[1].target_country, 2);
}

#[test]
fn changed_headers_null_slots_and_bad_identities_are_named_failures() {
    let mut valid = fixture();
    relation(
        &mut valid,
        RELATION[0],
        cguarantee_relation::VTABLE_RVA,
        cguarantee_relation::TYPE,
        1,
        3,
    );
    array(
        &mut valid,
        status(0, 3) + SERIALIZABLE_RELATIONS,
        serializable_data(0, 3),
        &[RELATION[0] as u64],
    );
    for owner in [0, 2] {
        own(&mut valid, owner, &[RELATION[0]]);
    }
    assert_eq!(run(&valid).0.tuples, Outcome::Rows { len: 1 });
    let cases: [Case<'_>; 12] = [
        ("relation status vtable mismatch", &|bytes: &mut Vec<u8>| {
            p64(bytes, status(0, 2), 0);
        }),
        ("null relation matrix entry", &|bytes: &mut Vec<u8>| {
            p64(bytes, matrix_data(0) + 2 * 8, 0);
        }),
        (
            "relation status target differs from matrix slot",
            &|bytes: &mut Vec<u8>| {
                p32(bytes, status(0, 2) + TARGET_COUNTRY, 3);
            },
        ),
        (
            "relation matrix does not cover country ordinals",
            &|bytes: &mut Vec<u8>| {
                p32(bytes, DIPLOMACY[0] + cdiplomacy_status::RELATIONS + 12, 2);
            },
        ),
        ("diplomacy vtable mismatch", &|bytes: &mut Vec<u8>| {
            p64(bytes, DIPLOMACY[1], 0);
        }),
        ("diplomacy country mismatch", &|bytes: &mut Vec<u8>| {
            p64(bytes, DIPLOMACY[1] + cdiplomacy_status::COUNTRY, 0x1234);
        }),
        ("country diplomacy is null", &|bytes: &mut Vec<u8>| {
            p64(bytes, COUNTRIES[1] + ccountry::DIPLOMACY_STATUS, 0);
        }),
        (
            "serializable relation vector invalid",
            &|bytes: &mut Vec<u8>| {
                p32(bytes, status(0, 3) + SERIALIZABLE_RELATIONS + 12, 9);
            },
        ),
        (
            "owned relation vector unreadable or invalid",
            &|bytes: &mut Vec<u8>| {
                p32(
                    bytes,
                    DIPLOMACY[2] + cdiplomacy_status::OWNED_RELATIONS + 12,
                    9,
                );
            },
        ),
        (
            "owned relation pointer null or invalid",
            &|bytes: &mut Vec<u8>| {
                p64(bytes, 0x3400 + 2 * 0x40, 0);
            },
        ),
        // The owned vector names a relation its status does not serialize.
        (
            "owned and serializable relations disagree",
            &|bytes: &mut Vec<u8>| {
                array(bytes, status(0, 3) + SERIALIZABLE_RELATIONS, 0, &[]);
            },
        ),
        // The status serializes a relation the owned vector does not name.
        (
            "owned and serializable relations disagree",
            &|bytes: &mut Vec<u8>| {
                array(
                    bytes,
                    status(0, 3) + SERIALIZABLE_RELATIONS,
                    serializable_data(0, 3),
                    &[RELATION[0] as u64, RELATION[0] as u64],
                );
            },
        ),
    ];
    let (complete, complete_opinions, _) = run(&valid);
    for (reason, break_it) in cases {
        let mut bytes = valid.clone();
        break_it(&mut bytes);
        let (result, opinions, _) = run(&bytes);
        // Owned-vector failures refuse tuples alone: opinions never read it.
        if reason.starts_with("owned ") {
            assert_eq!(
                (result.opinions, opinions),
                (complete.opinions, complete_opinions.clone()),
                "{reason}"
            );
        } else {
            assert_eq!(result.opinions, Outcome::Unavailable { reason }, "{reason}");
        }
        assert_eq!(result.tuples, Outcome::Unavailable { reason });
    }
    // The owned vector is read only for tuples.
    let mut bytes = valid.clone();
    p64(&mut bytes, 0x3400 + 2 * 0x40, 0);
    let mut opinions = [OpinionRow::default(); 4];
    let result = walk(
        &memory(&bytes),
        BASE,
        GS,
        &mut opinions,
        &mut [],
        [true, false],
    );
    assert_eq!(result.opinions, Outcome::Rows { len: 0 });
}

#[test]
fn unclassified_and_contradictory_relation_identities_are_refused() {
    let mut valid = fixture();
    relation(
        &mut valid,
        RELATION[0],
        cguarantee_relation::VTABLE_RVA,
        cguarantee_relation::TYPE,
        1,
        3,
    );
    array(
        &mut valid,
        status(0, 3) + SERIALIZABLE_RELATIONS,
        serializable_data(0, 3),
        &[RELATION[0] as u64],
    );
    for owner in [0, 2] {
        own(&mut valid, owner, &[RELATION[0]]);
    }
    assert_eq!(run(&valid).0.tuples, Outcome::Rows { len: 1 });
    let cases: [Case<'_>; 4] = [
        (
            "serializable relation has an unclassified vtable",
            &|bytes: &mut Vec<u8>| p64(bytes, RELATION[0], 0x1234),
        ),
        (
            "relation type word differs from its class",
            &|bytes: &mut Vec<u8>| p32(bytes, RELATION[0] + crelation::TYPE_ID, 1),
        ),
        // A second relation in the same status whose first endpoint is
        // another country.
        (
            "serializable relation is not owned by its first endpoint",
            &|bytes: &mut Vec<u8>| {
                relation(
                    bytes,
                    RELATION[1],
                    cguarantee_relation::VTABLE_RVA,
                    cguarantee_relation::TYPE,
                    2,
                    3,
                );
                array(
                    bytes,
                    status(0, 3) + SERIALIZABLE_RELATIONS,
                    serializable_data(0, 3),
                    &[RELATION[0] as u64, RELATION[1] as u64],
                );
            },
        ),
        ("null serializable relation entry", &|bytes: &mut Vec<
            u8,
        >| {
            p64(bytes, serializable_data(0, 3), 0)
        }),
    ];
    for (reason, break_it) in cases {
        let mut bytes = valid.clone();
        break_it(&mut bytes);
        assert_eq!(
            run(&bytes).0.tuples,
            Outcome::Unavailable { reason },
            "{reason}"
        );
    }
}

#[test]
fn a_truncated_address_space_never_yields_a_partial_success() {
    let bytes = fixture();
    for cut in [status(2, 3) + OPINION, matrix_data(2) + 8] {
        let (result, _, _) = run(&bytes[..cut]);
        assert!(
            matches!(result.opinions, Outcome::Unavailable { .. }),
            "{cut:#x} {result:?}"
        );
        assert!(matches!(result.tuples, Outcome::Unavailable { .. }));
    }
}

/// One observed opinion walk; returns the outcome, its rows and the bytes
/// it read.
fn observed(
    bytes: &[u8],
    baseline: &mut Baseline,
    token: (u64, u64),
    marks: &[Mark],
) -> (Outcome, Vec<OpinionRow>, usize) {
    observed_slice(bytes, baseline, token, marks, &|_| true)
}

/// `observed` for the owners `select` names.
fn observed_slice(
    bytes: &[u8],
    baseline: &mut Baseline,
    token: (u64, u64),
    marks: &[Mark],
    select: &dyn Fn(u32) -> bool,
) -> (Outcome, Vec<OpinionRow>, usize) {
    let slice = memory(bytes);
    let memory = Counting::new(&slice);
    let mut opinions = [OpinionRow::default(); 8];
    let result = walk_with(
        &memory,
        BASE,
        GS,
        &mut opinions,
        &mut [],
        [true, false],
        &mut Resolver::default(),
        baseline,
        Some(Observation {
            token,
            marks: Some(marks),
        }),
        select,
    );
    baseline.grow();
    (
        result.opinions,
        opinions[..result.opinions.len()].to_vec(),
        memory.take()[1],
    )
}

/// A walk that observed every opinion write since the baseline replays
/// it, reading only the marked statuses; a new observation, a refused
/// walk or a mark that is not a recorded status reads every status.
#[test]
fn observed_walks_replay_the_baseline_and_read_only_marked_statuses() {
    let mut bytes = fixture();
    p32(&mut bytes, status(1, 2) + OPINION, 5);
    let mut baseline = Baseline::default();
    let token = (1, 1);
    let row = |owner_country, target_country, opinion| OpinionRow {
        owner_country,
        target_country,
        opinion,
    };
    let mark = |owner: usize, target: usize| Mark {
        status: status(owner, target),
        diplomacy: DIPLOMACY[owner],
    };
    // Unrecorded (no storage yet), sized (the owners' rows get storage;
    // the two owners without rows already fit and are recorded), recording
    // the third, then replaying all three.
    let cold = observed(&bytes, &mut baseline, token, &[]);
    let sized = observed(&bytes, &mut baseline, token, &[]);
    let recording = observed(&bytes, &mut baseline, token, &[]);
    let reused = baseline.counts().reused_owners;
    let replay = observed(&bytes, &mut baseline, token, &[]);
    assert_eq!((cold.0, &cold.1), (sized.0, &sized.1));
    assert_eq!((cold.0, &cold.1), (recording.0, &recording.1));
    assert_eq!((replay.0, &replay.1), (cold.0, &cold.1));
    assert_eq!(cold.1, [row(2, 2, 5)]);
    assert_eq!(baseline.valid_owners(), 3);
    // No pointer page and no status: against the cold walk, which reads
    // every owner in full, nine statuses of 16 bytes and their nine
    // pointers are not read.
    assert!(replay.2 + 9 * (16 + 8) <= cold.2, "{replay:?} {cold:?}");
    assert!(replay.2 < recording.2, "{replay:?} {recording:?}");
    assert_eq!(baseline.counts().reused_owners - reused, 3);
    // Unmarked statuses are not read: an unobserved write stays unseen,
    // exactly what a missed writer would do (verify mode exists for it).
    let mut unobserved = bytes.clone();
    p32(&mut unobserved, status(0, 3) + OPINION, 9);
    p64(&mut unobserved, status(2, 1), 0);
    assert_eq!(observed(&unobserved, &mut baseline, token, &[]).1, cold.1);
    // A marked status is re-read: inserted, changed and removed rows.
    let mut written = bytes.clone();
    p32(&mut written, status(0, 3) + OPINION, 9);
    p32(&mut written, status(2, 1) + OPINION, (-4_i32) as u32);
    let marks = [mark(2, 1), mark(0, 3), mark(2, 1)];
    let (outcome, rows, _) = observed(&written, &mut baseline, token, &marks);
    assert_eq!(outcome, Outcome::Rows { len: 3 });
    assert_eq!(rows, [row(1, 3, 9), row(2, 2, 5), row(3, 1, -4)]);
    p32(&mut written, status(1, 2) + OPINION, 0);
    let (_, rows, _) = observed(&written, &mut baseline, token, &[mark(1, 2)]);
    assert_eq!(rows, [row(1, 3, 9), row(3, 1, -4)]);
    assert_eq!(baseline.counts().marks, 4);
    // An owner whose matrix moved is read in full, unmarked writes included.
    let mut moved = written.clone();
    let copy = 0x3300;
    let entries: Vec<u64> = (0..4).map(|t| status(1, t) as u64).collect();
    array(
        &mut moved,
        DIPLOMACY[1] + cdiplomacy_status::RELATIONS,
        copy,
        &entries,
    );
    p32(&mut moved, status(1, 1) + OPINION, 6);
    let (_, rows, _) = observed(&moved, &mut baseline, token, &[]);
    assert_eq!(rows, [row(1, 3, 9), row(2, 1, 6), row(3, 1, -4)]);
    // A mark that is not the matrix's status at its slot refuses the
    // observation: every status is read, so the vtable fault is seen.
    let mut stray = moved.clone();
    let fake = 0x20000;
    stray[fake..fake + 0x400].copy_from_slice(&moved[status(0, 2)..status(0, 2) + 0x400]);
    p64(&mut stray, status(2, 3), 0);
    let refused = observed(
        &stray,
        &mut baseline,
        token,
        &[Mark {
            status: fake,
            diplomacy: DIPLOMACY[0],
        }],
    );
    assert_eq!(
        refused.0,
        Outcome::Unavailable {
            reason: "relation status vtable mismatch"
        }
    );
    assert_eq!(baseline.counts().refused_observations, 1);
    // The refused walk forgot every owner: with the fault gone, the next
    // walk reads everything (and so sees the unmarked write).
    p32(&mut moved, status(2, 3) + OPINION, 2);
    let (_, rows, read) = observed(&moved, &mut baseline, token, &[]);
    assert_eq!(
        rows,
        [row(1, 3, 9), row(2, 1, 6), row(3, 1, -4), row(3, 3, 2)]
    );
    assert_eq!(read, cold.2);
    // A new observation (a load, gap or unobserved interval) reads everything.
    p32(&mut moved, status(0, 1) + OPINION, 1);
    assert_eq!(observed(&moved, &mut baseline, token, &[]).1.len(), 4);
    assert_eq!(observed(&moved, &mut baseline, (1, 2), &[]).1.len(), 5);
    // A walk that panics midway leaves no baseline behind: the next walk
    // reads every status, the unmarked write included.
    struct Panicking<'a>(SliceMemory<'a>);
    impl Memory for Panicking<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            // The third owner's diplomacy: two owners are visited by then.
            let third = DIPLOMACY[2]..DIPLOMACY[2] + 0x100;
            assert!(!third.contains(&address), "fault injected mid-walk");
            self.0.copy(address, output)
        }
    }
    observed(&moved, &mut baseline, (1, 2), &[]);
    assert!(baseline.token().is_some());
    let fresh = baseline.counts().full_walks;
    let replays = baseline.counts().incremental_walks;
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        walk_with(
            &Panicking(memory(&moved)),
            BASE,
            GS,
            &mut [OpinionRow::default(); 8],
            &mut [],
            [true, false],
            &mut Resolver::default(),
            &mut baseline,
            Some(Observation {
                token: (1, 2),
                marks: Some(&[]),
            }),
            |_| true,
        )
    }));
    assert!(panicked.is_err());
    assert_eq!(baseline.counts().incremental_walks, replays + 1);
    assert_eq!(baseline.token(), None);
    p32(&mut moved, status(2, 2) + OPINION, 8);
    let (_, rows, _) = observed(&moved, &mut baseline, (1, 2), &[]);
    assert!(rows.contains(&row(3, 2, 8)), "{rows:?}");
    // No observation reads every owner in full without recording.
    let _ = fresh;
    // Tuple-only walks leave the baseline alone.
    let mut tuples = [TupleRow::default(); 4];
    walk_with(
        &memory(&moved),
        BASE,
        GS,
        &mut [],
        &mut tuples,
        [false, true],
        &mut Resolver::default(),
        &mut baseline,
        None,
        |_| true,
    );
    let incremental = baseline.counts().incremental_walks;
    observed(&moved, &mut baseline, (1, 2), &[]);
    assert_eq!(baseline.counts().incremental_walks, incremental + 1);
    // Truncation is the full walk's: the replay pushes through the same
    // counters.
    let mut short = [OpinionRow::default(); 2];
    let result = walk_with(
        &memory(&moved),
        BASE,
        GS,
        &mut short,
        &mut [],
        [true, false],
        &mut Resolver::default(),
        &mut baseline,
        Some(Observation {
            token: (1, 2),
            marks: Some(&[]),
        }),
        |_| true,
    );
    assert_eq!(
        result.opinions,
        Outcome::Truncated {
            len: 2,
            unread_rows: 4
        }
    );
    assert_eq!(baseline.counts().incremental_walks, incremental + 2);
}

/// Under slices, an unselected owner keeps its rows and its marks until
/// its own slice; each observed slice walk equals an unobserved walk of
/// the same slice. An owner whose marks overflow is read in full at its
/// next slice.
#[test]
fn sliced_observed_walks_keep_other_owners_marks_for_their_slice() {
    let mut bytes = fixture();
    p32(&mut bytes, status(1, 2) + OPINION, 5);
    let mut baseline = Baseline::default();
    let token = (1, 1);
    let mark = |owner: usize, target: usize| Mark {
        status: status(owner, target),
        diplomacy: DIPLOMACY[owner],
    };
    for _ in 0..3 {
        observed(&bytes, &mut baseline, token, &[]);
    }
    assert_eq!(baseline.valid_owners(), 3);
    // Owner IDs are 1, 2, 3 (fixture owners 0, 1, 2).
    let only = |id: u32| move |owner: u32| owner == id;
    let unobserved = |bytes: &[u8], id: u32| {
        observed_slice(bytes, &mut Baseline::default(), token, &[], &only(id)).1
    };
    p32(&mut bytes, status(0, 3) + OPINION, 9);
    p32(&mut bytes, status(2, 1) + OPINION, (-4_i32) as u32);
    // Owner 1's slice: its mark applies; owner 3's waits. (Each owner's
    // first nonzero opinion has no row storage yet, so it is read in full
    // at its next slice.)
    let (_, rows, _) = observed_slice(
        &bytes,
        &mut baseline,
        token,
        &[mark(0, 3), mark(2, 1)],
        &only(1),
    );
    assert_eq!(rows, unobserved(&bytes, 1));
    assert_eq!(baseline.counts().marks, 1);
    // Owner 3's slice later: its pending mark applies without a new one.
    let (_, rows, _) = observed_slice(&bytes, &mut baseline, token, &[], &only(3));
    assert_eq!(rows, unobserved(&bytes, 3));
    assert!(rows.iter().any(|row| row.opinion == -4), "{rows:?}");
    assert_eq!(baseline.counts().marks, 2);
    // Marks beyond an owner's pending storage (the minimum and its
    // quarter headroom) make it invalid once: its next slice reads it in
    // full, unmarked writes included.
    let invalidated = baseline.counts().invalidated_owners;
    let flood = vec![mark(1, 3); Baseline::MIN_PENDING * 2];
    observed_slice(&bytes, &mut baseline, token, &flood, &only(1));
    assert_eq!(baseline.counts().invalidated_owners, invalidated + 1);
    let before = baseline.counts().read_owners;
    p32(&mut bytes, status(1, 1) + OPINION, 7);
    let (_, rows, _) = observed_slice(&bytes, &mut baseline, token, &[], &only(2));
    assert_eq!(rows, unobserved(&bytes, 2));
    assert_eq!(baseline.counts().read_owners, before + 1);
}

/// `take` and `updates` grow separately: when only `take` grew, marks
/// beyond `updates` read their owners in full instead of overrunning it.
#[test]
fn marks_beyond_either_buffer_read_their_owners_in_full() {
    let mut bytes = fixture();
    p32(&mut bytes, status(1, 2) + OPINION, 5);
    let mut baseline = Baseline::default();
    let token = (1, 1);
    for _ in 0..3 {
        observed(&bytes, &mut baseline, token, &[]);
    }
    assert_eq!(baseline.valid_owners(), 3);
    baseline.shorten_updates(0);
    p32(&mut bytes, status(0, 3) + OPINION, 9);
    let mark = Mark {
        status: status(0, 3),
        diplomacy: DIPLOMACY[0],
    };
    let invalidated = baseline.counts().invalidated_owners;
    let (outcome, rows, _) = observed(&bytes, &mut baseline, token, &[mark]);
    let (reference_outcome, reference, _) = observed(&bytes, &mut Baseline::default(), token, &[]);
    assert_eq!((outcome, rows), (reference_outcome, reference));
    assert_eq!(baseline.counts().invalidated_owners, invalidated + 3);
}

/// Only relations the owner is the first endpoint of name its statuses;
/// one it receives, or with a null second endpoint, names none.
#[test]
fn received_relations_and_null_endpoints_name_no_status() {
    let mut bytes = fixture();
    relation(
        &mut bytes,
        RELATION[0],
        cguarantee_relation::VTABLE_RVA,
        cguarantee_relation::TYPE,
        2,
        1,
    );
    relation(
        &mut bytes,
        RELATION[1],
        cguarantee_relation::VTABLE_RVA,
        cguarantee_relation::TYPE,
        1,
        0,
    );
    array(
        &mut bytes,
        status(1, 1) + SERIALIZABLE_RELATIONS,
        serializable_data(1, 1),
        &[RELATION[0] as u64],
    );
    own(&mut bytes, 0, &RELATION);
    own(&mut bytes, 1, &[RELATION[0]]);
    let slice = memory(&bytes);
    let counting = Counting::new(&slice);
    let mut tuples = [TupleRow::default(); 4];
    let result = walk(&counting, BASE, GS, &mut [], &mut tuples, [false, true]);
    assert_eq!(result.tuples, Outcome::Rows { len: 1 });
    assert_eq!((tuples[0].owner_country, tuples[0].target_country), (2, 1));
    // A tuple-only pass reads the one status it needs, not the matrix.
    let mut opinions = [OpinionRow::default(); 4];
    let both = Counting::new(&slice);
    walk(&both, BASE, GS, &mut opinions, &mut tuples, [true; 2]);
    assert!(counting.take()[2] < both.take()[2]);
}

/// The walk this pass replaced: every status of every matrix, four spans
/// each. Kept as the reference its rows must equal.
mod dense {
    use super::super::*;

    const SLOT_FIELDS: [RecordField; 4] = [
        field(0, 0, 8),
        field(TARGET_COUNTRY, 8, 4),
        field(OPINION, 12, 4),
        field(SERIALIZABLE_RELATIONS, 16, 16),
    ];
    const SLOT_BYTES: usize = 32;
    const SLOT_LAYOUT: RecordLayout<'static> =
        RecordLayout::new(&SLOT_FIELDS, SLOT_BYTES).expect("valid record layout");

    pub fn walk(
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        opinions: &mut [OpinionRow],
        tuples: &mut [TupleRow],
        resolved: &mut Resolver,
    ) -> Walk {
        let result = (|| {
            let countries = ccountry::Reader::new(memory, base, game_state)?;
            let status_vtable = (base + VTABLE_RVA) as u64;
            let mut highest_slot = 0;
            for slot in 1..countries.count() {
                if countries.at(slot)?.is_some() {
                    highest_slot = slot;
                }
            }
            let mut counters = Counters::default();
            let mut pointers = [0_usize; BATCH];
            let mut records = [0_u8; BATCH * SLOT_BYTES];
            resolved.begin(&countries);
            for slot in 1..countries.count() {
                let Some(country) = countries.at(slot)? else {
                    continue;
                };
                let diplomacy = diplomacy_root(memory, base, country)?;
                let matrix = Array::read(memory, diplomacy + cdiplomacy_status::RELATIONS, 8)
                    .ok_or("relation matrix unreadable or invalid")?;
                if matrix.count <= highest_slot {
                    return Err("relation matrix does not cover country ordinals");
                }
                let mut index = 1;
                while index < matrix.count {
                    let len = (matrix.count - index).min(BATCH as u32) as usize;
                    read_pointers(memory, matrix, index, &mut pointers[..len])?;
                    let copied = memory.copy_records(
                        RecordSources::Pointers(&pointers[..len]),
                        &SLOT_LAYOUT,
                        &mut records[..len * SLOT_BYTES],
                    );
                    for offset in 0..len {
                        if copied < (offset + 1) * SLOT_FIELDS.len() {
                            return Err("relation matrix slot fields unreadable");
                        }
                        let record = &records[offset * SLOT_BYTES..(offset + 1) * SLOT_BYTES];
                        if u64::from_le_bytes(record[..8].try_into().unwrap()) != status_vtable {
                            return Err("relation status vtable mismatch");
                        }
                        let target_country = u32::from_le_bytes(record[8..12].try_into().unwrap());
                        if resolved.resolve(&countries, target_country)?.canonical_slot
                            != index + offset as u32
                        {
                            return Err("relation status target differs from matrix slot");
                        }
                        let opinion = i32::from_le_bytes(record[12..16].try_into().unwrap());
                        if opinion != 0 {
                            counters.push_opinion(
                                opinions,
                                OpinionRow {
                                    owner_country: country.id,
                                    target_country,
                                    opinion,
                                },
                            )?;
                        }
                        let serializable = Array::from_header(record[16..].try_into().unwrap(), 8)
                            .ok_or("serializable relation vector invalid")?;
                        read_tuples(
                            memory,
                            base,
                            &countries,
                            country,
                            target_country,
                            serializable,
                            tuples,
                            &mut counters,
                            resolved,
                        )?;
                    }
                    index += len as u32;
                }
            }
            Ok(counters.walk())
        })();
        result.unwrap_or_else(|reason| Walk {
            opinions: Outcome::Unavailable { reason },
            tuples: Outcome::Unavailable { reason },
        })
    }
}

/// A world of `countries` countries computed on demand, so 5,000 of them
/// (25 million matrix statuses) need no fixture bytes. Every country is
/// the first endpoint of three relations with its next neighbours (plus a
/// second relation in the same status for every fifth country), cycling
/// through serialized, war and subject classes; relations are owned by both
/// endpoints. Some countries have an alias id that relations, targets and
/// first endpoints use, and about one pair in 37 has an opinion.
mod world {
    use super::super::*;
    use crate::{
        cgame_state, cguarantee_relation, cmarket_access_rights_relation,
        cmilitary_access_relation, cnon_aggression_pact_relation,
    };

    pub const BASE: usize = 0;
    pub const STATE: usize = 0x1000;
    const TABLE: usize = 0x10_0000;
    const LINKS: usize = 0x20_0000;
    const OBJECTS: usize = 0x100_0000;
    const OBJECT: usize = 0x1000;
    const DIPLOMACIES: usize = 0x10_0000_0000;
    const DIPLOMACY: usize = 0x400;
    const MATRICES: usize = 0x20_0000_0000;
    const OWNED: usize = 0x30_0000_0000;
    const OWNED_STRIDE: usize = 0x100;
    const RELATIONS: usize = 0x40_0000_0000;
    const RELATION: usize = 0x80;
    const SERIALIZED: usize = 0x50_0000_0000;
    const STATUSES: usize = 0x100_0000_0000;
    const NEIGHBOURS: usize = 3;
    const CLASSES: [(usize, u32); 6] = [
        (cguarantee_relation::VTABLE_RVA, cguarantee_relation::TYPE),
        (
            cnon_aggression_pact_relation::VTABLE_RVA,
            cnon_aggression_pact_relation::TYPE,
        ),
        (cwar_relation::VTABLE_RVA, cwar_relation::TYPE),
        (
            cmarket_access_rights_relation::VTABLE_RVA,
            cmarket_access_rights_relation::TYPE,
        ),
        (csubject_relation::VTABLE_RVA, csubject_relation::TYPE_VALUE),
        (
            cmilitary_access_relation::VTABLE_RVA,
            cmilitary_access_relation::TYPE,
        ),
    ];

    pub struct World {
        countries: usize,
        /// Opinion writes happen once a day (`next_day`).
        day: std::cell::Cell<u32>,
    }

    impl World {
        pub fn new(countries: usize) -> Self {
            assert!(countries > NEIGHBOURS + 1);
            Self {
                countries,
                day: std::cell::Cell::new(0),
            }
        }

        /// About one status in 211 changes its opinion by three, which
        /// inserts, changes and removes rows.
        fn written(owner: usize, target: usize) -> bool {
            (owner * 131 + target * 7).is_multiple_of(211)
        }

        pub fn next_day(&self) {
            self.day.set(self.day.get() + 1);
        }

        /// Every status a writer changed today, as a hook would mark it:
        /// with the owning diplomacy the detour reads at +0x1F0.
        pub fn marks(&self) -> Vec<Mark> {
            let mut marks = Vec::new();
            for owner in 1..self.slots() {
                for target in 1..self.slots() {
                    if Self::written(owner, target) {
                        marks.push(Mark {
                            status: STATUSES + (owner * self.slots() + target) * OBJECT_SIZE,
                            diplomacy: DIPLOMACIES + owner * DIPLOMACY,
                        });
                    }
                }
            }
            marks
        }

        /// Matrix slots, including the reserved slot zero.
        fn slots(&self) -> usize {
            self.countries + 1
        }

        fn aliased(&self, country: usize) -> bool {
            country.is_multiple_of(11)
        }

        fn alias(&self, country: usize) -> usize {
            self.slots() + country / 11 - 1
        }

        fn links(&self) -> usize {
            self.slots() + self.countries / 11
        }

        fn neighbour(&self, country: usize, step: usize) -> usize {
            (country - 1 + step) % self.countries + 1
        }

        /// Relation ids: `(first - 1) * NEIGHBOURS + step - 1` for the
        /// neighbour relations, then one extra per fifth country.
        fn extra(&self, first: usize) -> Option<usize> {
            first
                .is_multiple_of(5)
                .then(|| self.countries * NEIGHBOURS + first / 5 - 1)
        }

        /// `(first, second, class)` of a relation.
        fn relation(&self, id: usize) -> (usize, usize, usize) {
            if id < self.countries * NEIGHBOURS {
                let first = id / NEIGHBOURS + 1;
                (first, self.neighbour(first, id % NEIGHBOURS + 1), id % 6)
            } else {
                let first = (id - self.countries * NEIGHBOURS + 1) * 5;
                (first, self.neighbour(first, 1), 3)
            }
        }

        fn serialized(&self, owner: usize, target: usize, output: &mut [usize; 2]) -> usize {
            let step = (target + self.countries - owner) % self.countries;
            if owner == 0 || target == 0 || step == 0 || step > NEIGHBOURS {
                return 0;
            }
            output[0] = (owner - 1) * NEIGHBOURS + step - 1;
            match self.extra(owner).filter(|_| step == 1) {
                Some(extra) => {
                    output[1] = extra;
                    2
                }
                None => 1,
            }
        }

        /// Relations naming `country`: the ones it receives, then its own.
        fn owned(&self, country: usize, output: &mut [usize; 16]) -> usize {
            let mut len = 0;
            for step in 1..=NEIGHBOURS {
                let first = (country - 1 + self.countries - step) % self.countries + 1;
                output[len] = (first - 1) * NEIGHBOURS + step - 1;
                len += 1;
                if step == 1
                    && let Some(extra) = self.extra(first)
                {
                    output[len] = extra;
                    len += 1;
                }
            }
            for step in 1..=NEIGHBOURS {
                output[len] = (country - 1) * NEIGHBOURS + step - 1;
                len += 1;
            }
            if let Some(extra) = self.extra(country) {
                output[len] = extra;
                len += 1;
            }
            len
        }

        fn opinion(&self, owner: usize, target: usize) -> i32 {
            let base = if (owner * 7919 + target * 104_729).is_multiple_of(37) {
                ((owner + target) % 199) as i32 - 99
            } else {
                0
            };
            let written = Self::written(owner, target) && self.day.get() % 2 == 1;
            base + if written { 3 } else { 0 }
        }

        /// Writes the part of one object's fields that overlaps `output`.
        fn object(&self, address: usize, object: usize, output: &mut [u8]) {
            let offset = address - object;
            let end = offset + output.len();
            let mut field = |at: usize, value: &[u8]| {
                let (from, to) = (at.max(offset), (at + value.len()).min(end));
                if from < to {
                    output[from - offset..to - offset].copy_from_slice(&value[from - at..to - at]);
                }
            };
            if object >= STATUSES {
                let pair = (object - STATUSES) / OBJECT_SIZE;
                let (owner, target) = (pair / self.slots(), pair % self.slots());
                let hit = |at: usize, len: usize| at < end && offset < at + len;
                if hit(0, 8) {
                    field(0, &((BASE + VTABLE_RVA) as u64).to_le_bytes());
                }
                if hit(TARGET_COUNTRY, 4) {
                    let raw = if self.aliased(target) && owner.is_multiple_of(4) {
                        self.alias(target)
                    } else {
                        target
                    };
                    field(TARGET_COUNTRY, &(raw as u32).to_le_bytes());
                }
                if hit(SERIALIZABLE_RELATIONS, 16) {
                    let mut relations = [0; 2];
                    let count = self.serialized(owner, target, &mut relations) as u32;
                    if count > 0 {
                        let data = (SERIALIZED + pair * 16) as u64;
                        field(SERIALIZABLE_RELATIONS, &data.to_le_bytes());
                        field(SERIALIZABLE_RELATIONS + 8, &count.to_le_bytes());
                        field(SERIALIZABLE_RELATIONS + 12, &count.to_le_bytes());
                    }
                }
                if hit(DIPLOMACY_STATUS, 8) {
                    let diplomacy = (DIPLOMACIES + owner * DIPLOMACY) as u64;
                    field(DIPLOMACY_STATUS, &diplomacy.to_le_bytes());
                }
                if hit(OPINION, 4) {
                    field(OPINION, &self.opinion(owner, target).to_le_bytes());
                }
            } else if object >= SERIALIZED {
                let pair = (object - SERIALIZED) / 16;
                let mut relations = [0; 2];
                let count =
                    self.serialized(pair / self.slots(), pair % self.slots(), &mut relations);
                for (index, relation) in relations[..count].iter().enumerate() {
                    field(
                        index * 8,
                        &((RELATIONS + relation * RELATION) as u64).to_le_bytes(),
                    );
                }
            } else if object >= RELATIONS {
                let id = (object - RELATIONS) / RELATION;
                let (first, second, class) = self.relation(id);
                let (vtable, type_id) = CLASSES[class];
                let first = if self.aliased(first) && id.is_multiple_of(2) {
                    self.alias(first)
                } else {
                    first
                };
                field(0, &((BASE + vtable) as u64).to_le_bytes());
                field(crelation::TYPE_ID, &type_id.to_le_bytes());
                field(crelation::FIRST_COUNTRY, &(first as u32).to_le_bytes());
                field(crelation::SECOND_COUNTRY, &(second as u32).to_le_bytes());
                field(crelation::START_HOUR, &(id as i32).to_le_bytes());
                field(crelation::END_HOUR, &(-(id as i32)).to_le_bytes());
                field(crelation::CANCEL, &[(id % 3) as u8]);
                field(crelation::RETIRED, &[u8::from(id.is_multiple_of(4)) * 2]);
            } else if object >= OWNED {
                let mut relations = [0; 16];
                let count = self.owned((object - OWNED) / OWNED_STRIDE, &mut relations);
                for (index, relation) in relations[..count].iter().enumerate() {
                    field(
                        index * 8,
                        &((RELATIONS + relation * RELATION) as u64).to_le_bytes(),
                    );
                }
            } else if object >= MATRICES {
                let owner = (object - MATRICES) / (self.slots() * 8);
                let first = (offset / 8).min(self.slots());
                let last = end.div_ceil(8).min(self.slots());
                for target in first..last {
                    let status = STATUSES + (owner * self.slots() + target) * OBJECT_SIZE;
                    field(target * 8, &(status as u64).to_le_bytes());
                }
            } else if object >= DIPLOMACIES {
                let owner = (object - DIPLOMACIES) / DIPLOMACY;
                let slots = self.slots() as u32;
                let mut owned = [0; 16];
                let owned = self.owned(owner, &mut owned) as u32;
                field(
                    0,
                    &((BASE + cdiplomacy_status::VTABLE_RVA) as u64).to_le_bytes(),
                );
                let matrix = MATRICES + owner * self.slots() * 8;
                field(cdiplomacy_status::RELATIONS, &(matrix as u64).to_le_bytes());
                field(cdiplomacy_status::RELATION_CAPACITY, &slots.to_le_bytes());
                field(cdiplomacy_status::RELATION_COUNT, &slots.to_le_bytes());
                let vector = OWNED + owner * OWNED_STRIDE;
                field(
                    cdiplomacy_status::OWNED_RELATIONS,
                    &(vector as u64).to_le_bytes(),
                );
                field(
                    cdiplomacy_status::OWNED_RELATION_CAPACITY,
                    &owned.to_le_bytes(),
                );
                field(
                    cdiplomacy_status::OWNED_RELATION_COUNT,
                    &owned.to_le_bytes(),
                );
                let country = OBJECTS + owner * OBJECT;
                field(cdiplomacy_status::COUNTRY, &(country as u64).to_le_bytes());
            } else if object >= OBJECTS {
                let slot = (object - OBJECTS) / OBJECT;
                field(0, &((BASE + ccountry::VTABLE_RVA) as u64).to_le_bytes());
                field(ccountry::ID, &(slot as u32).to_le_bytes());
                let diplomacy = DIPLOMACIES + slot * DIPLOMACY;
                field(
                    ccountry::DIPLOMACY_STATUS,
                    &(diplomacy as u64).to_le_bytes(),
                );
            } else if object >= LINKS {
                let first = offset / 4;
                for raw in first..end.div_ceil(4).min(self.links()) {
                    let slot = if raw < self.slots() {
                        raw
                    } else {
                        (raw - self.slots() + 1) * 11
                    };
                    field(raw * 4, &(slot as u32).to_le_bytes());
                }
            } else if object >= TABLE {
                let first = (offset / 8).max(1);
                for slot in first..end.div_ceil(8).min(self.slots()) {
                    field(slot * 8, &((OBJECTS + slot * OBJECT) as u64).to_le_bytes());
                }
            } else {
                for (at, data, count) in [
                    (cgame_state::COUNTRIES, TABLE, self.slots()),
                    (cgame_state::COUNTRY_LINKS, LINKS, self.links()),
                ] {
                    field(at, &(data as u64).to_le_bytes());
                    field(at + 8, &(count as u32).to_le_bytes());
                    field(at + 12, &(count as u32).to_le_bytes());
                }
            }
        }
    }

    impl Memory for World {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            // The walks' hot reads: whole status fields and pointer pages.
            if address >= STATUSES {
                let (pair, offset) = (
                    (address - STATUSES) / OBJECT_SIZE,
                    (address - STATUSES) % OBJECT_SIZE,
                );
                let (owner, target) = (pair / self.slots(), pair % self.slots());
                match (offset, output.len()) {
                    (0, 8) => {
                        output.copy_from_slice(&((BASE + VTABLE_RVA) as u64).to_le_bytes());
                        return true;
                    }
                    (OPINION, 4) => {
                        output.copy_from_slice(&self.opinion(owner, target).to_le_bytes());
                        return true;
                    }
                    (TARGET_COUNTRY, 4) | (SERIALIZABLE_RELATIONS, 16) => {
                        let object = STATUSES + pair * OBJECT_SIZE;
                        output.fill(0);
                        self.object(address, object, output);
                        return true;
                    }
                    _ => {}
                }
            } else if (MATRICES..OWNED).contains(&address) && address.is_multiple_of(8) {
                let stride = self.slots() * 8;
                let owner = (address - MATRICES) / stride;
                let first = (address - MATRICES) % stride / 8;
                if output.len().is_multiple_of(8) && first + output.len() / 8 <= self.slots() {
                    for (index, chunk) in output.chunks_exact_mut(8).enumerate() {
                        let pair = owner * self.slots() + first + index;
                        let status = (STATUSES + pair * OBJECT_SIZE) as u64;
                        chunk.copy_from_slice(&status.to_le_bytes());
                    }
                    return true;
                }
            }
            output.fill(0);
            let object = if address >= STATUSES {
                STATUSES + (address - STATUSES) / OBJECT_SIZE * OBJECT_SIZE
            } else if address >= SERIALIZED {
                SERIALIZED + (address - SERIALIZED) / 16 * 16
            } else if address >= RELATIONS {
                RELATIONS + (address - RELATIONS) / RELATION * RELATION
            } else if address >= OWNED {
                OWNED + (address - OWNED) / OWNED_STRIDE * OWNED_STRIDE
            } else if address >= MATRICES {
                let stride = self.slots() * 8;
                MATRICES + (address - MATRICES) / stride * stride
            } else if address >= DIPLOMACIES {
                DIPLOMACIES + (address - DIPLOMACIES) / DIPLOMACY * DIPLOMACY
            } else if address >= OBJECTS {
                OBJECTS + (address - OBJECTS) / OBJECT * OBJECT
            } else if address >= LINKS {
                LINKS
            } else if address >= TABLE {
                TABLE
            } else if address >= STATE {
                STATE
            } else {
                return false;
            };
            self.object(address, object, output);
            true
        }
    }
}

/// Guarded calls as the live memory issues them (one per copy, scattered
/// batch and record gather), with the bytes and cache lines they touch.
struct Counting<'a, M> {
    inner: Lines<'a, M>,
    calls: std::cell::Cell<usize>,
}

struct Lines<'a, M> {
    memory: &'a M,
    bytes: std::cell::Cell<usize>,
    lines: std::cell::Cell<usize>,
}

impl<M: Memory> Memory for Lines<'_, M> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if !output.is_empty() {
            self.bytes.set(self.bytes.get() + output.len());
            let lines = (address + output.len() - 1) / 64 - address / 64 + 1;
            self.lines.set(self.lines.get() + lines);
        }
        self.memory.copy(address, output)
    }
}

impl<'a, M: Memory> Counting<'a, M> {
    fn new(memory: &'a M) -> Self {
        Self {
            inner: Lines {
                memory,
                bytes: Default::default(),
                lines: Default::default(),
            },
            calls: Default::default(),
        }
    }
    fn take(&self) -> [usize; 3] {
        [
            self.calls.replace(0),
            self.inner.bytes.replace(0),
            self.inner.lines.replace(0),
        ]
    }
}

impl<M: Memory> Memory for Counting<'_, M> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.calls.set(self.calls.get() + 1);
        self.inner.copy(address, output)
    }
    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        self.calls.set(self.calls.get() + 1);
        self.inner.copy_scattered(addresses, destinations)
    }
    fn copy_scattered_prefetched<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        self.calls.set(self.calls.get() + 1);
        self.inner.copy_scattered(addresses, destinations)
    }
    fn copy_records(
        &self,
        sources: RecordSources<'_>,
        layout: &RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        self.calls.set(self.calls.get() + 1);
        self.inner.copy_records(sources, layout, destination)
    }
}

/// Rows of the dense reference and of observed, sliced days: after a day
/// of opinion writes (about one status in 211 changes, inserting,
/// changing and removing rows) is observed, each hour's slice walk equals
/// an unobserved walk of the same slice, and the day's slices hold exactly
/// the dense rows. Prints the dense walk, a full walk after a load, and the
/// mean and worst observed hour (guarded calls, bytes, cache lines). Run
/// with `--release --nocapture` to measure.
#[test]
fn sliced_observed_days_equal_the_dense_walk_up_to_five_thousand_countries() {
    for countries in [440, 1_500, 5_000] {
        let world = world::World::new(countries);
        let memory = Counting::new(&world);
        // Bounds from the world's shape: every neighbour pair can hold an
        // opinion and at most two tuples.
        let opinions_len = countries * countries / 30 + 64;
        let tuples_len = countries * 4;
        let rows = || {
            (
                vec![OpinionRow::default(); opinions_len],
                vec![TupleRow::default(); tuples_len],
            )
        };
        let links = countries + 1 + countries / 11;
        let mut resolver = Resolver::with_capacity(links);
        let token = (1, 1);
        let walk = |marks: &[Mark],
                    baseline: &mut Baseline,
                    select: &dyn Fn(u32) -> bool,
                    resolver: &mut Resolver| {
            let mut output = rows();
            let begin = std::time::Instant::now();
            let result = walk_with(
                &memory,
                world::BASE,
                world::STATE,
                &mut output.0,
                &mut output.1,
                [true; 2],
                resolver,
                baseline,
                Some(Observation {
                    token,
                    marks: Some(marks),
                }),
                select,
            );
            (result, output, memory.take(), begin.elapsed())
        };
        // Warm up: storage, the owners' row storage, then every owner read
        // and recorded in full.
        let mut baseline = Baseline::default();
        let mut load = None;
        for _ in 0..3 {
            let (_, _, cost, time) = walk(&[], &mut baseline, &|_| true, &mut resolver);
            baseline.grow();
            load = Some((cost, time));
        }
        assert_eq!(baseline.valid_owners(), countries);
        let (load_cost, load_time) = load.unwrap();
        println!(
            "{countries} countries, full walk after a load: {load_cost:?} [calls, bytes, lines], {load_time:?}"
        );
        // A day of opinion writes, observed at the next hour.
        world.next_day();
        let marks = world.marks();
        baseline.reserve_marks(marks.len());
        let (mut opinions, mut tuples) = (Vec::new(), Vec::new());
        let mut hours = Vec::new();
        for hour in 0..24_u32 {
            let slice = move |owner: u32| owner % 24 == hour;
            let hour_marks: &[Mark] = if hour == 0 { &marks } else { &[] };
            let (observed, rows, cost, time) =
                walk(hour_marks, &mut baseline, &slice, &mut resolver);
            let (unobserved, reference, _, _) = walk(
                &[],
                &mut Baseline::default(),
                &slice,
                &mut Resolver::with_capacity(links),
            );
            assert_eq!(observed, unobserved, "{countries} hour {hour}");
            assert_eq!(
                rows.0[..observed.opinions.len()],
                reference.0[..unobserved.opinions.len()],
                "{countries} hour {hour}"
            );
            opinions.extend_from_slice(&rows.0[..observed.opinions.len()]);
            tuples.extend_from_slice(&rows.1[..observed.tuples.len()]);
            hours.push((cost, time));
        }
        assert!(baseline.counts().marks as usize >= marks.len() / 2);
        // The day's slices hold exactly the dense walk's rows of that day.
        let mut expected = rows();
        let dense = dense::walk(
            &memory,
            world::BASE,
            world::STATE,
            &mut expected.0,
            &mut expected.1,
            &mut Resolver::with_capacity(links),
        );
        let dense_cost = memory.take();
        println!("{countries} countries, dense: {dense_cost:?}");
        opinions.sort_by_key(|row| (row.owner_country, row.target_country));
        let mut dense_opinions = expected.0[..dense.opinions.len()].to_vec();
        dense_opinions.sort_by_key(|row| (row.owner_country, row.target_country));
        assert_eq!(opinions, dense_opinions, "{countries}");
        assert_eq!(tuples.len(), dense.tuples.len(), "{countries}");
        let mean = |field: usize| hours.iter().map(|(cost, _)| cost[field]).sum::<usize>() / 24;
        let worst = hours
            .iter()
            .max_by_key(|(cost, _)| cost[2])
            .expect("24 hours");
        println!(
            "{countries} countries, observed hour ({} marks that day): mean [{}, {}, {}] [calls, bytes, lines], worst {:?}, {:?}",
            marks.len(),
            mean(0),
            mean(1),
            mean(2),
            worst.0,
            worst.1
        );
        // An observed hour reads a small share of what a full walk reads.
        assert!(worst.0[2] * 8 < load_cost[2], "{:?} {load_cost:?}", worst.0);
    }
}

/// One object per region, so a synthetic world only stores the bytes the
/// walks read.
#[derive(Default)]
struct Sparse(std::collections::BTreeMap<usize, Vec<u8>>);

impl Sparse {
    fn object(&mut self, at: usize, len: usize) -> &mut Vec<u8> {
        self.0.entry(at).or_insert_with(|| vec![0; len])
    }
    fn p32(&mut self, object: usize, len: usize, at: usize, value: u32) {
        self.object(object, len)[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    fn p64(&mut self, object: usize, len: usize, at: usize, value: u64) {
        self.object(object, len)[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    /// A PDX array header at `object + at` over new data at `data`.
    fn array(&mut self, object: usize, len: usize, at: usize, data: usize, values: &[u64]) {
        self.p64(object, len, at, data as u64);
        self.p32(object, len, at + 8, values.len() as u32);
        self.p32(object, len, at + 12, values.len() as u32);
        let bytes: Vec<u8> = values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        self.0.insert(data, bytes);
    }
}

impl Memory for Sparse {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        let Some((&start, bytes)) = self.0.range(..=address).next_back() else {
            return false;
        };
        let offset = address - start;
        let Some(source) = bytes.get(offset..offset + output.len()) else {
            return false;
        };
        output.copy_from_slice(source);
        true
    }
}

/// A hub country (slot 1) with military access and docking rights toward
/// each of `partners` countries, in two differently shuffled orders, and
/// each partner owning its two relations with the hub.
fn star(partners: usize) -> Sparse {
    // The walk reads the diplomacy status, the occupation rule and the civil-war link.
    const COUNTRY: usize = ccountry::CIVIL_WAR_LINK + 4;
    const DIPLOMACY: usize = cdiplomacy_status::COUNTRY + 8;
    const STATUS: usize = AIR_BASE_ACCESS + 8;
    // The owned-relation read reaches the retired byte.
    const RELATION: usize = crelation::RETIRED + 1;
    let slots = partners + 2;
    let country = |slot: usize| 0x100_0000 + slot * 0x1000;
    let diplomacy = |slot: usize| 0x1000_0000 + slot * 0x1000;
    let matrix = |slot: usize| 0x2000_0000 + slot * 0x1_0000;
    let owned = |slot: usize| 0x4000_0000 + slot * 0x2_0000;
    // The status of `owner` toward `target`; one of them is the hub.
    let status = |owner: usize, target: usize| {
        0x8000_0000 + (owner.max(target) * 2 + usize::from(owner == 1)) * 0x1000
    };
    // Relation `class` (0 military access, 1 docking rights) with a partner.
    let relation = |partner: usize, class: usize| 0x1_0000_0000 + (partner * 2 + class) * 0x100;
    let mut world = Sparse::default();
    let mut table = vec![u64::MAX];
    table.extend((1..slots).map(|slot| country(slot) as u64));
    world.array(GS, 0x1000, cgame_state::COUNTRIES, 0x2000, &table);
    world.p64(GS, 0x1000, cgame_state::COUNTRY_LINKS, 0x40_0000);
    world.p32(GS, 0x1000, cgame_state::COUNTRY_LINKS + 8, slots as u32);
    world.p32(GS, 0x1000, cgame_state::COUNTRY_LINKS + 12, slots as u32);
    let links: Vec<u8> = (0..slots as u32).flat_map(u32::to_le_bytes).collect();
    world.0.insert(0x40_0000, links);
    let partner = |index: usize| index + 2;
    for slot in 1..slots {
        let (object, root) = (country(slot), diplomacy(slot));
        world.p64(object, COUNTRY, 0, (BASE + ccountry::VTABLE_RVA) as u64);
        world.p32(object, COUNTRY, ccountry::ID, slot as u32);
        world.p64(object, COUNTRY, ccountry::DIPLOMACY_STATUS, root as u64);
        world.p64(
            root,
            DIPLOMACY,
            0,
            (BASE + cdiplomacy_status::VTABLE_RVA) as u64,
        );
        world.p64(root, DIPLOMACY, cdiplomacy_status::COUNTRY, object as u64);
        // Only the hub's statuses and those toward the hub are set.
        let row: Vec<u64> = (0..slots)
            .map(|target| {
                if (slot == 1) != (target == 1) {
                    status(slot, target) as u64
                } else {
                    0
                }
            })
            .collect();
        let at = cdiplomacy_status::RELATIONS;
        world.array(root, DIPLOMACY, at, matrix(slot), &row);
        let relations: Vec<u64> = if slot == 1 {
            // Every partner twice, each class in its own shuffled order.
            let order = |index: usize, step: usize| partner(index * step % partners);
            (0..partners)
                .map(|index| relation(order(index, 7919), 0) as u64)
                .chain((0..partners).map(|index| relation(order(index, 104_729), 1) as u64))
                .collect()
        } else {
            vec![relation(slot, 0) as u64, relation(slot, 1) as u64]
        };
        let at = cdiplomacy_status::OWNED_RELATIONS;
        world.array(root, DIPLOMACY, at, owned(slot), &relations);
    }
    for index in 0..partners {
        let other = partner(index);
        let classes = [
            cmilitary_access_relation::TYPE,
            cdocking_rights_relation::TYPE,
        ];
        for (class, type_id) in classes.into_iter().enumerate() {
            let object = relation(other, class);
            world.p32(object, RELATION, crelation::TYPE_ID, type_id);
            world.p32(object, RELATION, crelation::FIRST_COUNTRY, 1);
            world.p32(object, RELATION, crelation::SECOND_COUNTRY, other as u32);
        }
        for (owner, target) in [(1, other), (other, 1)] {
            let object = status(owner, target);
            world.p64(object, STATUS, 0, (BASE + VTABLE_RVA) as u64);
            world.p64(object, STATUS, DIPLOMACY_STATUS, diplomacy(owner) as u64);
            world.p32(object, STATUS, TARGET_COUNTRY, target as u32);
            world.p64(object, STATUS, MILITARY_ACCESS, relation(other, 0) as u64);
            world.p64(object, STATUS, DOCKING_RIGHTS, relation(other, 1) as u64);
        }
    }
    world
}

/// A country with thousands of access partners, each named by two of its
/// owned relations: the walk emits one row per partner in first-mention
/// order, and its duplicate check stays constant-time (the scan it
/// replaced was quadratic in one country's partners). Prints the walk's
/// time; run with `--release --nocapture` to measure.
#[test]
fn links_walk_scales_to_five_thousand_partners() {
    for partners in [1_500, 5_000] {
        let world = star(partners);
        let slots = partners + 2;
        let mut resolver = Resolver::with_capacity(slots);
        let mut rows = vec![LinkRow::default(); partners * 2];
        let both = LINK_MILITARY_ACCESS | LINK_DOCKING_RIGHTS;
        // First mentions: the military access order.
        let mut expected: Vec<LinkRow> = (0..partners)
            .map(|index| LinkRow {
                country: 1,
                target: (index * 7919 % partners + 2) as u32,
                links: both,
            })
            .collect();
        expected.extend((2..slots as u32).map(|slot| LinkRow {
            country: slot,
            target: 1,
            links: both,
        }));
        let mut times = Vec::new();
        for _ in 0..5 {
            let begin = std::time::Instant::now();
            let outcome = links(
                &world,
                BASE,
                GS,
                &mut rows,
                // No country has an occupation rule, a civil war or a retired war.
                WarOutput {
                    rows: &mut [],
                    countries: &mut [],
                },
                &mut resolver,
            )
            .links;
            times.push(begin.elapsed());
            resolver.grow();
            assert_eq!(outcome, Outcome::Rows { len: partners * 2 }, "{partners}");
            assert_eq!(rows, expected, "{partners}");
        }
        times.sort();
        println!(
            "{partners} partners, links walk ({} owned relations): median {:?}, worst {:?}",
            partners * 4,
            times[2],
            times[4]
        );
    }
}
