//! Subjects, peace outcomes and idea history prepared on their own, before
//! the hourly walk, from blocks read by their changes: they publish and save
//! exactly what hourly preparation does across quiet spans, deadlines inside
//! them, fields they do not watch changing every hour, two branches, a gap,
//! append, resume and refusals, with one and several pre-pass workers, at
//! once or one after another; their queries answer while the hourly walk has
//! accepted nothing; and their quiet hours build no frame.
use super::family_lanes::{assert_payloads_equal, payloads};
use super::*;
use crate::prepared::prepass;
use crate::products::subjects::prepared::counted;
use independent::{Own, quiet_politics};
use replay::rows::{
    ActiveIdeaRow, AutonomyRow, CharacterRow, CountryAppearanceRow, CountryCounterInputRow,
    CountryWarRow, HiredAdvisorRow, LexicalTokenRow, ModifierDefinitionRow, PeaceConferenceRow,
    PeaceSettlementRow, PoliticalPartyRow, PoliticsRow, ProvinceRow, SettledPeaceTransfer,
    SubjectRow, WarRelationRow,
};

type Payloads = BTreeMap<(String, u64), Vec<u8>>;

const A: u32 = 100_000;
const B: u32 = 100_001;
const C: u32 = 100_002;
const D: u32 = 100_003;
const COUNTRIES: [u32; 4] = [A, B, C, D];

/// Branch 1's last hour.
const LAST: i32 = 216;

