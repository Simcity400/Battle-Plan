use super::*;
use crate::test_fixtures::continuation_records;
use replay::{publication::replace_file, rows::*};
use std::path::PathBuf;

struct Fixture {
    root: PathBuf,
    path: PathBuf,
    stores: Vec<Arc<store::Store>>,
}
impl Fixture {
    fn new(replay: &Replay) -> Self {
        let root =
            std::env::temp_dir().join(format!("scoreboard-prepared-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("recording.battleplan");
        std::fs::write(&path, replay.bytes()).unwrap();
        Self {
            root,
            path,
            stores: Vec::new(),
        }
    }
    fn source(&mut self) -> Source {
        let source = Source::open(
            crate::SourceKey {
                source_id: "scoreboard-prepared".into(),
                revision: crate::protocol::replay_revision(&self.path).unwrap(),
            },
            self.path.clone(),
            &AtomicBool::new(false),
        )
        .unwrap();
        self.stores
            .push(crate::prepared::files(&source, None).unwrap().0);
        source
    }
    fn replace(&self, replay: &Replay) {
        let candidate = self.root.join("next");
        std::fs::write(&candidate, replay.bytes()).unwrap();
        replace_file(&candidate, &self.path).unwrap();
    }
    fn run(&mut self, expected_blocks: usize) -> Source {
        let source = self.source();
        assert_eq!(
            crate::prepared::pipeline::run(&source, None, &AtomicBool::new(false), |_| {})
                .unwrap()
                .new_blocks,
            expected_blocks
        );
        source
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        for store in &self.stores {
            let _ = store.clear();
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
fn query(
    source: &Source,
    selection: &ScoreboardParams,
    hour: i32,
    index: bool,
    bars: bool,
    sort: ScoreboardColumnKey,
    descending: bool,
) -> Value {
    let (store, key) = crate::prepared::files(source, None).unwrap();
    prepared::query(source, json!({"selection":selection,"hour":hour,"index":index,"bars":bars,"sort":sort,"descending":descending}),
        &LoadContext { content: None }, &store, &key, &AtomicBool::new(false)).unwrap()
}
fn at(reply: &Value, hour: i32) -> Value {
    let mut rows = BTreeMap::new();
    for row in reply["series"].as_array().unwrap() {
        let covers = |run: &Value| {
            run["firstHour"].as_i64().unwrap() <= i64::from(hour)
                && run["lastHour"].as_i64().unwrap() >= i64::from(hour)
        };
        if row["membership"]
            .as_array()
            .is_some_and(|spans| !spans.iter().any(covers))
        {
            continue;
        }
        let Some(identity) = row["identities"]
            .as_array()
            .unwrap()
            .iter()
            .find(|identity| covers(identity))
        else {
            continue;
        };
        let mut columns = BTreeMap::new();
        for column in row["columns"].as_array().unwrap() {
            let value = column["runs"]
                .as_array()
                .unwrap()
                .iter()
                .find(|run| covers(run));
            let read = column["readRuns"]
                .as_array()
                .unwrap()
                .iter()
                .find(|run| covers(run));
            let read_hour = read.map(|read| {
                if read["hourly"].as_bool().unwrap() {
                    hour
                } else {
                    read["readHour"].as_i64().unwrap() as i32
                }
            });
            columns.insert(
                column["key"].as_str().unwrap(),
                (
                    value.and_then(|run| run["amount"].as_str()),
                    read_hour,
                    value.is_some_and(|run| run["atLeast"].as_bool() == Some(true)),
                ),
            );
        }
        rows.insert(
            row["tag"].as_str().unwrap(),
            (
                identity["slot"].as_u64().unwrap(),
                identity["countryId"].as_u64().unwrap(),
                columns,
            ),
        );
    }
    serde_json::to_value(rows).unwrap()
}
fn parity(
    source: &Source,
    selection: &ScoreboardParams,
    hour: i32,
    sort: ScoreboardColumnKey,
    descending: bool,
) {
    let index = query(source, selection, hour, true, true, sort, descending);
    assert!(!index.is_null());
    let selected = index["data"]["series"].as_array().unwrap();
    assert!(
        selected
            .iter()
            .flat_map(|row| row["columns"].as_array().unwrap())
            .all(|column| column["readRuns"].as_array().unwrap().is_empty()),
        "the quantity index must not fabricate supplying dates"
    );
    let page_selection = ScoreboardParams {
        tags: selected
            .iter()
            .map(|row| row["tag"].as_str().unwrap().to_owned())
            .collect(),
        top: None,
        ..selection.clone()
    };
    let page = query(
        source,
        &page_selection,
        hour,
        false,
        false,
        sort,
        descending,
    );
    assert_eq!(
        source
            .state::<country_factories::Scans>()
            .lock()
            .unwrap()
            .runs,
        0,
        "prepared composition must never call the factory oracle"
    );
    // Keep the deliberate oracle scan out of the prepared request's cache,
    // including when later hours reuse this source.
    let oracle_source = Source::from_replay(
        source.key().clone(),
        source.path().to_path_buf(),
        source.replay(),
        &AtomicBool::new(false),
    )
    .unwrap();
    let oracle = Scoreboard::load(
        &oracle_source,
        selection,
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        oracle_source
            .state::<country_factories::Scans>()
            .lock()
            .unwrap()
            .runs,
        1,
        "only the independent parity oracle scans factory history"
    );
    let oracle = serde_json::to_value(oracle).unwrap();
    assert_eq!(
        at(&page["data"], hour),
        at(&oracle, hour),
        "hour {hour}, sort {sort:?}, descending={descending}"
    );
    for key in &selection.columns {
        let name = serde_json::to_value(key).unwrap();
        let maximum = at(&oracle, hour)
            .as_object()
            .unwrap()
            .values()
            .filter_map(|row| row[2][name.as_str().unwrap()][0].as_str())
            .map(|amount| amount.parse::<i128>().unwrap())
            .max()
            .unwrap_or(0)
            .max(0);
        assert_eq!(index["maxima"][name.as_str().unwrap()], maximum.to_string());
    }
}
fn all_frame(hour: i32) -> DecodedFrame {
    let mut frame = multi_measure_frame(
        hour,
        if hour < 102 { 100 } else { 102 },
        Some(if hour < 102 { 7 } else { 11 }),
    );
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *frame.tables[TableId::Countries.index()]
    {
        rows[0].political_power_raw = Some(i64::MIN + i64::from(hour));
        rows.extend((2..=3).map(|id| CountryRow {
            id: 9000 + id,
            canonical_slot: id + 6,
            tag: format!("opponent-{id}").into_bytes(),
            political_power_raw: (id == 3).then_some(i64::MAX),
            ..Default::default()
        }));
        if hour >= 106 {
            rows[0].tag = b"renamed native holder".to_vec();
        }
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *frame.tables[TableId::CountryLinks.index()]
    {
        rows.extend([
            CountryLinkRow {
                id: 12,
                canonical_slot: 8,
            },
            CountryLinkRow {
                id: 13,
                canonical_slot: 9,
            },
            CountryLinkRow {
                id: 9002,
                canonical_slot: 8,
            },
            CountryLinkRow {
                id: 9003,
                canonical_slot: 9,
            },
        ]);
        rows.sort_by_key(|row| row.id);
    }
    *frame.tables[TableId::CountryFactories.index()] = if hour == 101 {
        TableView::Unavailable {
            reason: "native factory counter unavailable".into(),
        }
    } else {
        TableView::Complete(TableRows::CountryFactories(vec![CountryFactoriesRow {
            country: 11,
            civilian_total_raw: 1_100_000,
            civilian_damaged_raw: 100_000,
            consumer_goods: 1,
            civilian_unavailable: 1,
            ..Default::default()
        }]))
    };
    *frame.tables[TableId::CountryCounterInputs.index()] =
        TableView::Complete(TableRows::CountryCounterInputs(vec![
            CountryCounterInputRow {
                country: 11,
                owned_state_count: match hour {
                    106 => -3,
                    107 => 0,
                    _ => 3,
                },
                ..Default::default()
            },
        ]));
    if hour == 101
        && let TableView::Complete(TableRows::CountryCounterInputs(rows)) =
            &mut *frame.tables[TableId::CountryCounterInputs.index()]
    {
        // Distinct native keys still conflict after both links resolve to slot 7.
        rows.push(CountryCounterInputRow {
            country: 9001,
            owned_state_count: 9,
            ..Default::default()
        });
    }
    *frame.tables[TableId::AirWings.index()] =
        TableView::Complete(TableRows::AirWings(vec![AirWingRow {
            id: 1,
            owner: 11,
            country_override: 12,
            planes: 5,
            ..Default::default()
        }]));
    *frame.tables[TableId::WingEquipment.index()] =
        TableView::Complete(TableRows::WingEquipment(vec![WingEquipmentRow {
            wing: 1,
            ordinal: 0,
            variant: 10,
            amount_raw: 500_000,
        }]));
    *frame.tables[TableId::TaskForces.index()] =
        TableView::Complete(TableRows::TaskForces(vec![TaskForceRow {
            id: 1,
            roster_owner: 11,
            logical_owner: 12,
            ..Default::default()
        }]));
    *frame.tables[TableId::Ships.index()] = TableView::Complete(TableRows::Ships(vec![ShipRow {
        id: 1,
        task_force: 1,
        ..Default::default()
    }]));
    *frame.tables[TableId::DivisionCensus.index()] =
        TableView::Complete(TableRows::DivisionCensus(vec![
            DivisionCensusRow {
                country: 11,
                grouped: 1,
                ungrouped: 0,
            },
            DivisionCensusRow {
                country: 12,
                grouped: 0,
                ungrouped: 0,
            },
            DivisionCensusRow {
                country: 13,
                grouped: 0,
                ungrouped: 0,
            },
        ]));
    *frame.tables[TableId::Divisions.index()] =
        TableView::Complete(TableRows::Divisions(vec![DivisionRow {
            id: 1,
            roster_owner: 12,
            logical_owner: 12,
            expeditionary_owner: 11,
            ..Default::default()
        }]));
    *frame.tables[TableId::DivisionLandInputs.index()] =
        TableView::Complete(TableRows::DivisionLandInputs(if hour == 101 {
            Vec::new()
        } else {
            vec![DivisionLandInputsRow {
                division: 1,
                manpower: vec![
                    DivisionManpowerContributor {
                        country: 12,
                        amount: i32::MAX,
                    },
                    DivisionManpowerContributor {
                        country: 11,
                        amount: 1,
                    },
                ],
                ..Default::default()
            }]
        }));
    *frame.tables[TableId::WarRelations.index()] = TableView::Complete(TableRows::WarRelations(
        (12..=13)
            .map(|second_country| WarRelationRow {
                first_country: 11,
                second_country,
                start_hour: 90,
                first_casualties: i64::MAX,
                second_casualties: if hour == 103 { -7 } else { 5 },
                ..Default::default()
            })
            .collect(),
    ));
    for id in [
        TableId::AirWings,
        TableId::WingEquipment,
        TableId::TaskForces,
        TableId::Ships,
        TableId::DivisionCensus,
        TableId::Divisions,
        TableId::DivisionLandInputs,
        TableId::CountryFactories,
        TableId::CountryCounterInputs,
        TableId::WarRelations,
        TableId::Nukes,
    ] {
        frame.metrics.table_ns[id.index()] = 1;
    }
    if hour == 103 {
        *frame.tables[TableId::CountryCounterInputs.index()] = TableView::Unavailable {
            reason: "native counter unavailable".into(),
        };
        *frame.tables[TableId::AirWings.index()] = TableView::Unavailable {
            reason: "air census unavailable".into(),
        };
    }
    if hour >= 108 {
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frame.tables[TableId::Countries.index()]
        {
            rows.retain(|row| row.id != 9003);
            rows[1].tag = b"GER".to_vec();
            if hour == 109 {
                rows[1].id = 9999;
            }
        }
        if hour == 109 {
            if let TableView::Complete(TableRows::CountryLinks(rows)) =
                &mut *frame.tables[TableId::CountryLinks.index()]
            {
                rows.push(CountryLinkRow {
                    id: 9999,
                    canonical_slot: 8,
                });
                rows.sort_by_key(|row| row.id);
            }
            for table in [TableId::DivisionCensus, TableId::Divisions] {
                *frame.tables[table.index()] = TableView::Unavailable {
                    reason: "no raw army run".into(),
                };
            }
        }
    }
    frame
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn scoreboard_prepared_composes_native_cells_after_two_commits_resume_append_gaps_and_renames() {
    let first = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, vec![all_frame(100), all_frame(101)])],
    )
    .unwrap();
    let mut fixture = Fixture::new(&first);
    let columns = ScoreboardColumnKey::ALL.to_vec();
    let selection = request(
        columns.clone(),
        vec!["GER".into(), "opponent-2".into(), "opponent-3".into()],
        None,
    );
    let unopened = fixture.source();
    assert!(
        query(
            &unopened,
            &selection,
            100,
            true,
            false,
            ScoreboardColumnKey::Factories,
            true
        )
        .is_null()
    );
    let source = fixture.run(1);
    parity(
        &source,
        &selection,
        100,
        ScoreboardColumnKey::Factories,
        true,
    );
    parity(
        &source,
        &selection,
        101,
        ScoreboardColumnKey::Factories,
        true,
    );
    let ambiguous = query(
        &source,
        &selection,
        101,
        false,
        false,
        ScoreboardColumnKey::Territory,
        true,
    );
    assert_eq!(
        at(&ambiguous["data"], 101)["GER"][2]["territory"],
        json!([null, null, false]),
        "conflicting native aliases must leave the owned-state cell unknown"
    );
    for column in ["factories", "factoriesBuilt", "factoriesTopbar"] {
        assert_eq!(
            at(&ambiguous["data"], 101)["GER"][2][column],
            json!([null, null, false]),
            "an unavailable factory read must leave {column} unknown"
        );
    }
    let second = first
        .append(&[continuation_records(
            &first,
            1,
            vec![all_frame(102), all_frame(103)],
        )])
        .unwrap();
    fixture.replace(&second);
    fixture.run(1);
    for hour in 100..=103 {
        let clean = fixture.source();
        parity(
            &clean,
            &selection,
            hour,
            ScoreboardColumnKey::Factories,
            true,
        );
    }
    let resumed = fixture.run(0);
    let (store, key) = crate::prepared::files(&resumed, None).unwrap();
    assert!(store.range_count(&key, "army-coverage-1").unwrap().unwrap() >= 2);
    let grown = second
        .append(&[continuation_records(
            &second,
            1,
            vec![all_frame(106), all_frame(107)],
        )])
        .unwrap();
    fixture.replace(&grown);
    let appended = fixture.run(1);
    for hour in [104, 105, 106, 107] {
        let clean = fixture.source();
        parity(
            &clean,
            &selection,
            hour,
            ScoreboardColumnKey::Factories,
            true,
        );
    }
    for descending in [true, false] {
        for sort in [
            ScoreboardColumnKey::PoliticalPower,
            ScoreboardColumnKey::Territory,
        ] {
            let top = request(
                columns.clone(),
                Vec::new(),
                Some(ScoreboardTop {
                    count: 2,
                    sort,
                    descending,
                }),
            );
            let clean = fixture.source();
            parity(&clean, &top, 107, sort, descending);
        }
    }
    let index = query(
        &appended,
        &selection,
        107,
        true,
        true,
        ScoreboardColumnKey::Factories,
        true,
    );
    assert!(
        index["data"]["series"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["tag"] == "renamed native holder")
    );
    let reused = grown
        .append(&[continuation_records(
            &grown,
            1,
            vec![all_frame(108), all_frame(109)],
        )])
        .unwrap();
    fixture.replace(&reused);
    fixture.run(1);
    let pick = request(columns, vec!["GER".into()], None);
    for hour in [108, 109] {
        let clean = fixture.source();
        parity(&clean, &pick, hour, ScoreboardColumnKey::Factories, true);
    }
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn scoreboard_prepared_top_and_mounted_page_keep_5000_countries_and_unknown_last_in_both_directions()
 {
    let mut first_frame = countries(100, 5000);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *first_frame.tables[TableId::Countries.index()]
    {
        rows[0].political_power_raw = Some(i64::MIN);
        rows[4999].political_power_raw = Some(i64::MAX);
    }
    let first =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![first_frame.clone()])]).unwrap();
    let mut fixture = Fixture::new(&first);
    fixture.run(1);
    first_frame.hour = 101;
    let second = first
        .append(&[continuation_records(&first, 1, vec![first_frame])])
        .unwrap();
    fixture.replace(&second);
    fixture.run(1);
    for descending in [true, false] {
        let source = fixture.source();
        let selection = request(
            vec![
                ScoreboardColumnKey::PoliticalPower,
                ScoreboardColumnKey::Factories,
                ScoreboardColumnKey::Territory,
            ],
            Vec::new(),
            Some(ScoreboardTop {
                count: 5000,
                sort: ScoreboardColumnKey::PoliticalPower,
                descending,
            }),
        );
        let mut first_page_selection = selection.clone();
        first_page_selection.top.as_mut().unwrap().count = 10;
        let started = std::time::Instant::now();
        let first_page_index = query(
            &source,
            &first_page_selection,
            101,
            true,
            true,
            ScoreboardColumnKey::PoliticalPower,
            descending,
        );
        let first_page_tags = first_page_index["data"]["series"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["tag"].as_str().unwrap().to_owned())
            .collect();
        let first_page_cells = query(
            &source,
            &request(selection.columns.clone(), first_page_tags, None),
            101,
            false,
            false,
            ScoreboardColumnKey::PoliticalPower,
            descending,
        );
        eprintln!(
            "scoreboard top-10 first index + cells at 5000 countries, direction={descending}: {:?}",
            started.elapsed()
        );
        assert_eq!(
            first_page_cells["data"]["series"].as_array().unwrap().len(),
            10
        );
        let started = std::time::Instant::now();
        let index = query(
            &source,
            &selection,
            101,
            true,
            true,
            ScoreboardColumnKey::PoliticalPower,
            descending,
        );
        eprintln!(
            "scoreboard 5000-country moment index, direction={descending}: {:?}",
            started.elapsed()
        );
        let rows = index["data"]["series"].as_array().unwrap();
        assert_eq!(rows.len(), 5000);
        assert_eq!(rows.last().unwrap()["tag"], "country-00002");
        assert_eq!(
            rows[0]["tag"],
            if descending {
                "country-05000"
            } else {
                "country-00001"
            }
        );
        let mounted = request(
            selection.columns.clone(),
            rows[10..22]
                .iter()
                .map(|row| row["tag"].as_str().unwrap().to_owned())
                .collect(),
            None,
        );
        let page = query(
            &source,
            &mounted,
            101,
            false,
            false,
            ScoreboardColumnKey::PoliticalPower,
            descending,
        );
        assert_eq!(page["data"]["series"].as_array().unwrap().len(), 12);
        assert_eq!(
            source
                .state::<country_factories::Scans>()
                .lock()
                .unwrap()
                .runs,
            0
        );
        let oracle = Scoreboard::load(
            &source,
            &mounted,
            &LoadContext { content: None },
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(
            at(&page["data"], 101),
            at(&serde_json::to_value(oracle).unwrap(), 101)
        );
    }
}

#[test]
fn scoreboard_prepared_registry_routes_without_preparation_or_an_oracle_scan() {
    let replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![countries(100, 3)])]).unwrap();
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source();
    let selection = request(Vec::new(), Vec::new(), None);
    let request = crate::transport::ProductRequest {
        source_id: source.key().source_id.clone(),
        revision: source.key().revision.clone(),
        product: "scoreboard".into(),
        params: json!({"selection": selection, "hour":0, "sort":"factories", "descending":true, "index":true,"bars":false}),
    };
    let routed: Value = serde_json::from_str(
        &crate::prepared::query::query(&source, &request, None, &AtomicBool::new(false)).unwrap(),
    )
    .unwrap();
    let oracle = Scoreboard::load(
        &source,
        &selection,
        &LoadContext { content: None },
        &AtomicBool::new(true),
    )
    .unwrap();
    assert_eq!(
        routed["data"]["data"],
        serde_json::to_value(oracle).unwrap()
    );
    assert_eq!(
        source
            .state::<country_factories::Scans>()
            .lock()
            .unwrap()
            .runs,
        0
    );
}
