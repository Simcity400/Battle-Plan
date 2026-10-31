//! Wars and war casualties prepared on their own beside the hourly walk,
//! publish and save exactly what hourly preparation does, read only the hours
//! a field they compare changes (and the wars' capitulation date), refuse
//! where hourly preparation refuses, and answer their queries while the
//! hourly walk has accepted nothing.
use super::family_lanes::{assert_payloads_equal, assert_versions, payloads};
use super::*;
use crate::prepared::prepass;
use crate::products::{war_casualties::prepared::block as casualties, wars::block as wars};
use replay_data::{
    rows::AutonomyRow, rows::CapitulationRow, rows::PoliticalPartyRow, rows::StateRow,
    rows::WarRelationRow,
};
use serde_json::{Value, json};

type Payloads = BTreeMap<(String, u64), Vec<u8>>;

/// Hours whose casualties move: the counters are constant in between.
const FIGHTING: [(i32, i32); 3] = [(100, 105), (114, 116), (20, 21)];

/// Branch 1's blocks; hours 112 and 113 are a gap between the third and
/// fourth.
const BLOCKS: [&[i32]; 6] = [
    &[100, 101, 102, 103],
    &[104, 105, 106, 107],
    &[108, 109, 110, 111],
    &[114, 115, 116, 117],
    &[118, 119, 120, 121],
    &[122, 123, 124, 125],
];

/// The hours a field either family compares changes or the wars'
/// capitulation date falls, block first hours included: ONE fights TWO
/// throughout and THREE until 109 (inside a block); casualties move in
/// [`FIGHTING`]; TWO is renamed at 110 (inside a block); THREE's
/// capitulation, dated 120, is recorded from 115, and 120 falls inside the
/// quiet hours of its block. Branch 2 is one block, 20 to 23.
const WARS_KEPT: usize = 17;
const CASUALTIES_KEPT: usize = 16;

/// The wars fixture at `hour`, with every field neither family compares
/// moving every hour: country balances, a state's manpower, a party's
/// popularity, an autonomy score and the clock.
fn war_frame(branch: u64, hour: i32) -> DecodedFrame {
    let mut frame = crate::products::wars::test_fixtures::recorded(hour);
    frame.branch = branch;
    // Recorded, as every recording reads it: a table unavailable every hour
    // is decoded again every hour, which makes every hour one the wars visit.
    *frame.tables[TableId::ReservedCountryNaming.index()] =
        TableView::Complete(TableRows::ReservedCountryNaming(vec![
            replay_data::rows::ReservedCountryNamingRow::default(),
        ]));
    let fought: i64 = FIGHTING
        .iter()
        .map(|&(first, last)| i64::from((last.min(hour) - first + 1).max(0)))
        .sum();
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *frame.tables[TableId::Countries.index()]
    {
        for row in rows {
            row.political_power_raw = Some(i64::from(hour) * 1000);
            row.command_power_raw = i64::from(hour);
        }
    }
    let ended = branch == 1 && hour >= 109;
    *frame.tables[TableId::WarRelations.index()] = TableView::Complete(TableRows::WarRelations(
        [2_u32, 3]
            .into_iter()
            .filter(|&second| second == 2 || !ended)
            .map(|second| WarRelationRow {
                first_country: 1,
                second_country: second,
                start_hour: 90,
                first_was_instigator: 1,
                first_casualties: 7 * fought + i64::from(second),
                second_casualties: 3 * fought,
                second_unknown_casualties: fought,
                ..Default::default()
            })
            .collect(),
    ));
    if ended
        && let TableView::Complete(TableRows::WarOwnedRelations(rows)) =
            &mut *frame.tables[TableId::WarOwnedRelations.index()]
    {
        rows.retain(|row| row.first_country != 3 && row.second_country != 3);
    }
    if branch == 1
        && hour >= 110
        && let TableView::Complete(TableRows::CountryAppearance(rows)) =
            &mut *frame.tables[TableId::CountryAppearance.index()]
    {
        rows[1].name_tag = b"TWO_NEW".to_vec();
    }
    *frame.tables[TableId::Capitulation.index()] = TableView::Complete(TableRows::Capitulation(
        std::iter::once(CapitulationRow {
            country: 2,
            ..Default::default()
        })
        .chain((branch == 1 && hour >= 115).then(|| CapitulationRow {
            country: 3,
            capitulated: 1,
            capitulated_hour: 120,
            ..Default::default()
        }))
        .collect(),
    ));
    *frame.tables[TableId::States.index()] =
        TableView::Complete(TableRows::States(vec![StateRow {
            id: 1,
            owner: 1,
            controller: 1,
            custom_name: b"Home".to_vec(),
            manpower_available: hour,
            ..Default::default()
        }]));
    *frame.tables[TableId::PoliticalParties.index()] =
        TableView::Complete(TableRows::PoliticalParties(vec![PoliticalPartyRow {
            country: 1,
            id: 1,
            token: b"party".to_vec(),
            popularity: i64::from(hour),
            ..Default::default()
        }]));
    *frame.tables[TableId::Autonomy.index()] =
        TableView::Complete(TableRows::Autonomy(vec![AutonomyRow {
            subject: 2,
            state: b"free".to_vec(),
            score: i64::from(hour),
            ..Default::default()
        }]));
    frame
}