/// Every table of the three families at `hour` of `branch`, over the four
/// countries A to D (`frame`), whose balances move every hour. Each table
/// also has fields the families do not read moving every hour: subject
/// autonomy ratios, autonomy scores, state manpower, province controllers,
/// war casualties, elections, party popularity, appearance colours,
/// controlled state counts, enemy occupation flags and character rosters.
///
/// Subjects: A holds B from 90; A holds C from 106 with a recorded end at 115
/// (a deadline inside a quiet span) and drops the row at 119; B holds D from
/// 120, cancelled from 123. B's level is low until 109, then high; C has no
/// level and D's is not text (diagnostic rates). The Subjects table is not
/// read at 127 and 128.
///
/// Peace: the roster is read at hour 21 of each day (117 reads an empty
/// roster in a quiet span). Conference 9 is listed from 141; the read at 165
/// finds it unchanged (a scheduled read of a held roster, a deadline); it
/// finalizes at the read at 189, when A's war with C records its end, state
/// 3 passes from C to A at 190 with a settlement row, and the read at 213
/// finds it gone. The settlement table is not read at 150 to 152.
///
/// Ideas: A holds an idea throughout and another from 106 to 110; B adopts a
/// law at 108; A appoints character 7 at 103 and character 8, who has no
/// Characters row (an hour-dated refusal), at 104. B is named by a name tag,
/// so its new holdings read the civil-war operands; A takes a cosmetic tag at
/// 130. ActiveIdeas is not read at 132 and 133.
fn politics_frame(branch: u64, hour: i32) -> DecodedFrame {
    let mut observed = frame(branch, hour, 4);
    let complete = |rows: TableRows| -> TableView { TableView::Complete(rows) };
    let unread = |reason: &str| TableView::Unavailable {
        reason: reason.into(),
    };
    let set = |observed: &mut DecodedFrame, table: TableId, view: TableView| {
        *observed.tables[table.index()] = view;
    };
    let ratio = i64::from(hour);

    // Subjects and their levels.
    let subject = |overlord, subject, start_hour, end_hour, cancel, state: &[u8]| SubjectRow {
        overlord,
        subject,
        start_hour,
        end_hour,
        cancel,
        autonomy_ratio: ratio,
        state: state.to_vec(),
    };
    let mut subjects = vec![subject(A, B, 90, 0, 0, b"puppet")];
    if (106..=118).contains(&hour) {
        subjects.push(subject(A, C, 106, 115, 0, b"puppet"));
    }
    if hour >= 120 {
        subjects.push(subject(B, D, 120, 0, u8::from(hour >= 123), b"dominion"));
    }
    set(
        &mut observed,
        TableId::Subjects,
        if (127..=128).contains(&hour) {
            unread("Subjects not read")
        } else {
            complete(TableRows::Subjects(subjects))
        },
    );
    let mut levels = vec![AutonomyRow {
        subject: B,
        score: ratio,
        state: if hour < 109 {
            b"low".to_vec()
        } else {
            b"high".to_vec()
        },
        ..Default::default()
    }];
    if hour >= 120 {
        levels.push(AutonomyRow {
            subject: D,
            score: ratio,
            state: vec![0xff],
            ..Default::default()
        });
    }
    set(
        &mut observed,
        TableId::Autonomy,
        complete(TableRows::Autonomy(levels)),
    );

    // Peace: owners, provinces, wars, the roster and settlements.
    if let TableView::Complete(TableRows::States(rows)) =
        &mut *observed.tables[TableId::States.index()]
    {
        for row in rows.iter_mut() {
            row.manpower_available = hour;
            if row.id == 3 && hour >= 190 {
                row.owner = A;
            }
        }
    }
    set(
        &mut observed,
        TableId::Provinces,
        complete(TableRows::Provinces(
            (1..=4_u32)
                .flat_map(|state| {
                    [(state * 2, 5), (state * 2 + 1, 0)].map(|(id, victory_points)| ProvinceRow {
                        id,
                        state,
                        victory_points,
                        controller: A + hour.rem_euclid(4) as u32,
                        ..Default::default()
                    })
                })
                .collect(),
        )),
    );
    set(
        &mut observed,
        TableId::WarRelations,
        complete(TableRows::WarRelations(vec![WarRelationRow {
            first_country: A,
            second_country: C,
            start_hour: 100,
            end_hour: if hour >= 189 { 189 } else { 0 },
            first_casualties: ratio,
            ..Default::default()
        }])),
    );
    let conference = |current_turn, finalized| PeaceConferenceRow {
        id: 9,
        ordinal: 0,
        current_turn,
        finalized,
    };
    let roster = match hour {
        141..=188 => vec![conference(1, 0)],
        189..=212 => vec![conference(2, 1)],
        _ => Vec::new(),
    };
    set(
        &mut observed,
        TableId::PeaceConferences,
        complete(TableRows::PeaceConferences(roster)),
    );
    observed.metrics.table_ns[TableId::PeaceConferences.index()] =
        if hour.rem_euclid(24) == 21 { 1 } else { 0 };
    let settlements = if hour >= 190 {
        vec![PeaceSettlementRow {
            sequence: 8,
            conference: Some(9),
            hour: Some(189),
            winners: vec![A],
            losers: vec![C],
            transfers: vec![SettledPeaceTransfer {
                state: 3,
                taker: A,
                automatic: false,
            }],
            ..Default::default()
        }]
    } else {
        Vec::new()
    };
    set(
        &mut observed,
        TableId::PeaceSettlements,
        if (150..=152).contains(&hour) {
            unread("Settlements not read")
        } else {
            complete(TableRows::PeaceSettlements(settlements))
        },
    );
    set(
        &mut observed,
        TableId::Politics,
        complete(TableRows::Politics(
            COUNTRIES
                .map(|country| PoliticsRow {
                    country,
                    ruling_party: Some(3),
                    last_election: hour,
                    ..Default::default()
                })
                .to_vec(),
        )),
    );
    set(
        &mut observed,
        TableId::PoliticalParties,
        complete(TableRows::PoliticalParties(
            COUNTRIES
                .map(|country| PoliticalPartyRow {
                    country,
                    id: 3,
                    token: b"party".to_vec(),
                    popularity: ratio,
                    ..Default::default()
                })
                .to_vec(),
        )),
    );
    set(
        &mut observed,
        TableId::CountryAppearance,
        complete(TableRows::CountryAppearance(
            COUNTRIES
                .map(|country| CountryAppearanceRow {
                    country,
                    red: hour as u32,
                    leader_ideology: b"ideology".to_vec(),
                    name_tag: if country == B {
                        b"BBB".to_vec()
                    } else {
                        Vec::new()
                    },
                    cosmetic_tag: if country == A && hour >= 130 {
                        b"FASCIST".to_vec()
                    } else {
                        Vec::new()
                    },
                    ..Default::default()
                })
                .to_vec(),
        )),
    );
    set(
        &mut observed,
        TableId::CountryCounterInputs,
        complete(TableRows::CountryCounterInputs(
            COUNTRIES
                .map(|country| CountryCounterInputRow {
                    country,
                    owned_state_count: if country == C && hour >= 190 { 0 } else { 1 },
                    controlled_state_count: hour,
                    ..Default::default()
                })
                .to_vec(),
        )),
    );
    set(
        &mut observed,
        TableId::LexicalTokens,
        complete(TableRows::LexicalTokens(
            [
                (5, "CHAR_SEVEN"),
                (77, "take_states"),
                (991, "frontier_demand"),
            ]
            .map(|(token, literal)| LexicalTokenRow {
                token,
                literal: literal.as_bytes().to_vec(),
            })
            .to_vec(),
        )),
    );

    // Ideas, advisors and naming operands.
    let mut ideas = vec![ActiveIdeaRow {
        country: A,
        ordinal: 0,
        token: b"mod_idea".to_vec(),
    }];
    if (106..=110).contains(&hour) {
        ideas.push(ActiveIdeaRow {
            country: A,
            ordinal: 1,
            token: b"mod_later_idea".to_vec(),
        });
    }
    if hour >= 108 {
        ideas.push(ActiveIdeaRow {
            country: B,
            ordinal: 0,
            token: b"law_x".to_vec(),
        });
    }
    set(
        &mut observed,
        TableId::ActiveIdeas,
        if (132..=133).contains(&hour) {
            unread("ActiveIdeas not read")
        } else {
            complete(TableRows::ActiveIdeas(ideas))
        },
    );
    let advisor = |ordinal, character, idea: &[u8], slot: &[u8]| HiredAdvisorRow {
        country: A,
        ordinal,
        character,
        idea: idea.to_vec(),
        slot: slot.to_vec(),
    };
    let advisors = [
        (103, advisor(0, 7, b"adv_idea", b"political_advisor")),
        (104, advisor(1, 8, b"ghost_idea", b"theorist")),
    ]
    .into_iter()
    .filter(|(from, _)| hour >= *from)
    .map(|(_, row)| row)
    .collect();
    set(
        &mut observed,
        TableId::HiredAdvisors,
        complete(TableRows::HiredAdvisors(advisors)),
    );
    set(
        &mut observed,
        TableId::Characters,
        complete(TableRows::Characters(vec![CharacterRow {
            id: 7,
            country: A,
            name_token_id: 5,
            token: b"char_seven".to_vec(),
            roster: hour.rem_euclid(3) as u8,
            gender: hour as u32,
            ..Default::default()
        }])),
    );
    set(
        &mut observed,
        TableId::ModifierDefinitions,
        complete(TableRows::ModifierDefinitions(vec![
            ModifierDefinitionRow {
                index: 0x8d,
                token: 5,
                flags: 3,
                ..Default::default()
            },
        ])),
    );
    set(
        &mut observed,
        TableId::CountryCivilWars,
        complete(TableRows::CountryCivilWars(Vec::new())),
    );
    set(
        &mut observed,
        TableId::CountryWars,
        complete(TableRows::CountryWars(
            COUNTRIES
                .map(|country| CountryWarRow {
                    country,
                    can_occupy_non_war: hour.rem_euclid(2) as u8,
                    enemies_read: 0,
                    ..Default::default()
                })
                .to_vec(),
        )),
    );
    observed
}

