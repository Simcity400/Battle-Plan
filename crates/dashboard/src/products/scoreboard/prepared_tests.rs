use super::*;
use crate::native_fixtures::{Recorded, UNIT_HOURS, streams};
use crate::prepared::pipeline;
use crate::test_fixtures::continuation_records;
use replay_data::rows::*;
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

// The tables the native recording holds: every one the scoreboard's suppliers read.
streams!(
    Campaign,
    Countries,
    CountryLinks,
    CountryAppearance,
    CountryCounterInputs,
    CountryExperienceGains,
    CountryCommandPower,
    CountryStabilityInputs,
    SliceReads,
    CountryFactories,
    States,
    Provinces,
    Nukes,
    LexicalTokens,
    VariantDefinitions,
    VariantChanges,
    VariantModules,
    VariantUpgrades,
    ProductionLines,
    ProductionLineOutput,
    RocketProductionOutput,
    AirWings,
    WingEquipment,
    TaskForces,
    Ships,
    ShipModels,
    DivisionCensus,
    Divisions,
    DivisionTemplateIcons,
    DivisionLandInputs,
    SentVolunteers,
    VolunteerHosts,
    WarRelations,
    Politics,
    PoliticalParties,
    Autonomy,
);

const SOURCE: &str = "scoreboard-prepared";
/// The first hour of each part the recording grows by: its first run's first unit and its
/// second, then, after the two hours from `GAP` no run holds, its second run's two.
const FIRST: i32 = 100;
const SECOND: i32 = FIRST + UNIT_HOURS;
const GAP: i32 = SECOND + UNIT_HOURS;
const THIRD: i32 = GAP + 2;
const FOURTH: i32 = THIRD + UNIT_HOURS;

fn hours(from: i32, to: i32) -> Vec<DecodedFrame> {
    (from..to).map(all_frame).collect()
}

/// A session of the recording as it is now, prepared: hours were left to prepare, and none is
/// once it is.
fn prepare(recorded: &Recorded) -> Source {
    let (source, flag) = (recorded.source(SOURCE), AtomicBool::new(false));
    assert!(
        pipeline::restore(&source, None, &flag).unwrap().is_none(),
        "hours are left to prepare"
    );
    pipeline::run(&source, None, &flag, |_| {}).unwrap();
    assert!(
        pipeline::restore(&source, None, &flag).unwrap().is_some(),
        "no hour is left to prepare"
    );
    source
}