/// Branch 1's first `blocks` blocks, then branch 2's one block when `second`.
fn scenario(blocks: usize, second: bool) -> Replay {
    scenario_of(war_frame, blocks, second)
}

/// Hours TWO and THREE fight a war of their own: it starts inside branch 1's
/// first block and ends inside its second.
const JOINED: (i32, i32) = (102, 105);

/// [`war_frame`] with a war between TWO and THREE through [`JOINED`], whose
/// casualties and both sides' unknown losses move every hour, and ONE's
/// unknown losses moving with the fighting: relation lifetimes first held and
/// ended inside blocks, and unknown losses whose agreement breaks where a
/// lifetime starts (TWO's) or holds across it (THREE's).
fn joined_frame(branch: u64, hour: i32) -> DecodedFrame {
    let mut frame = war_frame(branch, hour);
    let fought: i64 = FIGHTING
        .iter()
        .map(|&(first, last)| i64::from((last.min(hour) - first + 1).max(0)))
        .sum();
    let joined = branch == 1 && (JOINED.0..=JOINED.1).contains(&hour);
    if let TableView::Complete(TableRows::WarRelations(rows)) =
        &mut *frame.tables[TableId::WarRelations.index()]
    {
        for row in rows.iter_mut() {
            row.first_unknown_casualties = 2 * fought;
        }
        if joined {
            let since = i64::from(hour - JOINED.0 + 1);
            rows.push(WarRelationRow {
                first_country: 2,
                second_country: 3,
                start_hour: JOINED.0,
                first_was_instigator: 1,
                first_casualties: 5 * since,
                second_casualties: 11 * since,
                first_unknown_casualties: fought,
                second_unknown_casualties: since,
                ..Default::default()
            });
        }
    }
    if joined
        && let TableView::Complete(TableRows::WarOwnedRelations(rows)) =
            &mut *frame.tables[TableId::WarOwnedRelations.index()]
    {
        for owner in [2, 3] {
            rows.push(replay_data::rows::WarOwnedRelationRow {
                owner,
                ordinal: 1,
                first_country: 2,
                second_country: 3,
                ..Default::default()
            });
        }
        rows.sort_by_key(|row| (row.owner, row.ordinal));
    }
    frame
}

/// Branch 1's first `blocks` blocks of `frame`, then branch 2's one block
/// when `second`.
fn scenario_of(frame: fn(u64, i32) -> DecodedFrame, blocks: usize, second: bool) -> Replay {
    let frames =
        |hours: &[i32]| -> Vec<DecodedFrame> { hours.iter().map(|&hour| frame(1, hour)).collect() };
    let mut replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, frames(BLOCKS[0]))]).unwrap();
    for &hours in &BLOCKS[1..blocks] {
        replay = replay
            .append(&[continuation_records(&replay, 1, frames(hours))])
            .unwrap();
    }
    if second {
        replay = replay
            .append(&[records(2, (20..=23).map(|hour| frame(2, hour)).collect())])
            .unwrap();
    }
    replay
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