/// Branch 1's hours from 100 through `last`, without 136 and 137 (a gap
/// between blocks), in blocks of 12, then branch 2's one block (hours 20 to
/// 25) when `second`.
fn scenario(last: i32, second: bool) -> Replay {
    let hours: Vec<i32> = (100..=last)
        .filter(|hour| !(136..=137).contains(hour))
        .collect();
    let frames = |hours: &[i32]| -> Vec<DecodedFrame> {
        hours.iter().map(|&hour| politics_frame(1, hour)).collect()
    };
    let mut blocks = hours.chunks(12);
    let mut replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, frames(blocks.next().unwrap()))],
    )
    .unwrap();
    for block in blocks {
        replay = replay
            .append(&[continuation_records(&replay, 1, frames(block))])
            .unwrap();
    }
    if second {
        replay = replay
            .append(&[records(
                2,
                (20..=25).map(|hour| politics_frame(2, hour)).collect(),
            )])
            .unwrap();
    }
    replay
}

/// The three families as the families prepared on their own.
fn families() -> Vec<&'static dyn Own> {
    vec![
        &quiet_politics::SUBJECTS,
        &quiet_politics::PEACE,
        &quiet_politics::IDEAS,
    ]
}

/// Each family's name, its continuation section's prefix and the hourly
/// branch state's fields that continuation holds.
const CONTINUATIONS: [(&str, &str, &[&str]); 3] = [
    (
        "subjects",
        "subject-state-",
        &["subject_changes", "subject_late"],
    ),
    ("peace", "peace-state-", &["peace"]),
    ("ideas", "idea-state-", &["idea_changes"]),
];

fn clear(source: Arc<Source>) {
    super::super::super::files(&source, None)
        .unwrap()
        .0
        .clear()
        .unwrap();
}

/// Every block committed on its own.
fn measured(source: &Source) {
    run_measured_through(
        source,
        None,
        &AtomicBool::new(false),
        true,
        Duration::ZERO,
        Some(u64::MAX),
        |_| Ok(()),
        |_| {},
    )
    .unwrap();
}

fn plain(source: &Source) {
    run(source, None, &AtomicBool::new(false), |_| {}).unwrap();
}

/// Prepare a fresh source with `prepare` and take its payloads.
fn prepared(fixture: &mut Fixture, name: &str, prepare: &dyn Fn(&Source)) -> Payloads {
    let source = fixture.source(name);
    prepare(&source);
    let payloads = payloads(&source, None);
    clear(source);
    payloads
}

/// Sections only one mode writes, or writes differently: the checkpoint
/// (which holds the families' progress), the families' own coverage pages
/// and continuations, and each branch's state, which holds their
/// continuations only in hourly preparation ([`assert_same`] compares both).
fn mode_specific(name: &str) -> bool {
    name == "preparation-checkpoint-v1"
        || name.starts_with("preparation-family-")
        || name.starts_with("preparation-branch-")
        || CONTINUATIONS
            .iter()
            .any(|(_, prefix, _)| name.starts_with(prefix))
}

