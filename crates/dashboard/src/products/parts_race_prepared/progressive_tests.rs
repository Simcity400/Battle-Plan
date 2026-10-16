use super::*;
use crate::test_fixtures::{CAMPAIGN, continuation_records, records, scale_frame, scale_tag};
use replay::rows::*;
use std::{path::PathBuf, sync::Arc};

pub(crate) fn frame(hour: i32, count: u32) -> DecodedFrame {
    let mut frame = scale_frame(1, hour, count);
    if hour >= 24
        && count > 0
        && let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frame.tables[TableId::Countries.index()]
    {
        rows[0].tag = b"renamed / first".to_vec();
    }
    *frame.tables[TableId::DivisionCensus.index()] =
        TableView::Complete(TableRows::DivisionCensus(
            (0..count)
                .map(|index| DivisionCensusRow {
                    country: 100_000 + index,
                    grouped: 1,
                    ..Default::default()
                })
                .collect(),
        ));
    *frame.tables[TableId::Divisions.index()] = TableView::Complete(TableRows::Divisions(
        (0..count)
            .map(|index| DivisionRow {
                id: u64::from(index) + 1,
                roster_owner: 100_000 + index,
                logical_owner: 100_000 + index,
                expeditionary_owner: u32::MAX,
                template: 1,
                // Both direct samples and replay queries use representable HP steps.
                hit_points: 100_000 + i64::from(hour) * DivisionRow::HIT_POINTS_STEP.raw(),
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::DivisionTemplateIcons.index()] =
        TableView::Complete(TableRows::DivisionTemplateIcons(vec![
            DivisionTemplateIconRow {
                template: 1,
                subunits: vec![TemplateSubunit {
                    token: 1,
                    count: 3,
                    local: 0,
                }],
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::DivisionLandInputs.index()] =
        TableView::Complete(TableRows::DivisionLandInputs(
            (0..count)
                .filter(|index| hour != 7 || *index != 0)
                .map(|index| DivisionLandInputsRow {
                    division: u64::from(index) + 1,
                    manpower: vec![DivisionManpowerContributor {
                        country: 100_000 + index,
                        amount: hour,
                    }],
                    ..Default::default()
                })
                .collect(),
        ));
    *frame.tables[TableId::DivisionStatus.index()] =
        TableView::Complete(TableRows::DivisionStatus(
            (0..count)
                .map(|index| DivisionStatusRow {
                    division: u64::from(index) + 1,
                    supply_ratio: if hour % 2 == 0 { 0 } else { 100_000 },
                    supply_grace_maximum: Recorded::Value(1_000_000),
                    supply_grace_remaining: Recorded::Value(0),
                    ..Default::default()
                })
                .collect(),
        ));
    *frame.tables[TableId::AirWings.index()] = TableView::Complete(TableRows::AirWings(
        (0..count)
            .map(|index| AirWingRow {
                id: u64::from(index) + 1,
                owner: 100_000 + index,
                planes: if hour % 2 == 0 { 7 } else { 9 },
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::WingEquipment.index()] = TableView::Complete(TableRows::WingEquipment(
        (0..count)
            .map(|index| WingEquipmentRow {
                wing: u64::from(index) + 1,
                ordinal: 0,
                variant: 1,
                amount_raw: if hour % 2 == 0 { 700_000 } else { 900_000 },
            })
            .collect(),
    ));
    *frame.tables[TableId::TaskForces.index()] = TableView::Complete(TableRows::TaskForces(
        (0..count)
            .map(|index| TaskForceRow {
                id: u64::from(index) + 1,
                roster_owner: 100_000 + index,
                logical_owner: 100_000 + index,
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::Ships.index()] = TableView::Complete(TableRows::Ships(
        (0..count)
            .map(|index| ShipRow {
                id: u64::from(index) + 1,
                task_force: u64::from(index) + 1,
                class_id: 2,
                strength: 100_000 + i64::from(hour),
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::ShipModels.index()] = TableView::Complete(TableRows::ShipModels(
        (0..count)
            .map(|index| ShipModelRow {
                ship: u64::from(index) + 1,
                counter: Some(ShipCounterInputs {
                    maximum_strength: 200_000 + i64::from(hour),
                    ..Default::default()
                }),
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: 1,
                literal: b"synthetic battalion".to_vec(),
            },
            LexicalTokenRow {
                token: 2,
                literal: b"synthetic ship".to_vec(),
            },
        ]));
    for &table in crate::products::army_size::prepared::TABLES
        .iter()
        .chain(crate::products::supply_health::prepared::TABLES)
        .chain(crate::products::navy::prepared::Navy::SAMPLE_TABLES)
        .chain(crate::products::air_force::prepared::Air::SAMPLE_TABLES)
    {
        frame.metrics.table_ns[table.index()] = 1;
    }
    frame
}

pub(crate) struct Fixture {
    pub source: Source,
    pub context: LoadContext,
    pub store: Arc<store::Store>,
    pub key: store::SourceKey,
    root: PathBuf,
}
impl Fixture {
    pub fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("forces-progressive-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("common/defines")).unwrap();
        std::fs::create_dir_all(root.join("user")).unwrap();
        std::fs::write(root.join("hoi4.exe"), "test").unwrap();
        std::fs::write(root.join("common/defines/synthetic.lua"), "NDefines={NMilitary={SUPPLY_GRACE=11,LOW_SUPPLY=.63},NGraphics={SUPPLY_UNIT_COUNTER_VERY_LOW_THRESHOLD=.17,SUPPLY_UNIT_COUNTER_LOW_THRESHOLD=.42}}").unwrap();
        std::fs::write(
            root.join("user/dlc_load.json"),
            r#"{"enabled_mods":[],"disabled_dlcs":[]}"#,
        )
        .unwrap();
        std::fs::write(
            root.join("user/settings.txt"),
            "language=l_english force_pow2_textures=no",
        )
        .unwrap();
        let (options, identity) = crate::test_fixtures::content_identity(&root);
        let mut wire = records(1, vec![frame(0, 1), frame(1, 1)]);
        if let replay::spool::Record::SessionStart(session) = &mut wire[0] {
            session.identity = identity;
        }
        let mut replay = Replay::from_records(CAMPAIGN.into(), &[wire]).unwrap();
        for block in 1..16 {
            replay = replay
                .append(&[continuation_records(
                    &replay,
                    1,
                    vec![frame(block * 2, 2), frame(block * 2 + 1, 2)],
                )])
                .unwrap();
        }
        let path = root.join("recording.battleplan");
        std::fs::write(&path, replay.bytes()).unwrap();
        let source = Source::open(
            crate::SourceKey {
                source_id: "forces-progressive".into(),
                revision: crate::protocol::replay_revision(&path).unwrap(),
            },
            path,
            &AtomicBool::new(false),
        )
        .unwrap();
        let context = LoadContext {
            content: Some(options),
        };
        let (store, key) = crate::prepared::files(&source, context.content.as_ref()).unwrap();
        Self {
            source,
            context,
            store,
            key,
            root,
        }
    }
    pub fn query(&self, product: &str, params: Value) -> Value {
        let request = crate::transport::ProductRequest {
            source_id: self.source.key().source_id.clone(),
            revision: self.source.key().revision.clone(),
            product: product.into(),
            params,
        };
        let reply: Value = serde_json::from_str(
            &crate::prepared::query::query(
                &self.source,
                &request,
                self.context.content.as_ref(),
                &AtomicBool::new(false),
            )
            .unwrap(),
        )
        .unwrap();
        reply["data"].clone()
    }
    pub fn identities(&self, writer: &mut store::Writer, last: i32) {
        let mut identities = country_history::HistoryScan::new(false, &[]);
        for hour in 0..=last {
            identities.observe(&frame(hour, if hour < 2 { 1 } else { 2 }));
        }
        writer
            .write_at(
                "country-identities-1",
                0,
                serde_json::to_vec(&identities).unwrap().as_slice(),
            )
            .unwrap();
        crate::prepared::pipeline::publish_coverage(writer, 1, 0, last, false).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.store.clear();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

pub(crate) fn tag(index: u32) -> String {
    String::from_utf8(scale_tag(index)).unwrap()
}

#[cfg(feature = "slow-tests")]
fn sampler_matches<F: Family>() {
    let frames = [frame(6, 5000), frame(7, 5000)];
    let replay =
        Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, frames.to_vec())]).unwrap());
    let source = Source::from_replay(
        crate::SourceKey {
            source_id: "force-sample".into(),
            revision: "1".into(),
        },
        "force-sample.battleplan".into(),
        replay.clone(),
        &AtomicBool::new(false),
    )
    .unwrap();
    let mut prepared = Prepared::<F>::default();
    for frame in &frames {
        prepared
            .observe(
                frame,
                &replay,
                &source,
                &LoadContext { content: None },
                &AtomicBool::new(false),
                &crate::prepared::frame_changes::FrameChangeTracker::new()
                    .observe(frame, replay.format),
            )
            .unwrap();
        let samples = sample::<F>(frame, None).unwrap();
        assert_eq!(samples.len(), 5000 * (4 + F::REFERENCES));
        let observed = std::array::from_fn(|index| F::observed(&prepared.scan, index, frame.hour));
        assert_eq!(
            samples,
            sample_rows::<F>(&prepared.scan, &observed, None).unwrap()
        );
        // Reading open state is a publication boundary: unchanged endpoints are deferred.
        for values in prepared.values.iter_mut().chain(&mut prepared.maximum) {
            values.seal();
        }
        for ((tag, field), (identity, value)) in &samples {
            assert!(identity.is_some());
            let key = prepared.keys[&series::field_key(tag, "")];
            let accumulator = if let Some(channel) = field.strip_prefix("maximum-") {
                &prepared.maximum[F::reference(channel.parse().unwrap()).unwrap()]
            } else {
                &prepared.values[field.parse::<usize>().unwrap()]
            };
            assert_eq!(
                *value,
                accumulator
                    .state(key)
                    .and_then(|state| state.last)
                    .filter(|run| run.last_hour == frame.hour)
                    .map(|run| run.value)
            );
        }
    }
    assert_eq!(
        sample::<F>(&frames[0], Some(&[("absent".into(), "0".into())]))
            .unwrap()
            .get(&("absent".into(), "0".into())),
        Some(&(None, None))
    );
}

#[test]
#[cfg(feature = "slow-tests")]
fn air_force_progressive_sampler_matches_observer_every_key_at_5000_countries() {
    sampler_matches::<crate::products::air_force::prepared::Air>();
}
#[test]
#[cfg(feature = "slow-tests")]
fn navy_progressive_sampler_matches_observer_every_key_at_5000_countries() {
    sampler_matches::<crate::products::navy::prepared::Navy>();
}

fn tail_and_zoom<F: Family>(family: Value, expected: f64) {
    let fixture = Fixture::new();
    let params = |from| json!({"branchId":"1", "family":family, "tags":[tag(0), tag(1)], "from":from, "to":31, "width":32});
    let cold = fixture.query("series-summary", params(0));
    assert!(
        !cold["summaries"][0]["summary"]["approximate"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        cold["series"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["tag"] == tag(1)),
        "tail-only countries need no catalogue key"
    );
    let after_rename = fixture.query(
        "series-summary",
        json!({"branchId":"1", "family":family,
        "tags":[tag(0)], "from":26,"to":31,"width":6}),
    );
    assert!(
        after_rename["series"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["tag"] == "renamed / first"
                && !row["runs"].as_array().unwrap().is_empty())
    );
    assert!(fixture.store.saved_key().unwrap().is_none());
    let mut prepared = Prepared::<F>::default();
    for hour in 0..2 {
        prepared
            .observe(
                &frame(hour, 1),
                fixture.source.replay_ref(),
                &fixture.source,
                &fixture.context,
                &AtomicBool::new(false),
                &crate::prepared::frame_changes::FrameChangeTracker::new().observe(
                    &frame(hour, if hour < 2 { 1 } else { 2 }),
                    fixture.source.replay_ref().format,
                ),
            )
            .unwrap();
        let mut writer = if hour == 0 {
            fixture.store.replace(fixture.key.clone()).unwrap()
        } else {
            fixture.store.extend(&fixture.key).unwrap()
        };
        prepared
            .flush(&mut writer, 1, &AtomicBool::new(false))
            .unwrap();
        fixture.identities(&mut writer, hour);
        writer.commit().unwrap();
        let bytes = serde_json::to_vec(&prepared).unwrap();
        prepared = serde_json::from_slice(&bytes).unwrap();
        prepared
            .resume(
                &frame(hour, 1),
                fixture.source.replay_ref(),
                &AtomicBool::new(false),
            )
            .unwrap();
    }
    let channel = family["measure"]
        .as_str()
        .map_or(0, |measure| usize::from(measure == "strength"));
    let ranges = fixture
        .store
        .range_count(&fixture.key, &format!("{}-values-1-{channel}", F::NAME))
        .unwrap();
    let rough = fixture.query("series-summary", params(0));
    assert_eq!(rough["coverage"]["lastHour"], 1);
    let exact = fixture.query("series-summary", params(6));
    for row in exact["summaries"].as_array().unwrap() {
        assert_eq!(row["summary"]["approximate"], json!([]));
    }
    let primary = exact["summaries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["tag"] == tag(0))
        .unwrap();
    assert!(
        primary["summary"]["runs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|run| run["firstHour"] == 9 && run["value"] == expected)
    );
    let alias = fixture.query(
        "series-summary",
        json!({"branchId":"1","family":family,"tags":[tag(0)],"from":0,"to":31,"width":32}),
    );
    assert!(
        alias["series"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["tag"] == "renamed / first")
    );
    assert!(
        !alias["series"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["tag"] == tag(1)),
        "sampling all holders must not expand manual selection"
    );
    let mut top = params(0);
    top["tags"] = json!([]);
    top["family"]["top"] = json!(1);
    assert!(
        !fixture.query("series-summary", top)["series"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let navigation = fixture.query("series-next-change", json!({"branchId":"1", "family":family,"tags":[tag(0)],"key":"ArrowRight","from":8,"firstHour":6,"lastHour":31}));
    assert_eq!(navigation["hour"].as_f64(), Some(9.0));
    assert_eq!(
        fixture
            .store
            .range_count(&fixture.key, &format!("{}-values-1-{channel}", F::NAME))
            .unwrap(),
        ranges,
        "transient queries never publish"
    );
    // Resume preparation into recording blocks appended after the original prefix.
    for hour in 2..4 {
        prepared
            .observe(
                &frame(hour, 2),
                fixture.source.replay_ref(),
                &fixture.source,
                &fixture.context,
                &AtomicBool::new(false),
                &crate::prepared::frame_changes::FrameChangeTracker::new().observe(
                    &frame(hour, if hour < 2 { 1 } else { 2 }),
                    fixture.source.replay_ref().format,
                ),
            )
            .unwrap();
    }
    let mut writer = fixture.store.extend(&fixture.key).unwrap();
    prepared
        .flush(&mut writer, 1, &AtomicBool::new(false))
        .unwrap();
    fixture.identities(&mut writer, 3);
    writer.commit().unwrap();
    let appended = fixture.query("series-summary", params(0));
    assert_eq!(appended["coverage"]["lastHour"], 3);
    assert!(
        appended["series"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["tag"] == tag(1))
    );
}
#[test]
fn air_force_progressive_queries_join_prepared_prefix_and_tail_then_zoom_exactly_without_writing() {
    tail_and_zoom::<crate::products::air_force::prepared::Air>(
        json!({"kind":"air-force-size","measure":"planes","ownership":"roster"}),
        9.0,
    );
}
#[test]
fn navy_progressive_queries_join_prepared_prefix_and_tail_then_zoom_exactly_without_writing() {
    tail_and_zoom::<crate::products::navy::prepared::Navy>(
        json!({"kind":"navy-size","measure":"strength","ownership":"roster","grouping":"class"}),
        100_009.0,
    );
}

#[test]
fn navy_progressive_parts_and_maximum_use_native_point_projection_and_exact_state() {
    let fixture = Fixture::new();
    let family =
        json!({"kind":"navy-size", "measure":"strength", "ownership":"roster", "grouping":"class"});
    let rough = fixture.query(
        "series-summary",
        json!({"branchId":"1", "family":family,
        "tags":[tag(0)], "from":0, "to":31, "width":32, "requireParts":true}),
    );
    assert!(!rough["parts"].as_array().unwrap().is_empty());
    assert!(
        !rough["summaries"][0]["parts"][0]["summary"]["approximate"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let state = fixture.query(
        "series-state",
        json!({"branchId":"1", "family":family,
        "reads":[{"tag":tag(0), "hour":9}], "requireParts":true}),
    );
    let run = &state["series"][0]["runs"][0];
    assert_eq!(run["total"], 100_009);
    assert_eq!(run["referenceTotal"], 200_009);
    assert_eq!(
        run["values"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_i64)
            .sum::<i64>(),
        100_009
    );
    assert!(fixture.store.saved_key().unwrap().is_none());
}

#[test]
fn progressive_part_absence_is_zero_and_explicit_refusal_stays_unknown() {
    let totals = [Run::new(0, 5, 10, true)];
    let parts = [Run::new(0, 1, 3, true), Run::new(4, 5, 7, false)];
    let witnessed = [(0, 2, 0, 1), (4, 5, 0, 1)];
    let runs = part_runs(&totals, &parts, &witnessed);
    assert_eq!(transient_at(&runs, 2), None);
    assert_eq!(transient_at(&runs, 3).unwrap().value, 0);
    assert_eq!(transient_at(&runs, 4).unwrap().value, 7);
}

#[test]
fn force_progressive_state_returns_only_each_requested_country_moment() {
    let fixture = Fixture::new();
    let state = fixture.query(
        "series-state",
        json!({"branchId":"1", "family":{
        "kind":"navy-size", "measure":"strength", "ownership":"roster", "grouping":"class"},
        "reads":[{"tag":tag(0),"hour":9},{"tag":tag(1),"hour":11}]}),
    );
    let rows = state["series"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    for row in rows {
        let hour = if row["tag"] == tag(0) { 9 } else { 11 };
        assert_eq!(row["runs"].as_array().unwrap().len(), 1);
        assert_eq!(row["runs"][0]["firstHour"], hour);
        assert_eq!(row["runs"][0]["lastHour"], hour);
        assert_eq!(row["runs"][0]["total"], 100_000 + hour);
    }
    assert!(fixture.store.saved_key().unwrap().is_none());
}

#[test]
fn air_force_progressive_totals_draw_while_dated_variant_parts_keep_their_coverage_gate() {
    let fixture = Fixture::new();
    let params = json!({"branchId":"1", "family":{
        "kind":"air-force-size", "measure":"planes", "ownership":"roster"},
        "tags":[tag(0)], "from":0,"to":31,"width":32});
    assert!(!fixture.query("series-summary", params.clone()).is_null());
    let mut parts = params;
    parts["requireParts"] = json!(true);
    assert!(fixture.query("series-summary", parts).is_null());
    assert!(fixture.store.saved_key().unwrap().is_none());
}