fn all_frame(hour: i32) -> DecodedFrame {
    let first = hour < SECOND;
    // Its hour 100 gives the first capture's production, 102 a later capture's.
    let mut frame = multi_measure_frame(
        if first { 100 } else { 102 },
        if first { FIRST } else { SECOND },
        Some(if first { 7 } else { 11 }),
    );
    frame.hour = hour;
    for table in [
        TableId::RocketProductionOutput,
        TableId::ProductionLines,
        TableId::VariantDefinitions,
    ] {
        frame.metrics.table_ns[table.index()] = u64::from(hour == FIRST || hour == SECOND);
    }
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
        if hour >= THIRD {
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
    *frame.tables[TableId::CountryFactories.index()] = if hour == FIRST + 1 {
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
                owned_state_count: if hour == THIRD {
                    -3
                } else if hour == THIRD + 1 {
                    0
                } else {
                    3
                },
                ..Default::default()
            },
        ]));
    if hour == FIRST + 1
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
        TableView::Complete(TableRows::DivisionLandInputs(if hour == FIRST + 1 {
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
                second_casualties: if hour == SECOND + 1 { -7 } else { 5 },
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
    if hour == SECOND + 1 {
        *frame.tables[TableId::CountryCounterInputs.index()] = TableView::Unavailable {
            reason: "native counter unavailable".into(),
        };
        *frame.tables[TableId::AirWings.index()] = TableView::Unavailable {
            reason: "air census unavailable".into(),
        };
    }
    if hour >= FOURTH {
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frame.tables[TableId::Countries.index()]
        {
            rows.retain(|row| row.id != 9003);
            rows[1].tag = b"GER".to_vec();
            if hour > FOURTH {
                rows[1].id = 9999;
            }
        }
        if hour > FOURTH {
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
fn scoreboard_prepared_composes_native_cells_after_two_commits_resume_append_gaps_and_renames() {
    for tables in [
        crate::products::air_force::facts::TABLES,
        crate::products::army_size::facts::TABLES,
        crate::products::war_casualties::facts::TABLES,
        crate::products::country_factories::facts::TABLES,
        crate::products::country_balances::facts::TABLES,
        crate::products::navy::facts::TABLES,
        crate::products::country_nukes::facts::TABLES,
        crate::prepared_production::TABLES,
        crate::products::country_stability::facts::TABLES,
        crate::products::territory::facts::TABLES,
    ] {
        assert!(
            tables.iter().all(|table| STREAMS.contains(table)),
            "the native recording holds every table the scoreboard's suppliers read: {tables:?}"
        );
    }
    let recorded = Recorded::new();
    let first = Replay::from_records(CAMPAIGN.into(), &[records(1, hours(FIRST, SECOND))]).unwrap();
    recorded.record(&first);
    let columns = ScoreboardColumnKey::ALL.to_vec();
    let selection = request(
        columns.clone(),
        vec!["GER".into(), "opponent-2".into(), "opponent-3".into()],
        None,
    );
    let unopened = recorded.source(SOURCE);
    // Unprepared, only columns one frame proves answer; cumulative ones wait.
    assert!(
        query(
            &unopened,
            &selection,
            FIRST,
            true,
            false,
            ScoreboardColumnKey::PoliticalPower,
            true
        )
        .is_object()
    );
    assert!(
        query(
            &unopened,
            &selection,
            FIRST,
            false,
            false,
            ScoreboardColumnKey::Factories,
            true
        )
        .is_object(),
        "a page answers while its suppliers are preparing"
    );
    let pending = query(
        &unopened,
        &selection,
        FIRST,
        true,
        false,
        ScoreboardColumnKey::Factories,
        true,
    );
    assert_eq!(pending["coverage"]["lastHour"], FIRST - 1, "{pending}");
    assert!(
        pending["preparing"]
            .as_array()
            .is_some_and(|preparing| preparing.contains(&json!("factories"))),
        "an index sorted by unprepared factories is pending on them: {pending}"
    );
    let source = prepare(&recorded);
    parity(
        &source,
        &selection,
        FIRST,
        ScoreboardColumnKey::Factories,
        true,
    );
    parity(
        &source,
        &selection,
        FIRST + 1,
        ScoreboardColumnKey::Factories,
        true,
    );
    let ambiguous = query(
        &source,
        &selection,
        FIRST + 1,
        false,
        false,
        ScoreboardColumnKey::Territory,
        true,
    );
    assert_eq!(
        at(&ambiguous["data"], FIRST + 1)["GER"][2]["territory"],
        json!([null, null, false]),
        "conflicting native aliases must leave the owned-state cell unknown"
    );
    for column in ["factories", "factoriesBuilt", "factoriesTopbar"] {
        assert_eq!(
            at(&ambiguous["data"], FIRST + 1)["GER"][2][column],
            json!([null, null, false]),
            "an unavailable factory read must leave {column} unknown"
        );
    }
    // The run gains a unit, which a preparation takes up from where the last left off.
    let second = first
        .append(&[continuation_records(&first, 1, hours(SECOND, GAP))])
        .unwrap();
    recorded.record(&second);
    prepare(&recorded);
    for hour in [FIRST, FIRST + 1, SECOND - 1, SECOND, SECOND + 1] {
        let clean = recorded.source(SOURCE);
        parity(
            &clean,
            &selection,
            hour,
            ScoreboardColumnKey::Factories,
            true,
        );
    }
    // Prepared again with nothing new, the store publishes nothing.
    let again = recorded.source(SOURCE);
    let flag = AtomicBool::new(false);
    assert!(pipeline::restore(&again, None, &flag).unwrap().is_some());
    let store = crate::prepared::files(&again, None).unwrap().0;
    let held = store.manifest_identity().unwrap();
    pipeline::run(&again, None, &flag, |_| {}).unwrap();
    assert_eq!(store.manifest_identity().unwrap(), held);
    // A later run of the branch, after two hours no run holds.
    let grown = second
        .append(&[continuation_records(&second, 1, hours(THIRD, FOURTH))])
        .unwrap();
    recorded.record(&grown);
    let appended = prepare(&recorded);
    for hour in [GAP, GAP + 1, THIRD, THIRD + 1] {
        let clean = recorded.source(SOURCE);
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
            let clean = recorded.source(SOURCE);
            parity(&clean, &top, THIRD + 1, sort, descending);
        }
    }
    let index = query(
        &appended,
        &selection,
        THIRD + 1,
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
    // That run gains a unit, in which another country takes up a tag and a new id.
    let reused = grown
        .append(&[continuation_records(
            &grown,
            1,
            hours(FOURTH, FOURTH + UNIT_HOURS),
        )])
        .unwrap();
    recorded.record(&reused);
    prepare(&recorded);
    let pick = request(columns, vec!["GER".into()], None);
    for hour in [FOURTH, FOURTH + 1] {
        let clean = recorded.source(SOURCE);
        parity(&clean, &pick, hour, ScoreboardColumnKey::Factories, true);
    }
}

#[test]
fn scoreboard_prepared_top_and_mounted_page_keep_every_country_and_unknown_last_in_both_directions()
{
    let frame = |hour: i32| {
        let mut frame = countries(hour, 24);
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frame.tables[TableId::Countries.index()]
        {
            rows[0].political_power_raw = Some(i64::MIN);
            rows[23].political_power_raw = Some(i64::MAX);
        }
        frame
    };
    let recorded = Recorded::new();
    let first = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, (FIRST..SECOND).map(frame).collect())],
    )
    .unwrap();
    recorded.record(&first);
    prepare(&recorded);
    let second = first
        .append(&[continuation_records(
            &first,
            1,
            (SECOND..GAP).map(frame).collect(),
        )])
        .unwrap();
    recorded.record(&second);
    // Political power is the balances family's, prepared natively beside the other suppliers.
    prepare(&recorded);
    for descending in [true, false] {
        let source = recorded.source(SOURCE);
        let selection = request(
            vec![
                ScoreboardColumnKey::PoliticalPower,
                ScoreboardColumnKey::Factories,
                ScoreboardColumnKey::Territory,
            ],
            Vec::new(),
            Some(ScoreboardTop {
                count: 24,
                sort: ScoreboardColumnKey::PoliticalPower,
                descending,
            }),
        );
        let mut first_page_selection = selection.clone();
        first_page_selection.top.as_mut().unwrap().count = 10;
        let first_page_index = query(
            &source,
            &first_page_selection,
            SECOND,
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
            SECOND,
            false,
            false,
            ScoreboardColumnKey::PoliticalPower,
            descending,
        );
        assert_eq!(
            first_page_cells["data"]["series"].as_array().unwrap().len(),
            10
        );
        let index = query(
            &source,
            &selection,
            SECOND,
            true,
            true,
            ScoreboardColumnKey::PoliticalPower,
            descending,
        );
        let rows = index["data"]["series"].as_array().unwrap();
        assert_eq!(rows.len(), 24);
        assert_eq!(rows.last().unwrap()["tag"], "country-00002");
        assert_eq!(
            rows[0]["tag"],
            if descending {
                "country-00024"
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
            SECOND,
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
            at(&page["data"], SECOND),
            at(&serde_json::to_value(oracle).unwrap(), SECOND)
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

#[test]
fn default_scoreboard_cells_mark_every_unprepared_supplier_pending() {
    use ScoreboardColumnKey::*;
    let replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![all_frame(100)])]).unwrap();
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source();
    let columns = vec![
        Factories,
        ArmySize,
        Territory,
        CasualtiesDealt,
        CasualtiesTaken,
        Planes,
        Ships,
    ];
    let selected = request(columns, vec!["GER".into()], None);
    let page = query(&source, &selected, 100, false, false, Factories, true);
    assert_eq!(page["coverage"]["firstHour"], 100);
    assert_eq!(page["coverage"]["lastHour"], 99);
    assert_eq!(page["pending"], json!([{"firstHour":100,"lastHour":100}]));
    let preparing = page["preparing"].as_array().unwrap();
    // Factory counts are read from the factories family's publication alone, never sampled.
    for column in ["casualtiesTaken", "factories"] {
        assert!(preparing.contains(&json!(column)), "{column}: {page}");
    }
    assert!(
        page["data"]["series"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["columns"]
                .as_array()
                .unwrap()
                .iter()
                .all(|column| column["runs"].as_array().unwrap().is_empty()))
    );
}