fn branch_state(bytes: &[u8]) -> serde_json::Value {
    serde_json::from_slice(&zstd::stream::decode_all(bytes).unwrap()).unwrap()
}

/// Every section both modes publish is byte-identical (with `refusals`, but
/// for each branch's coverage page, whose refusals the families keep in their
/// own pages). Without refusals, each family's continuation is the one hourly
/// preparation keeps in each branch's state, and the rest of that state is
/// the same.
fn assert_same(actual: &Payloads, hourly: &Payloads, label: &str, refusals: bool) {
    let shared = |payloads: &Payloads| -> Payloads {
        payloads
            .iter()
            .filter(|((name, _), _)| {
                !mode_specific(name) && !(refusals && name.starts_with("preparation-coverage-"))
            })
            .map(|(key, bytes)| (key.clone(), bytes.clone()))
            .collect()
    };
    assert_payloads_equal(&shared(actual), &shared(hourly), label);
    for published in [
        "subjects-",
        "subject-changes-",
        "peace-deals-",
        "idea-history-",
        "idea-changes-",
    ] {
        assert!(
            actual.keys().any(|(name, _)| name.starts_with(published)),
            "{label}: nothing published as {published}"
        );
    }
    if refusals {
        return;
    }
    let mut states = 0;
    for ((name, range), bytes) in hourly {
        let Some(branch) = name.strip_prefix("preparation-branch-") else {
            continue;
        };
        states += 1;
        let mut expected = branch_state(bytes);
        let mut own = branch_state(&actual[&(name.clone(), *range)]);
        for (family, prefix, fields) in CONTINUATIONS {
            let section = actual
                .get(&(format!("{prefix}{branch}"), 0))
                .unwrap_or_else(|| panic!("{label}: no {family} continuation of branch {branch}"));
            let continuation: serde_json::Value = serde_json::from_slice(section).unwrap();
            for &field in fields {
                let value = expected.as_object_mut().unwrap().remove(field).unwrap();
                own.as_object_mut().unwrap().remove(field);
                let saved = if fields.len() == 1 {
                    &continuation
                } else {
                    &continuation[field]
                };
                assert_eq!(saved, &value, "{label}: branch {branch}'s {field}");
            }
        }
        assert_eq!(own, expected, "{label}: branch {branch}'s other state");
    }
    assert!(states > 0, "{label}: no branch state");
}

/// Blocks each family observed hour by hour, and folds that fell back.
fn notes() -> Vec<usize> {
    ["subjects", "peace", "ideas"]
        .into_iter()
        .flat_map(|family| {
            [
                prepass::noted(&format!("{family}.sequential")),
                prepass::noted(&format!("{family}.fallback")),
            ]
        })
        .collect()
}

/// Hours recorded in `replay`.
fn hours(replay: &Replay) -> usize {
    let mut hours = 0;
    for index in &replay.index {
        replay
            .read_block_alone(
                index,
                &[TableId::Campaign],
                |_, _| {
                    hours += 1;
                    Ok(())
                },
                || Ok(()),
            )
            .unwrap();
    }
    hours
}

#[test]
fn quiet_politics_prepared_on_their_own_publish_and_save_what_hourly_preparation_does() {
    let replay = scenario(LAST, true);
    let mut fixture = Fixture::new(&replay);
    let hourly =
        independent::with_test_hourly(|| prepared(&mut fixture, "politics-parity", &plain));
    for workers in [1, 3] {
        for together in [true, false] {
            let label = format!("{workers} workers, at once {together}");
            let before = notes();
            let own = prepass::with_test_prepass(workers, None, || {
                independent::with_test_families(&families(), || {
                    if together {
                        prepared(&mut fixture, "politics-parity", &plain)
                    } else {
                        independent::with_test_one_after_another(|| {
                            prepared(&mut fixture, "politics-parity", &plain)
                        })
                    }
                })
            });
            assert_same(&own, &hourly, &label, false);
            assert_eq!(
                notes(),
                before,
                "{label}: every block folded from its extract"
            );
        }
    }
}

