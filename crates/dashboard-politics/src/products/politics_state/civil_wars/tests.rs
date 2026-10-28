use super::*;
use crate::products::politics_state::{
    history::{Part, Prepared},
    tests::{Changed, countries, frame, set},
};
use dashboard_base::prepared::series::tests::Fixture;
use replay_data::rows::{
    CountryCivilWarRow, CountryCounterInputRow, CountryWarRow, PoliticsRow, TableId, TableRows,
    WarRelationRow,
};
use replay_frame::frame::{DecodedFrame, TableView};

const ALL: &[(u32, u32, &str)] = &[(1, 1, "OLD"), (2, 2, "NEW"), (3, 3, "SPL"), (4, 4, "OUT")];

/// Records hours `first..=last` with the given civil-war lists, the
/// countries in `present` existing (owning a state) throughout.
fn record(
    state: &mut Prepared,
    first: i32,
    last: i32,
    present: &[(u32, u32, &str)],
    lists: &[(u32, u32)],
) {
    record_owning(state, first, last, present, Some(present), lists);
}

/// As [`record`], only the countries in `owning` owning a state; `None`
/// leaves the states owned unrecorded.
fn record_owning(
    state: &mut Prepared,
    first: i32,
    last: i32,
    present: &[(u32, u32, &str)],
    owning: Option<&[(u32, u32, &str)]>,
    lists: &[(u32, u32)],
) {
    for hour in first..=last {
        let mut at = frame(hour);
        countries(&mut at, present, &[]);
        match owning {
            Some(owning) => set(
                &mut at,
                TableId::CountryCounterInputs,
                TableRows::CountryCounterInputs(
                    present
                        .iter()
                        .map(|&(id, _, _)| CountryCounterInputRow {
                            country: id,
                            owned_state_count: i32::from(
                                owning.iter().any(|&(owner, _, _)| owner == id),
                            ),
                            ..Default::default()
                        })
                        .collect(),
                ),
            ),
            None => {
                *at.tables[TableId::CountryCounterInputs.index()] = TableView::Unavailable {
                    reason: "not recorded".into(),
                };
            }
        }
        set(
            &mut at,
            TableId::CountryCivilWars,
            TableRows::CountryCivilWars(
                lists
                    .iter()
                    .enumerate()
                    .map(|(ordinal, &(country, target))| CountryCivilWarRow {
                        country,
                        ordinal: ordinal as u32,
                        target,
                    })
                    .collect(),
            ),
        );
        state.step(&at);
    }
}

fn wars(state: &mut Prepared, through: i32, annexations: &[Annexation]) -> CivilWarsReply {
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut writer = fixture.writer();
    state.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    query(
        &fixture.store,
        &fixture.key,
        &CivilWarsParams {
            branch_id: "1".into(),
        },
        Some(through),
        annexations,
        &flag,
    )
    .unwrap()
}

fn span(first_hour: i32, last_hour: i32) -> HourSpan {
    HourSpan {
        first_hour,
        last_hour,
    }
}

fn annex(hour: i32, annexer: &str, annexed: &str, civil_war: bool) -> Annexation {
    Annexation {
        hour,
        read_hour: hour,
        annexer: Some(annexer.into()),
        annexed: Some(annexed.into()),
        civil_war,
    }
}

#[test]
fn a_breakaway_that_splits_again_builds_one_tree_with_asymmetric_lists() {
    let mut state = Prepared::default();
    record(&mut state, 90, 99, &ALL[..1], &[]);
    // Only the breakaway lists the old country: lists are not mirrored.
    record(&mut state, 100, 104, &ALL[..2], &[(2, 1)]);
    record(&mut state, 105, 110, &ALL[..3], &[(2, 1), (3, 2)]);
    let reply = wars(&mut state, 110, &[]);
    assert_eq!(reply.wars.len(), 1);
    let war = &reply.wars[0];
    assert_eq!((war.first_hour, war.last_hour), (100, 110));
    assert_eq!(war.at_war, [span(100, 110)]);
    assert_eq!(reply.known, [span(90, 110)]);
    let tags: Vec<_> = war.sides.iter().map(|side| side.tags[0].tag.as_str()).collect();
    assert_eq!(tags, ["OLD", "NEW", "SPL"]);
    assert_eq!(
        war.sides.iter().map(|side| side.parent).collect::<Vec<_>>(),
        [None, Some(0), Some(1)]
    );
    assert!(war.sides[0].existed_before);
    assert!(!war.sides[1].existed_before);
    assert_eq!(war.outcome, None);
}