/// Run `run` with wars and casualties the only families prepared on their
/// own: every other family is prepared hour by hour in both modes.
fn own<T>(run: impl FnOnce() -> T) -> T {
    let families: [&'static dyn independent::Own; 2] =
        [&independent::WARS, &independent::CASUALTIES];
    independent::with_test_families(&families, run)
}

fn clear(source: Arc<Source>) {
    super::super::super::files(&source, None)
        .unwrap()
        .0
        .clear()
        .unwrap();
}

fn query(source: &Source, product: &str, params: Value) -> io::Result<Value> {
    let request = crate::transport::ProductRequest {
        source_id: source.key().source_id.clone(),
        revision: source.key().revision.clone(),
        product: product.into(),
        params,
    };
    let reply = super::super::super::query::query(source, &request, None, &AtomicBool::new(false))?;
    let mut reply: Value = serde_json::from_str(&reply).unwrap();
    Ok(reply["data"].take())
}

/// The wars', casualties' and breakdown's queries of one branch, by label.
fn requests(branch: u64) -> Vec<(&'static str, &'static str, Value)> {
    let id = branch.to_string();
    let (hours, last): (Vec<i32>, i32) = if branch == 1 {
        (BLOCKS.concat(), 125)
    } else {
        ((20..=23).collect(), 23)
    };
    let reads: Vec<Value> = hours.iter().map(|hour| json!({"hour": hour})).collect();
    vec![
        (
            "wars, recorded viewpoint",
            "wars",
            json!({"branchId": id, "viewpointTag": null, "reads": reads}),
        ),
        (
            "wars, viewpoint TWO",
            "wars",
            json!({"branchId": id, "viewpointTag": "TWO", "reads": reads}),
        ),
        (
            "casualty pairs manifest",
            "casualty-pairs",
            json!({"branchId": id, "tags": ["ONE"], "between": false, "window": null}),
        ),
        (
            "casualty pairs window",
            "casualty-pairs",
            json!({"branchId": id, "tags": ["ONE"], "between": false, "window": 0}),
        ),
        (
            "casualty breakdown",
            "casualty-breakdown",
            json!({"branchId": id, "tag": "ONE", "window": null}),
        ),
        (
            "casualty values",
            "series-state",
            json!({"branchId": id, "tags": ["ONE"], "family": {"kind": "war-casualties"},
                "reads": [{"tag": "", "hour": last}]}),
        ),
    ]
}

/// Sections only one mode writes: the checkpoint (which holds the families'
/// progress), the versions page (checked against each run's plan), the
/// families' coverage pages, the casualties' own continuation and the recorded
/// viewpoint's published walk; branch states are compared without the
/// casualties.
fn mode_specific(name: &str) -> bool {
    name == "preparation-checkpoint-v1"
        || name == "preparation-versions-v1"
        || name.starts_with("preparation-family-")
        || name.starts_with("preparation-branch-")
        || name.starts_with("war-casualties-state-")
        || name.starts_with("preparation-coverage-")
        || walk_section(name)
}

/// A branch coverage page without the refusals of the families prepared on
/// their own here: hourly preparation records them on the branch's page,
/// a family prepared on its own on its own page.
fn branch_coverage(bytes: &[u8]) -> Value {
    let mut page: Value = serde_json::from_slice(bytes).unwrap();
    let page_object = page.as_object_mut().unwrap();
    if let Some(Value::Object(refused)) = page_object.get_mut("refused") {
        refused.remove("wars");
        refused.remove("casualties");
        if refused.is_empty() {
            page_object.remove("refused");
        }
    }
    page
}

/// The recorded viewpoint's walk published while preparing.
fn walk_section(name: &str) -> bool {
    name.starts_with("wars-walk-")
}

/// The recorded viewpoint's wars of each branch, reading every recorded hour.
fn default_wars(source: &Source) -> Vec<Value> {
    [1, 2]
        .into_iter()
        .map(|branch| {
            let (_, product, params) = requests(branch).swap_remove(0);
            query(source, product, params).unwrap()
        })
        .collect()
}

fn decompressed_json(bytes: &[u8]) -> Value {
    serde_json::from_slice(&zstd::stream::decode_all(bytes).unwrap()).unwrap()
}

/// Every section both modes publish is byte-identical, and every branch
/// state equal but for the casualties. With `continuation`, the casualties'
/// own continuation is the one hourly preparation saves in its branch state.
fn assert_same(actual: &Payloads, hourly: &Payloads, label: &str, continuation: bool) {
    assert_versions(actual, label);
    assert_versions(hourly, label);
    let shared = |payloads: &Payloads| -> Payloads {
        payloads
            .iter()
            .filter(|((name, _), _)| !mode_specific(name))
            .map(|(key, bytes)| (key.clone(), bytes.clone()))
            .collect()
    };
    for prefix in [
        "wars-series-",
        "wars-1-catalogue",
        "casualty-values-",
        "casualty-pairs-",
    ] {
        assert!(
            actual.keys().any(|(name, _)| name.starts_with(prefix)),
            "{label}: nothing published as {prefix}"
        );
    }
    assert_payloads_equal(&shared(actual), &shared(hourly), label);
    let coverage = |payloads: &Payloads| -> BTreeMap<(String, u64), Value> {
        payloads
            .iter()
            .filter(|((name, _), _)| name.starts_with("preparation-coverage-"))
            .map(|(key, bytes)| (key.clone(), branch_coverage(bytes)))
            .collect()
    };
    let pages = coverage(hourly);
    assert!(!pages.is_empty(), "{label}: no branch coverage page");
    assert_eq!(coverage(actual), pages, "{label}: branch coverage pages");
    let mut states = 0;
    for ((name, range), bytes) in hourly
        .iter()
        .filter(|((name, _), _)| name.starts_with("preparation-branch-"))
    {
        let mut expected = decompressed_json(bytes);
        let mut found = decompressed_json(&actual[&(name.clone(), *range)]);
        let saved = expected
            .as_object_mut()
            .unwrap()
            .remove("casualties")
            .unwrap();
        found.as_object_mut().unwrap().remove("casualties");
        assert_eq!(found, expected, "{label}: {name}");
        if continuation {
            let branch: u64 = name
                .strip_prefix("preparation-branch-")
                .unwrap()
                .parse()
                .unwrap();
            let own = &actual[&(casualties::state_section(branch), 0)];
            assert_eq!(
                decompressed_json(own),
                saved,
                "{label}: branch {branch}'s casualty continuation"
            );
            states += 1;
        }
    }
    assert!(
        !continuation || states > 0,
        "{label}: no casualty continuation"
    );
}

/// This thread's counts: each family's blocks observed hour by hour and
/// folds that fell back, then the hours observed on carried state and the
/// frames extracts were given.
fn counts() -> [usize; 8] {
    [
        prepass::noted("wars.sequential"),
        prepass::noted("wars.fallback"),
        prepass::noted("casualties.sequential"),
        prepass::noted("casualties.fallback"),
        wars::observed_hours(),
        wars::built_frames(),
        casualties::observed_hours(),
        casualties::built_frames(),
    ]
}

#[test]
fn wars_and_casualties_prepared_on_their_own_publish_and_save_what_hourly_preparation_does() {
    let replay = scenario(6, true);
    let total = hours(&replay);
    let mut fixture = Fixture::new(&replay);
    let hourly = independent::with_test_hourly(|| {
        let source = fixture.source("wars-parity");
        run(&source, None, &AtomicBool::new(false), |_| {}).unwrap();
        let payloads = payloads(&source, None);
        clear(source);
        payloads
    });
    // Read whole and read by changes one family after another on this
    // thread, which counts their work; read by changes at once.
    for (whole, workers, parallelism, counted) in [
        (true, 1, 2, true),
        (false, 1, 2, true),
        (false, 3, 4, false),
    ] {
        let source = fixture.source("wars-parity");
        let before = counts();
        let prepare = || {
            own(|| {
                prepass::with_test_prepass(workers, None, || {
                    lanes::with_test_lanes(parallelism, None, || {
                        run(&source, None, &AtomicBool::new(false), |_| {})
                    })
                })
            })
        };
        let read = || {
            if counted {
                independent::with_test_one_after_another(prepare)
            } else {
                prepare()
            }
        };
        if whole {
            prepass::with_test_alone(read)
        } else {
            read()
        }
        .unwrap();
        let label = format!(
            "{} with {workers} workers, {parallelism} processors",
            if whole {
                "read whole"
            } else {
                "read by changes"
            }
        );
        assert_same(&payloads(&source, None), &hourly, &label, true);
        let after = counts();
        let delta: Vec<usize> = after.iter().zip(before).map(|(a, b)| a - b).collect();
        assert_eq!(
            delta[..4],
            [0, 0, 0, 0],
            "{label}: every block folded from its extract"
        );
        if counted {
            let (wars_observed, wars_frames) = (delta[4], delta[5]);
            let (casualties_observed, casualties_frames) = (delta[6], delta[7]);
            if whole {
                assert_eq!(wars_frames, total, "{label}: a wars frame every hour");
                assert_eq!(
                    casualties_frames, total,
                    "{label}: a casualty frame every hour"
                );
            } else {
                // Quiet hours build no frame and are not observed again.
                assert_eq!(wars_frames, WARS_KEPT, "{label}: wars frames");
                assert_eq!(
                    casualties_frames, CASUALTIES_KEPT,
                    "{label}: casualty frames"
                );
            }
            // The extracts observe every kept hour on the workers; the folds
            // observe each block's first hour on the carried state and no
            // other.
            let blocks = replay.index.len();
            assert_eq!(
                wars_observed, blocks,
                "{label}: wars hours observed on the carried observer"
            );
            assert_eq!(
                casualties_observed, blocks,
                "{label}: casualty hours observed on the carried observer"
            );
        }
        clear(source);
    }
}

#[test]
fn wars_and_casualties_prepared_on_their_own_commit_resume_reload_and_append_as_hourly_preparation()
{
    let first = scenario(3, false);
    let grown = scenario(6, true);
    let plain = |source: &Source| {
        run(source, None, &AtomicBool::new(false), |_| {}).unwrap();
    };
    // Every block committed on its own.
    let measured = |source: &Source| {
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
    };
    // The hourly walk prepares one block per run.
    let resumed = |source: &Source| {
        let flag = AtomicBool::new(false);
        while unprepared_blocks(source, None).unwrap() != Some(0) {
            run_measured(source, None, &flag, false, Duration::ZERO, |_| {}).unwrap();
        }
    };
    let modes: [PrepareReloadMode; 4] = [
        ("run", &plain, false),
        ("measured", &measured, false),
        ("measured, reloaded", &measured, true),
        ("resumed", &resumed, false),
    ];
    let mut fixture = Fixture::new(&first);
    // The published walk of every mode committing each block.
    let mut walks: Option<Payloads> = None;
    for (mode, prepare, reload) in modes {
        let both = |fixture: &mut Fixture| -> (Payloads, Vec<Value>) {
            super::super::RELOAD_FLUSHED.with(|flag| flag.set(reload));
            fixture.replace(&first);
            let source = fixture.source("wars-commits");
            prepare(&source);
            drop(source);
            fixture.replace(&grown);
            let source = fixture.source("wars-commits");
            prepare(&source);
            super::super::RELOAD_FLUSHED.with(|flag| flag.set(false));
            let result = (payloads(&source, None), default_wars(&source));
            clear(source);
            result
        };
        let (hourly, hourly_wars) = independent::with_test_hourly(|| both(&mut fixture));
        let (own, own_wars) = own(|| both(&mut fixture));
        assert_same(&own, &hourly, mode, true);
        // Its published walk answers what walking every input answers.
        assert_eq!(
            own_wars, hourly_wars,
            "{mode}: the recorded viewpoint's wars"
        );
        assert!(
            own.keys().any(|(name, _)| walk_section(name)),
            "{mode}: the walk is published"
        );
        if mode != "run" {
            // Continued, reloaded and resumed walks publish the same pages
            // at the same commits.
            let published: Payloads = own
                .into_iter()
                .filter(|((name, _), _)| walk_section(name))
                .collect();
            let expected = walks.take().unwrap_or_else(|| published.clone());
            assert_payloads_equal(&published, &expected, &format!("{mode}: published walk"));
            walks = Some(expected);
        }
    }
}

#[test]
fn the_recorded_viewpoints_walk_published_while_preparing_answers_from_its_points() {
    use crate::products::wars::walks_decoded_inputs as decoded;
    let replay = scenario(6, true);
    let mut fixture = Fixture::new(&replay);
    let request = |view: Option<&str>, hours: &[i32]| {
        let reads: Vec<Value> = hours.iter().map(|hour| json!({"hour": hour})).collect();
        json!({"branchId": "1", "viewpointTag": view, "reads": reads})
    };
    let sets: [&[i32]; 6] = [
        &[100],
        &[110],
        &[125],
        &[111, 114],
        &[101, 116, 121],
        &BLOCKS.concat(),
    ];
    // ONE is the recorded viewpoint at every hour; TWO never is.
    let views = [None, Some("ONE"), Some("TWO")];
    // What walking every input answers.
    let expected: Vec<Vec<Value>> = independent::with_test_hourly(|| {
        let source = fixture.source("wars-walk");
        run(&source, None, &AtomicBool::new(false), |_| {}).unwrap();
        let replies = views
            .iter()
            .map(|&view| {
                sets.iter()
                    .map(|hours| query(&source, "wars", request(view, hours)).unwrap())
                    .collect()
            })
            .collect();
        clear(source);
        replies
    });
    assert_eq!(expected[1], expected[0], "the recorded viewpoint's own walk");
    assert_ne!(expected[2], expected[0]);
    own(|| {
        let source = fixture.source("wars-walk");
        run(&source, None, &AtomicBool::new(false), |_| {}).unwrap();
        drop(source);
        // Each query on a source of its own: nothing it walked is kept.
        for (view, expected) in views.into_iter().zip(&expected) {
            for (hours, expected) in sets.iter().zip(expected) {
                let source = fixture.source("wars-walk");
                let before = decoded();
                assert_eq!(
                    &query(&source, "wars", request(view, hours)).unwrap(),
                    expected,
                    "{view:?}, hours {hours:?}"
                );
                let walked = decoded() - before;
                if view == Some("TWO") {
                    // Another viewpoint walks its inputs.
                    assert!(walked > 0, "{view:?}, hours {hours:?}");
                    continue;
                }
                // The last hour reads the published tail alone; any other hour
                // walks fewer inputs than a point's interval from its point.
                if hours.len() == 1 && hours[0] == 125 {
                    assert_eq!(walked, 0, "{view:?}: the last hour walks no input");
                }
                assert!(
                    walked < hours.len() * crate::products::wars::POINT_INPUTS as usize,
                    "{view:?}, hours {hours:?}: {walked} inputs walked"
                );
            }
        }
        let source = fixture.source("wars-walk");
        clear(source);
    });
}

/// An hour the published walk refused may refuse otherwise from a viewpoint, so
/// a viewpoint walks its inputs and answers what walking them answers.
#[test]
fn a_viewpoint_walks_where_the_published_walk_refused_an_hour() {
    use crate::products::wars::walks_decoded_inputs as decoded;
    fn refusing(branch: u64, hour: i32) -> DecodedFrame {
        let mut frame = war_frame(branch, hour);
        if hour == 110 {
            *frame.tables[TableId::WarOwnedRelations.index()] = TableView::Unavailable {
                reason: "test refusal".into(),
            };
        }
        frame
    }
    let replay = scenario_of(refusing, 6, true);
    let mut fixture = Fixture::new(&replay);
    let request = json!({"branchId": "1", "viewpointTag": "ONE", "reads": [{"hour": 125}]});
    let expected = independent::with_test_hourly(|| {
        let source = fixture.source("wars-refused");
        run(&source, None, &AtomicBool::new(false), |_| {}).unwrap();
        let reply = query(&source, "wars", request.clone()).unwrap();
        clear(source);
        reply
    });
    own(|| {
        let source = fixture.source("wars-refused");
        run(&source, None, &AtomicBool::new(false), |_| {}).unwrap();
        drop(source);
        let source = fixture.source("wars-refused");
        let before = decoded();
        assert_eq!(query(&source, "wars", request.clone()).unwrap(), expected);
        assert!(decoded() > before, "the viewpoint walks its inputs");
        clear(source);
    });
}

/// The prepared lane refuses a read needing its hour's recorded frame, so it
/// must refuse before walking: the sampled lane walks it again.
#[test]
fn a_wars_read_of_an_hours_frame_is_refused_on_the_prepared_lane_before_it_walks() {
    use crate::products::wars::walks_decoded_inputs as decoded;
    let replay = scenario(6, true);
    let mut fixture = Fixture::new(&replay);
    let request = json!({"branchId": "1", "viewpointTag": "TWO", "reads": [{"hour": 110}],
        "current": [{"hour": 110}]});
    own(|| {
        let source = fixture.source("wars-read-first");
        run(&source, None, &AtomicBool::new(false), |_| {}).unwrap();
        drop(source);
        // A source of its own holds no frame, so the hour's must be decoded.
        let source = fixture.source("wars-read-first");
        let before = decoded();
        let (_, refused) = dashboard_base::prepared::query::progressive::without_block_decodes(
            || query(&source, "wars", request.clone()),
        );
        assert!(refused, "the hour's frame is read from the recording");
        assert_eq!(decoded(), before, "refused before walking");
        query(&source, "wars", request.clone()).unwrap();
        assert!(decoded() > before, "the sampled read walks");
        clear(source);
    });
}

/// Casualties at an hour come from prepared counters alone: only the hours
/// asked for as `current` read their own frame.
#[test]
fn a_wars_read_of_casualties_alone_answers_on_the_prepared_lane() {
    let replay = scenario(6, true);
    let mut fixture = Fixture::new(&replay);
    let request = json!({"branchId": "1", "viewpointTag": "TWO", "reads": [{"hour": 110}]});
    own(|| {
        let source = fixture.source("wars-casualties-alone");
        run(&source, None, &AtomicBool::new(false), |_| {}).unwrap();
        drop(source);
        // A source of its own holds no frame, so any frame read would be refused.
        let source = fixture.source("wars-casualties-alone");
        let (answer, refused) = dashboard_base::prepared::query::progressive::without_block_decodes(
            || query(&source, "wars", request.clone()),
        );
        assert!(!refused, "no recorded frame is read");
        let answer = answer.unwrap();
        assert!(answer.get("current").is_none());
        assert!(answer["wars"].is_array());
        clear(source);
    });
}

#[test]
fn wars_and_casualties_prepared_on_their_own_refuse_where_hourly_preparation_does() {
    let replay = scenario(6, true);
    let measured = |source: &Source| {
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
    };
    let checkpointed = |source: &Source| {
        let (store, key) = super::super::super::files(source, None).unwrap();
        checkpoint(&store, &key, None).unwrap().unwrap().0
    };
    // At a block's first hour, inside a block, and inside quiet hours.
    for (family, hour, product) in [
        ("wars", 108, "wars"),
        ("wars", 119, "wars"),
        ("casualties", 108, "casualty-pairs"),
        ("casualties", 115, "casualty-pairs"),
        ("casualties", 106, "casualty-pairs"),
    ] {
        let refusals = [(family, hour)];
        let label = format!("{family} refusing at {hour}");
        let mut fixture = Fixture::new(&replay);
        let (hourly, hourly_refused) = independent::with_test_hourly(|| {
            lanes::with_test_refusals(&refusals, || {
                let source = fixture.source("wars-refusal");
                measured(&source);
                let result = (payloads(&source, None), checkpointed(&source).refused);
                clear(source);
                result
            })
        });
        let source = fixture.source("wars-refusal");
        let sequential = prepass::noted(&format!("{family}.sequential"));
        own(|| {
            lanes::with_test_refusals(&refusals, || {
                prepass::with_test_prepass(2, None, || measured(&source))
            })
        });
        assert_eq!(
            prepass::noted(&format!("{family}.sequential")) - sequential,
            1,
            "{label}: the refusing block is observed hour by hour"
        );
        assert_same(&payloads(&source, None), &hourly, &label, false);
        let expected = &hourly_refused[&1].0[family];
        assert_eq!(expected.hour, hour, "{label}");
        let state = checkpointed(&source);
        let own = &state.independent[family].refused[&1].0[family];
        assert_eq!(
            (own.hour, &own.error),
            (expected.hour, &expected.error),
            "{label}"
        );
        let params = |branch: &str| {
            if family == "wars" {
                json!({"branchId": branch})
            } else {
                json!({"branchId": branch, "tags": ["ONE"], "between": false, "window": null})
            }
        };
        let error = query(&source, product, params("1")).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{label}");
        assert_eq!(
            error.to_string(),
            format!("Injected refusal of {family} at hour {hour}"),
            "{label}"
        );
        assert!(
            !query(&source, product, params("2")).unwrap().is_null(),
            "{label}: the other branch answers"
        );
        clear(source);
    }
}

#[test]
fn wars_and_casualties_answer_from_their_own_coverage_while_the_hourly_walk_accepts_nothing() {
    let replay = scenario(6, true);
    let mut fixture = Fixture::new(&replay);
    // What every query answers once hourly preparation prepared everything.
    let expected: Vec<Vec<Value>> = independent::with_test_hourly(|| {
        let source = fixture.source("wars-stalled");
        run(&source, None, &AtomicBool::new(false), |_| {}).unwrap();
        let replies = [1, 2]
            .into_iter()
            .map(|branch| {
                requests(branch)
                    .into_iter()
                    .map(|(label, product, params)| {
                        let reply = query(&source, product, params).unwrap();
                        assert!(!reply.is_null(), "branch {branch}: {label} answers");
                        reply
                    })
                    .collect()
            })
            .collect();
        clear(source);
        replies
    });
    own(|| {
        let source = fixture.source("wars-stalled");
        let flag = AtomicBool::new(false);
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
        for (branch, expected) in [1, 2].into_iter().zip(&expected) {
            for ((label, product, params), expected) in requests(branch).into_iter().zip(expected) {
                assert_eq!(
                    &query(&source, product, params).unwrap(),
                    expected,
                    "branch {branch}: {label} answers from its family's own coverage"
                );
            }
            assert!(
                !query(&source, "subjects", json!({"branchId": branch.to_string()}))
                    .unwrap()
                    .is_null(),
                "branch {branch}: a family of the hourly walk answers its pending prefix"
            );
        }
        // The hourly walk then resumes from its first block and both stay.
        let accepted = run(&source, None, &flag, |_| {}).unwrap();
        assert_eq!(accepted.new_blocks, replay.index.len());
        for id in ["wars", "war-casualties"] {
            let ready = accepted
                .sections
                .iter()
                .find(|section| section.id == id)
                .unwrap();
            assert_eq!(ready.state, State::Ready, "{id}");
        }
        clear(source);
    });
}

#[test]
fn wars_and_casualties_fold_lifetimes_held_and_ended_inside_blocks_from_their_extracts() {
    let replay = scenario_of(joined_frame, 6, true);
    let blocks = replay.index.len();
    let plain = |source: &Source| {
        run(source, None, &AtomicBool::new(false), |_| {}).unwrap();
    };
    // Every block committed on its own and reloaded: each fold continues an
    // observer restored from its continuation.
    let reloaded = |source: &Source| {
        super::super::RELOAD_FLUSHED.with(|flag| flag.set(true));
        let result = run_measured_through(
            source,
            None,
            &AtomicBool::new(false),
            true,
            Duration::ZERO,
            Some(u64::MAX),
            |_| Ok(()),
            |_| {},
        );
        super::super::RELOAD_FLUSHED.with(|flag| flag.set(false));
        result.unwrap();
    };
    let modes: [PrepareMode; 2] = [("run", &plain), ("reloaded", &reloaded)];
    let mut fixture = Fixture::new(&replay);
    for (mode, prepare) in modes {
        let hourly = independent::with_test_hourly(|| {
            let source = fixture.source("wars-joined");
            prepare(&source);
            let payloads = payloads(&source, None);
            clear(source);
            payloads
        });
        for (workers, parallelism) in [(1, 2), (3, 4)] {
            let label = format!("{mode} with {workers} workers");
            let source = fixture.source("wars-joined");
            let before = counts();
            independent::with_test_one_after_another(|| {
                own(|| {
                    prepass::with_test_prepass(workers, None, || {
                        lanes::with_test_lanes(parallelism, None, || prepare(&source))
                    })
                })
            });
            assert_same(&payloads(&source, None), &hourly, &label, true);
            let delta: Vec<usize> = counts().iter().zip(before).map(|(a, b)| a - b).collect();
            assert_eq!(
                delta[..4],
                [0, 0, 0, 0],
                "{label}: every block folded from its extract"
            );
            // Wartime hours, the lifetime's first and last included, are
            // observed by the extracts; the folds observe each block's first
            // hour on the carried state and no other.
            assert_eq!(
                delta[4], blocks,
                "{label}: wars hours observed on the carried observer"
            );
            assert_eq!(
                delta[6], blocks,
                "{label}: casualty hours observed on the carried observer"
            );
            assert!(
                delta[7] > blocks,
                "{label}: the casualty extracts observed the wartime hours"
            );
            clear(source);
        }
    }
}

#[test]
fn casualty_windows_of_a_batch_are_published_when_a_later_block_of_it_refuses() {
    let replay = scenario(6, true);
    // Inside the fourth block of branch 1, whose one batch holds every block.
    let refusals = [("casualties", 115)];
    let mut fixture = Fixture::new(&replay);
    let hourly = independent::with_test_hourly(|| {
        lanes::with_test_refusals(&refusals, || {
            let source = fixture.source("wars-batch-refusal");
            run(&source, None, &AtomicBool::new(false), |_| {}).unwrap();
            let payloads = payloads(&source, None);
            clear(source);
            payloads
        })
    });
    let source = fixture.source("wars-batch-refusal");
    own(|| {
        lanes::with_test_refusals(&refusals, || {
            prepass::with_test_prepass(2, None, || {
                run(&source, None, &AtomicBool::new(false), |_| {})
            })
        })
    })
    .unwrap();
    let published = payloads(&source, None);
    for window in 0..3 {
        assert!(
            published.contains_key(&(casualty_pairs(1), window)),
            "block {window}'s pair window, before the refusing block, is published"
        );
    }
    assert!(
        !published.contains_key(&(casualty_pairs(1), 3)),
        "the refusing block publishes no pair window"
    );
    assert_same(
        &published,
        &hourly,
        "casualties refusing inside a batch",
        false,
    );
    clear(source);
}

/// A branch's casualty pair windows.
fn casualty_pairs(branch: u64) -> String {
    format!("casualty-pairs-{branch}")
}

/// Before wars preparation covers the branch, the wars reply answers its
/// prepared prefix instead of null: lanes walked through the prefix's change
/// inputs, the hours after it pending rather than unobserved, and for a read
/// past it the groups at that hour from its frame alone, whose sides, names
/// and casualty counters are those the complete walk observes there. Native
/// snapshots supply contributions at every requested hour independently of
/// preparation; once complete, nothing is pending and those snapshots stay
/// identical.
#[test]
fn wars_answer_their_prepared_prefix_with_the_groups_at_a_later_hour() {
    let replay = scenario(4, false);
    let mut fixture = Fixture::new(&replay);
    independent::with_test_hourly(|| {
        let source = fixture.source("wars-prefix");
        let flag = AtomicBool::new(false);
        let params = |reads: &[i32]| {
            let reads: Vec<_> = reads.iter().map(|hour| json!({"hour": hour})).collect();
            json!({"branchId": "1", "viewpointTag": null, "reads": reads, "current": reads})
        };
        let cold = query(&source, "wars", params(&[])).unwrap();
        assert!(cold["wars"].as_array().unwrap().is_empty());
        assert!(!cold["pending"].as_array().unwrap().is_empty());
        assert!(cold.get("current").is_none());
        let cold_read = query(&source, "wars", params(&[115])).unwrap();
        assert_eq!(cold_read["current"].as_array().unwrap().len(), 1);
        while coverage(&source, None, 1, &flag)
            .unwrap()
            .is_none_or(|coverage| coverage.last_hour < 103)
        {
            run_measured(&source, None, &flag, true, Duration::ZERO, |_| {}).unwrap();
        }
        let accepted = coverage(&source, None, 1, &flag).unwrap().unwrap();
        assert!(!accepted.complete && accepted.last_hour < 115);
        let prefix = query(&source, "wars", params(&[accepted.last_hour, 115])).unwrap();
        assert_eq!(prefix["coverage"], serde_json::to_value(&accepted).unwrap());
        assert_eq!(
            prefix["approximate"],
            json!([{"firstHour": accepted.first_hour, "lastHour": accepted.last_hour}])
        );
        let pending = crate::prepared::prefix::pending(&source, 1, accepted.last_hour, None);
        assert_eq!(pending.last().map(|span| span.last_hour), Some(117));
        assert_eq!(prefix["pending"], serde_json::to_value(&pending).unwrap());
        for field in ["unobserved", "capitulationsUnobserved"] {
            for span in prefix[field].as_array().unwrap() {
                assert!(
                    span["lastHour"].as_i64().unwrap() <= i64::from(accepted.last_hour),
                    "{prefix}"
                );
            }
        }
        assert!(!prefix["wars"].as_array().unwrap().is_empty());
        assert!(
            prefix["wars"]
                .as_array()
                .unwrap()
                .iter()
                .all(|war| { war["lastHour"].as_i64().unwrap() <= i64::from(accepted.last_hour) })
        );
        let current = prefix["current"].as_array().unwrap();
        assert_eq!(
            current
                .iter()
                .map(|read| read["hour"].as_i64().unwrap())
                .collect::<Vec<_>>(),
            [i64::from(accepted.last_hour), 115],
            "exactly the requested native snapshots: {prefix}"
        );
        assert_eq!(current[1], cold_read["current"][0]);
        let prefix_opponents = if accepted.last_hour < 109 {
            json!(["TWO", "THREE"])
        } else {
            json!(["TWO"])
        };
        for (read, opponents) in current.iter().zip([prefix_opponents, json!(["TWO"])]) {
            assert_eq!(read["available"], true);
            assert_eq!(read["wars"].as_array().unwrap().len(), 1);
            let contributions = read["contributions"].as_array().unwrap();
            assert_eq!(contributions.len(), 1);
            assert_eq!(contributions[0]["viewpointSide"], json!(["ONE"]));
            assert_eq!(contributions[0]["opposingSide"], opponents);
            assert_eq!(contributions[0]["participants"], json!([]));
            assert_eq!(
                contributions[0]["reason"],
                "Recorded content is needed for lend-lease participation"
            );
            let observation = &read["wars"][0]["observations"][0];
            assert_eq!(
                observation["viewpointSide"],
                contributions[0]["viewpointSide"]
            );
            assert_eq!(
                observation["opposingSide"],
                contributions[0]["opposingSide"]
            );
        }
        let at = current[1]["wars"].as_array().unwrap();
        assert!(!at.is_empty());
        assert!(
            at.iter()
                .all(|war| war["id"].as_str().unwrap().starts_with("@115:"))
        );
        run(&source, None, &flag, |_| {}).unwrap();
        let complete = query(&source, "wars", params(&[accepted.last_hour, 115])).unwrap();
        assert!(complete.get("pending").is_none());
        assert_eq!(complete["current"], prefix["current"]);
        let wars = complete["wars"].as_array().unwrap();
        for war in at {
            let observed = &war["observations"][0];
            let same = wars
                .iter()
                .find(|exact| {
                    exact["observations"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|item| {
                            item["firstHour"].as_i64().unwrap() <= 115
                                && 115 <= item["lastHour"].as_i64().unwrap()
                                && item["viewpointSide"] == observed["viewpointSide"]
                                && item["opposingSide"] == observed["opposingSide"]
                                && item["name"] == observed["name"]
                        })
                })
                .unwrap_or_else(|| panic!("{war} is not observed at 115: {complete}"));
            let losses = same["casualties"]
                .as_array()
                .unwrap()
                .iter()
                .find(|run| run[0].as_i64().unwrap() <= 115 && 115 <= run[1].as_i64().unwrap())
                .unwrap();
            assert_eq!(
                &war["casualties"][0].as_array().unwrap()[2..],
                &losses.as_array().unwrap()[2..]
            );
        }
        clear(source);
    });
}