#[test]
fn quiet_politics_prepared_on_their_own_commit_resume_reload_and_append_as_hourly_preparation() {
    let first = scenario(161, false);
    let grown = scenario(LAST, true);
    // The hourly walk prepares one block per run.
    let resumed = |source: &Source| {
        let flag = AtomicBool::new(false);
        while unprepared_blocks(source, None).unwrap() != Some(0) {
            run_measured(source, None, &flag, false, Duration::ZERO, |_| {}).unwrap();
        }
    };
    // Cancelled at the first accepted report, the families' own commits
    // included, then prepared again. Every block commits on its own in both
    // runs: the modes report acceptance at different points (the hourly walk
    // per block before its commit, a family after its commit), so batched
    // commits would split the same series into different ranges.
    let cancelled = |source: &Source| {
        let flag = AtomicBool::new(false);
        match run_measured_through(
            source,
            None,
            &flag,
            true,
            Duration::ZERO,
            Some(u64::MAX),
            |_| Ok(()),
            |_| flag.store(true, Ordering::Release),
        ) {
            Ok(_) => {}
            Err(error) => assert_eq!(error.kind(), io::ErrorKind::Interrupted),
        }
        flag.store(false, Ordering::Release);
        measured(source);
    };
    let modes: [(&str, &dyn Fn(&Source), bool); 5] = [
        ("run", &plain, false),
        ("measured", &measured, false),
        ("measured, reloaded", &measured, true),
        ("resumed", &resumed, false),
        ("cancelled", &cancelled, false),
    ];
    let mut fixture = Fixture::new(&first);
    for (mode, prepare, reload) in modes {
        let both = |fixture: &mut Fixture| -> Payloads {
            super::super::RELOAD_FLUSHED.with(|flag| flag.set(reload));
            fixture.replace(&first);
            let source = fixture.source("politics-commits");
            prepare(&source);
            drop(source);
            fixture.replace(&grown);
            let source = fixture.source("politics-commits");
            prepare(&source);
            super::super::RELOAD_FLUSHED.with(|flag| flag.set(false));
            let payloads = payloads(&source, None);
            clear(source);
            payloads
        };
        let hourly = independent::with_test_hourly(|| both(&mut fixture));
        let own = prepass::with_test_prepass(2, None, || {
            independent::with_test_families(&families(), || both(&mut fixture))
        });
        assert_same(&own, &hourly, mode, false);
    }
}

/// The families' refusals of each branch, from the checkpoint in hourly
/// preparation, or from their own coverage pages.
fn family_refusals(source: &Source, own: bool) -> BTreeMap<u64, BTreeMap<String, Refusal>> {
    let (store, key) = super::super::super::files(source, None).unwrap();
    let (state, _) = checkpoint(&store, &key, None).unwrap().unwrap();
    let mut refusals = BTreeMap::new();
    for branch in [1, 2] {
        let refused = if own {
            let mut refused = BTreeMap::new();
            for (family, _, _) in CONTINUATIONS {
                let page = store
                    .read(&key, &independent::coverage_section(family, branch), 0)
                    .unwrap()
                    .map(|file| {
                        serde_json::from_reader::<_, CoveragePage>(std::io::BufReader::new(file))
                            .unwrap()
                    });
                if let Some(refusal) = page.and_then(|page| page.refused.0.get(family).cloned()) {
                    refused.insert(family.to_owned(), refusal);
                }
            }
            refused
        } else {
            state
                .refused
                .get(&branch)
                .map(|refused| {
                    refused
                        .0
                        .iter()
                        .filter(|(family, _)| {
                            CONTINUATIONS
                                .iter()
                                .any(|(name, _, _)| *name == family.as_str())
                        })
                        .map(|(family, refusal)| (family.clone(), refusal.clone()))
                        .collect()
                })
                .unwrap_or_default()
        };
        refusals.insert(branch, refused);
    }
    refusals
}

#[test]
fn quiet_politics_prepared_on_their_own_refuse_where_hourly_preparation_does() {
    let replay = scenario(LAST, true);
    let mut fixture = Fixture::new(&replay);
    let flag = AtomicBool::new(false);
    // At a block's first hour, and at an hour inside a block that no field
    // the family reads changes.
    for (family, hour) in [
        ("subjects", 124),
        ("subjects", 117),
        ("peace", 124),
        ("peace", 118),
        ("ideas", 124),
        ("ideas", 118),
    ] {
        let label = format!("{family} refusing at {hour}");
        let refusals = [(family, hour)];
        let (hourly, hourly_refused) = independent::with_test_hourly(|| {
            lanes::with_test_refusals(&refusals, || {
                let source = fixture.source("politics-refusal");
                measured(&source);
                let result = (payloads(&source, None), family_refusals(&source, false));
                clear(source);
                result
            })
        });
        let source = fixture.source("politics-refusal");
        let sequential = prepass::noted(&format!("{family}.sequential"));
        lanes::with_test_refusals(&refusals, || {
            prepass::with_test_prepass(2, None, || {
                independent::with_test_families(&families(), || measured(&source))
            })
        });
        assert_eq!(
            prepass::noted(&format!("{family}.sequential")) - sequential,
            1,
            "{label}: the refusing block is observed hour by hour"
        );
        assert_same(&payloads(&source, None), &hourly, &label, true);
        let own_refused = family_refusals(&source, true);
        assert_eq!(own_refused, hourly_refused, "{label}");
        assert_eq!(own_refused[&1][family].hour, hour, "{label}");
        assert!(own_refused[&2].is_empty(), "{label}: branch 2 goes on");
        // The family's queries refuse from its own page on that branch only.
        let product = match family {
            "subjects" => "subjects",
            "peace" => "peace-deals",
            _ => "idea-history",
        };
        independent::with_test_families(&families(), || {
            let error = refusal(&source, None, 1, product, &flag).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{label}");
            assert_eq!(
                error.to_string(),
                format!("Injected refusal of {family} at hour {hour}"),
                "{label}"
            );
            refusal(&source, None, 2, product, &flag).unwrap();
        });
        clear(source);
    }
}

