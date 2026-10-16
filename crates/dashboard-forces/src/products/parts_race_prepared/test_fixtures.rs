//! The force families' synthetic recording, prepared store and frames, shared by their tests
//! and the query hub's tests.
use super::*;
use dashboard_base::test_fixtures::{
    CAMPAIGN, continuation_records, records, scale_frame, scale_tag,
};
use replay::rows::*;
use std::{path::PathBuf, sync::Arc};

pub fn frame(hour: i32, count: u32) -> DecodedFrame {
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

pub struct Fixture {
    pub source: Source,
    pub context: LoadContext,
    pub store: Arc<store::Store>,
    pub key: store::SourceKey,
    root: PathBuf,
}
impl Fixture {
    pub fn new() -> Self {
        engine::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
        let (options, identity) = dashboard_base::test_fixtures::content_identity(&root);
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
            dashboard_base::source::SourceKey {
                source_id: "forces-progressive".into(),
                revision: dashboard_core::protocol::replay_revision(&path).unwrap(),
            },
            path,
            &AtomicBool::new(false),
        )
        .unwrap();
        let context = LoadContext {
            content: Some(options),
        };
        let (store, key) =
            dashboard_base::prepared::files(&source, context.content.as_ref()).unwrap();
        Self {
            source,
            context,
            store,
            key,
            root,
        }
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
        dashboard_base::prepared::coverage::publish_coverage(writer, 1, 0, last, false).unwrap();
    }
}
impl Default for Fixture {
    fn default() -> Self {
        Self::new()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.store.clear();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

pub fn tag(index: u32) -> String {
    String::from_utf8(scale_tag(index)).unwrap()
}