#[test]
fn a_later_rival_does_not_change_the_side_a_breakaway_broke_from() {
    let mut state = Prepared::default();
    record(&mut state, 90, 99, &ALL[..1], &[]);
    record(&mut state, 100, 104, &ALL[..2], &[(2, 1)]);
    record(&mut state, 105, 109, &ALL[..3], &[(2, 1), (3, 2)]);
    // SPL, which broke from NEW, lists OLD only later.
    record(&mut state, 110, 115, &ALL[..3], &[(2, 1), (3, 2), (3, 1)]);
    let reply = wars(&mut state, 115, &[]);
    assert_eq!(
        reply.wars[0].sides.iter().map(|side| side.parent).collect::<Vec<_>>(),
        [None, Some(0), Some(1)]
    );
}

#[test]
fn an_annexer_listed_only_after_the_annexation_was_not_a_side() {
    let mut state = Prepared::default();
    record(&mut state, 90, 99, &ALL[..3], &[]);
    record(&mut state, 100, 110, &ALL[..3], &[(1, 2), (2, 1)]);
    // SPL annexes NEW at 110 and only then becomes OLD's rival, in the same episode.
    record_owning(&mut state, 111, 120, &ALL[..3], Some(&[ALL[0], ALL[2]][..]), &[(3, 1)]);
    record(&mut state, 121, 125, &ALL[..1], &[]);
    let reply = wars(
        &mut state,
        125,
        &[annex(110, "SPL", "NEW", true), annex(121, "OLD", "SPL", true)],
    );
    let war = &reply.wars[0];
    assert_eq!(war.sides.len(), 3);
    assert_eq!((war.sides[2].tags[0].tag.as_str(), war.sides[2].first_hour), ("SPL", 111));
    assert_eq!(war.sides[1].exit.as_ref().unwrap().kind, SideExitKind::AnnexedByOther);
    assert_eq!(war.sides[2].exit.as_ref().unwrap().kind, SideExitKind::AnnexedBySide);
    assert_eq!(war.outcome, None);
}

#[test]
fn a_side_restored_after_its_annexation_was_not_defeated_by_it() {
    let ended = |present: &[(u32, u32, &str)], lists: &[(u32, u32)], annexations: &[Annexation]| {
        let mut state = Prepared::default();
        record(&mut state, 100, 100, present, lists);
        // With the states owned unrecorded, only annexations tell a restoration.
        record_owning(&mut state, 101, 120, present, None, lists);
        record(&mut state, 121, 125, &ALL[..1], &[]);
        wars(&mut state, 125, annexations)
    };
    // Annexed by a side, restored, then annexed by an outsider: the last annexation decides.
    let reply = ended(
        &ALL[..2],
        &[(1, 2), (2, 1)],
        &[annex(110, "OLD", "NEW", true), annex(121, "OUT", "NEW", true)],
    );
    let exit = reply.wars[0].sides[1].exit.as_ref().unwrap();
    assert_eq!((exit.kind, exit.hour), (SideExitKind::AnnexedByOther, 121));
    assert_eq!(reply.wars[0].outcome, None);
    // Annexed by a side, then annexing another: it was restored, so it did not leave.
    let reply = ended(
        &ALL[..3],
        &[(1, 2), (2, 1), (3, 1)],
        &[annex(105, "OLD", "NEW", true), annex(115, "NEW", "SPL", true)],
    );
    let war = &reply.wars[0];
    assert_eq!(war.sides[1].exit, None);
    assert_eq!(war.sides[2].exit.as_ref().unwrap().kind, SideExitKind::AnnexedBySide);
    assert_eq!(war.outcome, None);
    // Annexing a country outside the war restores it all the same.
    let reply = ended(
        &ALL[..2],
        &[(1, 2), (2, 1)],
        &[annex(110, "OLD", "NEW", true), annex(115, "NEW", "OUT", false)],
    );
    assert_eq!(reply.wars[0].sides[1].exit, None);
    assert_eq!(reply.wars[0].outcome, None);
    // Restored long after its war ended, it was still defeated in that war.
    let mut state = Prepared::default();
    record(&mut state, 100, 120, &ALL[..2], &[(1, 2), (2, 1)]);
    record(&mut state, 121, 125, &ALL[..1], &[]);
    record(&mut state, 400, 500, &ALL[..2], &[]);
    let reply = wars(
        &mut state,
        500,
        &[annex(121, "OLD", "NEW", true), annex(450, "NEW", "OUT", false)],
    );
    assert_eq!(reply.wars[0].outcome.as_ref().map(|outcome| outcome.winner), Some(0));
}