fn query(
    source: &Source,
    product: &str,
    params: serde_json::Value,
) -> io::Result<serde_json::Value> {
    let request = crate::transport::ProductRequest {
        source_id: source.key().source_id.clone(),
        revision: source.key().revision.clone(),
        product: product.into(),
        params,
    };
    let reply = super::super::super::query::query(source, &request, None, &AtomicBool::new(false))?;
    let mut reply: serde_json::Value = serde_json::from_str(&reply).unwrap();
    Ok(reply["data"].take())
}

/// The peace-deals reply the prepared pages give for `branch`.
fn peace_reply(source: &Source, branch: &str) -> serde_json::Value {
    let (store, key) = super::super::super::files(source, None).unwrap();
    crate::products::peace_deals::prepared::query(
        source,
        &crate::products::LoadContext { content: None },
        serde_json::json!({ "branchId": branch }),
        &store,
        &key,
        &AtomicBool::new(false),
    )
    .unwrap()
}

#[test]
fn quiet_politics_answer_from_their_own_coverage_while_the_hourly_walk_accepts_nothing() {
    use crate::products::{
        LoadContext, Product,
        subjects::{SubjectParams, Subjects},
    };
    let replay = scenario(LAST, true);
    let mut fixture = Fixture::new(&replay);
    // The peace replies of hourly preparation, with every block accepted.
    let peace: Vec<serde_json::Value> = independent::with_test_hourly(|| {
        let source = fixture.source("politics-hourly-peace");
        plain(&source);
        let replies = ["1", "2"]
            .map(|branch| peace_reply(&source, branch))
            .to_vec();
        clear(source);
        replies
    });
    independent::with_test_families(&families(), || {
        let source = fixture.source("politics-stalled");
        let flag = AtomicBool::new(false);
        let ideas = serde_json::json!({ "branchId": "1", "tag": "000" });
        assert_eq!(
            query(&source, "idea-history", ideas.clone())
                .unwrap_err()
                .to_string(),
            "Choose the game and user folders in Settings first"
        );
        // The hourly walk stalls: it is cancelled at its first commit.
        let mut commits = 0;
        let error = independent::with_test_one_after_another(|| {
            run_checked(
                &source,
                None,
                &flag,
                |_| {},
                |_| {
                    commits += 1;
                    flag.store(true, Ordering::Release);
                    Ok(())
                },
            )
        })
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        assert_eq!(commits, 1);
        flag.store(false, Ordering::Release);
        assert_eq!(
            unprepared_blocks(&source, None).unwrap(),
            Some(replay.index.len()),
            "the hourly walk accepted no block"
        );
        for (position, branch) in ["1", "2"].into_iter().enumerate() {
            let id = branch.parse().unwrap();
            assert!(
                coverage(&source, None, id, &flag).unwrap().is_none(),
                "branch {branch}: the hourly walk covers nothing"
            );
            for product in ["subjects", "peace-deals", "idea-history"] {
                assert!(
                    complete(&source, None, id, product, &flag).unwrap(),
                    "branch {branch}: {product} covers the branch on its own"
                );
            }
            let oracle = Subjects::load(
                &source,
                &SubjectParams {
                    branch_id: branch.into(),
                },
                &LoadContext { content: None },
                &flag,
            )
            .unwrap();
            let mut answered = query(
                &source,
                "subjects",
                serde_json::json!({ "branchId": branch }),
            )
            .unwrap();
            assert_eq!(answered["coverage"]["complete"], true);
            assert_eq!(answered["pending"], serde_json::json!([]));
            for field in ["coverage", "pending"] {
                answered.as_object_mut().unwrap().remove(field);
            }
            assert_eq!(
                answered,
                serde_json::to_value(oracle).unwrap(),
                "branch {branch}: the subjects answer from their own coverage"
            );
            assert_eq!(
                peace_reply(&source, branch),
                peace[position],
                "branch {branch}: the peace pages are hourly preparation's"
            );
        }
        // Past its gate, the idea query asks for the game's content, which
        // its classification and naming need.
        let error = query(&source, "idea-history", ideas).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Choose the game and user folders in Settings first"
        );
        clear(source);
    });
}