#[test]
fn a_side_existing_again_after_its_annexation_was_not_defeated_by_it() {
    let lists = [(1, 2), (2, 1)];
    // OLD annexes NEW at 110 and the listings run on; from 120 NEW owns a state where `later` says so.
    let annexed = |later: Option<&[(u32, u32, &str)]>| {
        let mut state = Prepared::default();
        record(&mut state, 100, 110, &ALL[..2], &lists);
        record_owning(&mut state, 111, 119, &ALL[..2], Some(&ALL[..1]), &lists);
        record_owning(&mut state, 120, 140, &ALL[..2], later, &lists);
        record(&mut state, 141, 145, &ALL[..1], &[]);
        wars(&mut state, 145, &[annex(110, "OLD", "NEW", true)])
    };
    // Restored and fighting again before the episode's last listing.
    let reply = annexed(Some(&ALL[..2]));
    let war = &reply.wars[0];
    assert_eq!((war.first_hour, war.last_hour), (100, 140));
    assert_eq!(war.sides[1].exit, None);
    assert_eq!(war.outcome, None);
    // The listings alone do not undo the annexation.
    let reply = annexed(Some(&ALL[..1]));
    assert_eq!(reply.wars[0].sides[1].exit.as_ref().map(|exit| exit.hour), Some(110));
    assert_eq!(
        reply.wars[0].outcome.as_ref().map(|outcome| (outcome.winner, outcome.hour)),
        Some((0, 110))
    );
    // Nor do hours whose existence is unknown.
    let reply = annexed(None);
    assert_eq!(reply.wars[0].outcome.as_ref().map(|outcome| outcome.winner), Some(0));
}

#[test]
fn annexation_of_every_other_side_by_a_side_proves_a_winner() {
    let lists = [(1, 2), (2, 1)];
    let ended = |annexations: &[Annexation]| {
        let mut state = Prepared::default();
        record(&mut state, 100, 120, &ALL[..2], &lists);
        record(&mut state, 121, 125, &ALL[..1], &[]);
        wars(&mut state, 125, annexations)
    };
    let won = ended(&[annex(121, "OLD", "NEW", true)]);
    let war = &won.wars[0];
    assert_eq!(war.at_war, [span(100, 120)]);
    assert_eq!(
        war.outcome,
        Some(CivilWarOutcome {
            winner: 0,
            hour: 121,
            read_hour: 121
        })
    );
    assert_eq!(war.sides[1].exit.as_ref().unwrap().kind, SideExitKind::AnnexedBySide);
    // A side annexing the other by another route, such as a peace conference, wins too.
    let won = ended(&[annex(121, "OLD", "NEW", false)]);
    assert_eq!(won.wars[0].outcome.as_ref().map(|outcome| outcome.winner), Some(0));
    assert_eq!(
        won.wars[0].sides[1].exit.as_ref().unwrap().kind,
        SideExitKind::AnnexedBySideOtherwise
    );
    // An annexation by an outsider decides nothing.
    let reply = ended(&[annex(121, "OUT", "NEW", true)]);
    assert_eq!(reply.wars[0].outcome, None);
    assert!(reply.wars[0].sides[1].exit.is_some());
    // Lists ending with no annexation leave the outcome unknown.
    assert_eq!(ended(&[]).wars[0].outcome, None);
}

#[test]
fn listings_apart_in_time_are_separate_civil_wars() {
    let mut state = Prepared::default();
    record(&mut state, 100, 105, &ALL[..2], &[(1, 2)]);
    record(&mut state, 106, 200, &ALL[..2], &[]);
    record(&mut state, 201, 205, &ALL[..2], &[(2, 1)]);
    let reply = wars(&mut state, 205, &[]);
    assert_eq!(reply.wars.len(), 2);
    assert_eq!((reply.wars[0].first_hour, reply.wars[0].last_hour), (100, 105));
    assert_eq!(reply.wars[0].at_war, [span(100, 105)]);
    assert_eq!(reply.wars[1].first_hour, 201);
}

#[test]
fn a_stale_listing_of_a_country_without_states_is_not_an_ongoing_war() {
    let mut state = Prepared::default();
    record(&mut state, 100, 110, &ALL[..2], &[(2, 1)]);
    // NEW keeps listing OLD after OLD lost its last state: the game's
    // predicate no longer holds for either, though the listing remains.
    record_owning(&mut state, 111, 120, &ALL[..2], Some(&ALL[1..2]), &[(2, 1)]);
    let reply = wars(&mut state, 120, &[]);
    let war = &reply.wars[0];
    assert_eq!((war.first_hour, war.last_hour), (100, 120));
    // Ongoing only while OLD still owned a state; the stale listing after it
    // is dated, so a viewer at any hour resolves it.
    assert_eq!(war.at_war, [span(100, 110)]);
}

#[test]
fn hours_the_lists_were_not_read_are_unknown_not_an_ending() {
    let mut state = Prepared::default();
    record(&mut state, 100, 110, &ALL[..2], &[(2, 1)]);
    for hour in 111..=120 {
        let mut at = frame(hour);
        countries(&mut at, &ALL[..2], &[]);
        *at.tables[TableId::CountryCivilWars.index()] = TableView::Unavailable {
            reason: "not recorded".into(),
        };
        state.step(&at);
    }
    let reply = wars(&mut state, 120, &[]);
    // The listing ends where the lists stop being read: that end is unknown.
    assert_eq!(reply.wars[0].last_hour, 110);
    assert_eq!(reply.known, [span(100, 110)]);
    assert!(
        reply
            .warnings
            .iter()
            .any(|warning| warning.contains("civil-war lists unavailable"))
    );
}

#[test]
fn only_hours_changing_what_a_part_reads_observe_it() {
    // NEW lists OLD; at war with NEW through a relation that ends at 104,
    // OLD has a civil war while NEW exists and lists it.
    let at = |hour: i32, old_states: i32, casualties: i64, capital: i32| {
        let mut at = frame(hour);
        countries(&mut at, &ALL[..2], &[]);
        set(
            &mut at,
            TableId::CountryCounterInputs,
            TableRows::CountryCounterInputs(vec![
                CountryCounterInputRow {
                    country: 1,
                    owned_state_count: old_states,
                    ..Default::default()
                },
                CountryCounterInputRow {
                    country: 2,
                    owned_state_count: 1,
                    ..Default::default()
                },
            ]),
        );
        set(
            &mut at,
            TableId::CountryCivilWars,
            TableRows::CountryCivilWars(vec![CountryCivilWarRow {
                country: 2,
                ordinal: 0,
                target: 1,
            }]),
        );
        set(
            &mut at,
            TableId::WarRelations,
            TableRows::WarRelations(vec![WarRelationRow {
                first_country: 1,
                second_country: 2,
                start_hour: 50,
                end_hour: 104,
                first_casualties: casualties,
                ..Default::default()
            }]),
        );
        set(
            &mut at,
            TableId::Politics,
            TableRows::Politics(vec![PoliticsRow {
                country: 1,
                capital,
                has_elections: false.into(),
                election_interval: 0.into(),
                ..Default::default()
            }]),
        );
        at
    };
    let mut state = Prepared::default();
    let hours = [
        // The first hour follows nothing: every part.
        (at(100, 1, 0, 7), Changed::default()),
        // A war's casualties, every hour of a war: nothing the parts read.
        (at(101, 1, 500, 7), Changed::fields(TableId::WarRelations, &["first_casualties"])),
        // OLD loses its last state: NEW's own list no longer names an existing country.
        (at(102, 0, 900, 7), Changed::fields(TableId::CountryCounterInputs, &["owned_state_count"])),
        // A capital moves: not part of an election schedule.
        (at(103, 0, 900, 8), Changed::fields(TableId::Politics, &["capital"])),
        // The relation's recorded end arrives with no row changing: OLD's enemy is gone.
        (at(104, 0, 900, 8), Changed::default()),
    ];
    for (frame, changed) in &hours {
        state.observe_changes(frame, changed).unwrap();
    }
    assert_eq!(
        state.observed,
        [
            (100, Part::Elections),
            (100, Part::Exiles),
            (100, Part::Balances),
            (100, Part::CivilWars),
            (102, Part::CivilWars),
            (104, Part::CivilWars),
        ]
    );
    let reply = wars(&mut state, 104, &[]);
    let war = &reply.wars[0];
    assert_eq!((war.first_hour, war.last_hour), (100, 104));
    // NEW held it through 101, OLD through its enemy until the relation ended.
    assert_eq!(war.at_war, [span(100, 103)]);
    assert_eq!(reply.known, [span(100, 104)]);
}