/// Read by their changes, each family alone builds frames only at the hours
/// a field it reads changes, at its deadlines and at each block's first
/// hour: fewer than half the recorded hours, every one kept and observed
/// once on the carried state. Read whole, every hour is a frame. Both
/// publish what hourly preparation does.
#[test]
fn quiet_politics_quiet_hours_build_no_frame() {
    let replay = scenario(LAST, true);
    let recorded = hours(&replay);
    let mut fixture = Fixture::new(&replay);
    let hourly =
        independent::with_test_hourly(|| prepared(&mut fixture, "politics-frames", &plain));
    let alone: [(&str, &'static dyn Own); 3] = [
        ("subjects", &quiet_politics::SUBJECTS),
        ("peace", &quiet_politics::PEACE),
        ("ideas", &quiet_politics::IDEAS),
    ];
    for (family, own) in alone {
        for whole in [true, false] {
            let label = format!("{family}, read whole {whole}");
            let before = (counted(family), notes());
            let source = fixture.source("politics-frames");
            let read = || {
                prepass::with_test_prepass(2, None, || {
                    independent::with_test_families(&[own], || plain(&source))
                })
            };
            if whole {
                prepass::with_test_alone(read);
            } else {
                read();
            }
            let payloads = payloads(&source, None);
            clear(source);
            assert_eq!(notes(), before.1, "{label}: every block folded");
            // Only this family's sections are its own here.
            let shared = |payloads: &Payloads| -> Payloads {
                payloads
                    .iter()
                    .filter(|((name, _), _)| !mode_specific(name))
                    .map(|(key, bytes)| (key.clone(), bytes.clone()))
                    .collect()
            };
            assert_payloads_equal(&shared(&payloads), &shared(&hourly), &label);
            let (observed, frames) = counted(family);
            let (observed, frames) = (observed - before.0.0, frames - before.0.1);
            if whole {
                assert_eq!(frames, recorded, "{label}: a frame every hour");
            } else {
                assert!(
                    frames * 2 < recorded,
                    "{label}: {frames} frames of {recorded} hours"
                );
            }
            assert_eq!(
                observed, frames,
                "{label}: every frame kept and observed once"
            );
        }
    }
}

/// Before peace preparation covers the branch, its event page answers the
/// prepared prefix instead of null: the deals as the pages through the
/// prefix's last hour give them, the conference still open there preparing
/// its outcome rather than claiming none was recorded, the hours after it
/// pending rather than unobserved. Once complete, it is hourly preparation's
/// reply with nothing pending.
#[test]
fn quiet_politics_event_pages_answer_their_prepared_prefix_with_the_hours_still_pending() {
    let replay = scenario(LAST, false);
    let mut fixture = Fixture::new(&replay);
    independent::with_test_hourly(|| {
        let source = fixture.source("politics-prefix");
        let flag = AtomicBool::new(false);
        let page = serde_json::json!({ "branchId": "1", "family": { "kind": "peace-deals" } });
        assert!(
            !query(&source, "events-page", page.clone())
                .unwrap()
                .is_null()
        );
        // The hourly walk accepts a block per run: stop while conference 9
        // (listed from 141, finalized at 189) is open.
        while coverage(&source, None, 1, &flag)
            .unwrap()
            .is_none_or(|coverage| coverage.last_hour < 150)
        {
            run_measured(&source, None, &flag, true, Duration::ZERO, |_| {}).unwrap();
        }
        let accepted = coverage(&source, None, 1, &flag).unwrap().unwrap();
        assert!(!accepted.complete && (141..189).contains(&accepted.last_hour));
        let prefix = query(&source, "events-page", page.clone()).unwrap();
        assert_eq!(prefix["coverage"], serde_json::to_value(&accepted).unwrap());
        assert_eq!(
            prefix["approximate"],
            serde_json::json!([
                { "firstHour": 100, "lastHour": 135 },
                { "firstHour": 138, "lastHour": accepted.last_hour },
            ]),
            "accepted peace outcomes remain revisable until the branch is complete"
        );
        let pending = crate::prepared::prefix::pending(&source, 1, accepted.last_hour, None);
        assert_eq!(
            pending.first().map(|span| span.first_hour),
            Some(accepted.last_hour + 1)
        );
        assert_eq!(pending.last().map(|span| span.last_hour), Some(LAST));
        assert_eq!(prefix["pending"], serde_json::to_value(&pending).unwrap());
        let unsettled = [
            "No complete ownership and war read after settlement; outcomes are not recorded.",
            "No end was observed; the conference outcome is not recorded.",
        ];
        let deals = prefix["deals"].as_array().unwrap();
        assert!(
            deals
                .iter()
                .any(|deal| deal["preparing"] == serde_json::json!(["outcome"])),
            "{prefix}"
        );
        for deal in deals {
            for line in deal["limitations"].as_array().unwrap() {
                assert!(!unsettled.contains(&line.as_str().unwrap()), "{prefix}");
            }
        }
        for spans in prefix["unobserved"].as_object().unwrap().values() {
            for span in spans.as_array().unwrap() {
                assert!(span["lastHour"].as_i64().unwrap() <= i64::from(accepted.last_hour));
            }
        }
        plain(&source);
        let complete = query(&source, "events-page", page).unwrap();
        assert_eq!(complete["coverage"]["complete"], true);
        assert_eq!(complete["pending"], serde_json::json!([]));
        let mut hourly = peace_reply(&source, "1");
        hourly["coverage"] = complete["coverage"].clone();
        hourly["pending"] = serde_json::json!([]);
        hourly["approximate"] = serde_json::json!([]);
        assert_eq!(complete, hourly);
        assert!(
            complete["deals"]
                .as_array()
                .unwrap()
                .iter()
                .all(|deal| deal["preparing"] == serde_json::json!([]))
        );
        clear(source);
    });
}

/// Each relation a subjects reply holds at `hour`, with the subject's level
/// there: (overlord, subject, start hour, level).
fn subjects_at(
    reply: &serde_json::Value,
    hour: i64,
) -> std::collections::BTreeSet<(String, String, i64, String)> {
    let mut held = std::collections::BTreeSet::new();
    for run in reply["runs"].as_array().unwrap() {
        if run["firstHour"].as_i64().unwrap() > hour || run["lastHour"].as_i64().unwrap() < hour {
            continue;
        }
        let level = run["steps"]
            .as_array()
            .unwrap()
            .iter()
            .find(|step| {
                step["firstHour"].as_i64().unwrap() <= hour
                    && step["lastHour"].as_i64().unwrap() >= hour
            })
            .expect("steps cover the run")["level"]
            .to_string();
        held.insert((
            run["overlord"].as_str().unwrap().to_owned(),
            run["subject"].as_str().unwrap().to_owned(),
            run["startHour"].as_i64().unwrap(),
            level,
        ));
    }
    held
}

/// Before subject preparation covers the branch, the subjects reply answers
/// its prepared prefix exactly and each later block from its keyframe, marked
/// approximate, instead of null: every hour of the prefix and every sampled
/// keyframe holds the relations and levels of the complete reply, the
/// relation open at the prefix's end goes on into the first sampled block,
/// the recorded gap after the prefix stays unobserved and nothing that was
/// sampled is pending. Once complete, it is hourly preparation's reply with
/// nothing pending or approximate.
#[test]
fn subjects_answer_their_prepared_prefix_and_sample_each_later_keyframe() {
    use crate::products::{
        LoadContext, Product,
        subjects::{SubjectParams, Subjects},
    };
    let replay = scenario(LAST, false);
    let mut fixture = Fixture::new(&replay);
    independent::with_test_hourly(|| {
        let source = fixture.source("subjects-prefix");
        let flag = AtomicBool::new(false);
        let params = serde_json::json!({ "branchId": "1" });
        assert!(
            !query(&source, "subjects", params.clone())
                .unwrap()
                .is_null()
        );
        let oracle = serde_json::to_value(
            Subjects::load(
                &source,
                &SubjectParams {
                    branch_id: "1".into(),
                },
                &LoadContext { content: None },
                &flag,
            )
            .unwrap(),
        )
        .unwrap();
        // The first commit ends after the branch's first block (100 to 111).
        run_measured(&source, None, &flag, true, Duration::ZERO, |_| {}).unwrap();
        let accepted = coverage(&source, None, 1, &flag).unwrap().unwrap();
        assert_eq!((accepted.last_hour, accepted.complete), (111, false));
        let reply = query(&source, "subjects", params.clone()).unwrap();
        assert_eq!(reply["coverage"], serde_json::to_value(&accepted).unwrap());
        for hour in 100..=111 {
            assert_eq!(
                subjects_at(&reply, hour),
                subjects_at(&oracle, hour),
                "hour {hour}"
            );
        }
        // Every later block is sampled at its keyframe, so nothing is pending.
        let keyframes: Vec<i64> = replay
            .index
            .iter()
            .filter(|block| block.branch == 1 && block.first_hour > 111)
            .map(|block| i64::from(block.first_hour))
            .collect();
        assert!(keyframes.len() > 3);
        for &hour in &keyframes {
            assert_eq!(
                subjects_at(&reply, hour),
                subjects_at(&oracle, hour),
                "keyframe {hour}"
            );
        }
        assert_eq!(
            reply["approximate"],
            serde_json::json!([
                {"firstHour": 112, "lastHour": 135},
                {"firstHour": 138, "lastHour": LAST}
            ])
        );
        assert_eq!(reply["pending"], serde_json::json!([]));
        assert!(
            reply["unobserved"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!({"firstHour": 136, "lastHour": 137}))
        );
        // A holds B from 90 throughout: one run from the branch's start into
        // the sampled hours, broken only at the gap.
        let held: Vec<_> = reply["runs"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|run| run["startHour"] == 90)
            .map(|run| {
                (
                    run["firstHour"].as_i64().unwrap(),
                    run["lastHour"].as_i64().unwrap(),
                )
            })
            .collect();
        assert_eq!(held, [(100, 135), (138, i64::from(LAST))]);
        plain(&source);
        let mut complete = query(&source, "subjects", params).unwrap();
        assert_eq!(complete["coverage"]["complete"], true);
        assert_eq!(complete["pending"], serde_json::json!([]));
        assert!(complete.get("approximate").is_none());
        for field in ["coverage", "pending"] {
            complete.as_object_mut().unwrap().remove(field);
        }
        assert_eq!(complete, oracle);
        clear(source);
    });
}