#[test]
fn the_predicate_reads_each_enemy_vector_as_the_game_scans_it() {
    // Only NEW and SPL list OLD; OLD has a civil war through SPL, its enemy
    // by a relation, which lists it.
    let mut state = Prepared::default();
    let mut at = frame(100);
    countries(&mut at, &ALL[..3], &[]);
    set(
        &mut at,
        TableId::CountryCounterInputs,
        TableRows::CountryCounterInputs(
            [(1, 1), (2, 0), (3, 1)]
                .map(|(country, owned_state_count)| CountryCounterInputRow {
                    country,
                    owned_state_count,
                    ..Default::default()
                })
                .to_vec(),
        ),
    );
    set(
        &mut at,
        TableId::CountryCivilWars,
        TableRows::CountryCivilWars(vec![
            CountryCivilWarRow {
                country: 2,
                ordinal: 0,
                target: 1,
            },
            CountryCivilWarRow {
                country: 3,
                ordinal: 0,
                target: 1,
            },
        ]),
    );
    set(
        &mut at,
        TableId::WarRelations,
        TableRows::WarRelations(vec![WarRelationRow {
            first_country: 3,
            second_country: 1,
            start_hour: 50,
            end_hour: 0,
            ..Default::default()
        }]),
    );
    state.step(&at);
    let reply = wars(&mut state, 100, &[]);
    assert_eq!(reply.wars[0].sides.len(), 3);
    assert_eq!(reply.wars[0].at_war, [span(100, 100)]);
    let holds = |frame: &DecodedFrame, slot: u32| dashboard_text::identity::civil_war(frame, slot);
    assert_eq!([1, 2, 3].map(|slot| holds(&at, slot)), [Ok(true), Ok(true), Ok(true)]);
    // A vector read from CountryWars replaces the relations: OLD then has no enemy.
    let vector = |countries: &[u32]| {
        let mut read = at.clone();
        set(
            &mut read,
            TableId::CountryWars,
            TableRows::CountryWars(
                countries
                    .iter()
                    .map(|&country| CountryWarRow {
                        country,
                        enemies_read: 1,
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        read
    };
    assert_eq!(holds(&vector(&[1]), 1), Ok(false));
    // An unresolved row refuses only an answer the scan reaches it for:
    // before OLD's own row it refuses OLD, after it nothing; NEW's own list
    // answers before any enemy is read.
    assert!(holds(&vector(&[0, 1]), 1).is_err());
    assert_eq!(holds(&vector(&[0, 1]), 2), Ok(true));
    assert_eq!(holds(&vector(&[1, 99]), 1), Ok(false));
}
