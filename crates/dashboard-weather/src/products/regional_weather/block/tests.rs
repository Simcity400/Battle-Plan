use super::*;
use crate::products::regional_weather::tests::{frame, row};
use dashboard_base::prepared::series::tests::{Fixture, assert_payloads_equal};
use std::sync::atomic::AtomicBool;

#[derive(Default, Serialize, Deserialize)]
struct Whole(Observer);

impl dashboard_keyed::Observer for Whole {
    const NAME: &'static str = Observer::NAME;
    const TABLES: &'static [TableId] = Observer::TABLES;
    const READ: TableId = Observer::READ;
    fn family() -> &'static Family {
        &FAMILY
    }
    fn fields() -> io::Result<Vec<TableFields>> {
        Observer::fields()
    }
    fn observe(
        &mut self,
        format: FormatVersion,
        frame: &DecodedFrame,
        values: &mut Values,
    ) -> io::Result<bool> {
        self.0.observe(format, frame, values)
    }
}

#[test]
fn regional_key_updates_match_independent_whole_observations_and_restores() {
    let mut changed = row(71);
    changed.light_rain = 3;
    changed.naval_detection = Recorded::NotRecorded;
    changed.combat = Recorded::NotRecorded;
    let mut inserted = row(91);
    inserted.snow = 2;
    let frames = [
        frame(10, Some(vec![row(71), row(72)])),
        frame(11, Some(vec![changed.clone(), row(72)])),
        frame(12, Some(vec![changed.clone(), row(72), inserted.clone()])),
        frame(13, Some(vec![row(72), inserted.clone()])),
        frame(14, None),
        frame(15, Some(vec![changed.clone(), inserted.clone()])),
        frame(16, Some(vec![row(71), inserted.clone()])),
        frame(17, Some(vec![])),
        frame(18, Some(vec![row(72)])),
    ];
    let publish = |incremental, restore| {
        let fixture = Fixture::new();
        let flag = AtomicBool::new(false);
        let mut scan = Scan::<Observer>::default();
        let mut whole = Scan::<Whole>::default();
        let mut previous = None;
        for (at, frame) in frames.iter().enumerate() {
            if incremental {
                scan.observer.inputs.push_back(Inputs {
                    point: (frame.branch, frame.hour),
                    previous,
                    regions: Some(match at {
                        1 | 3 | 6 => vec![71],
                        2 => vec![91],
                        7 => vec![71, 91],
                        _ => vec![71, 72, 91],
                    }),
                    provinces: None,
                    _credit: None,
                });
                scan.prepare(FormatVersion::CURRENT, frame, true).unwrap();
            } else {
                whole.prepare(FormatVersion::CURRENT, frame, true).unwrap();
            }
            previous = Some((frame.branch, frame.hour));
            if at == 2 || at == 5 || at + 1 == frames.len() {
                let mut writer = fixture.writer();
                if incremental {
                    scan.flush(&mut writer, 1, &flag).unwrap();
                } else {
                    whole.flush(&mut writer, 1, &flag).unwrap();
                }
                writer.commit().unwrap();
                if restore {
                    scan = serde_json::from_slice(&serde_json::to_vec(&scan).unwrap()).unwrap();
                }
            }
        }
        fixture.payloads()
    };
    let oracle = publish(false, false);
    assert_payloads_equal(&publish(true, false), &oracle, "regional changed keys");
    assert_payloads_equal(&publish(true, true), &oracle, "regional restored keys");
}

#[test]
fn temperature_hints_drop_clock_snow_and_pass_only_keys_but_keep_anchor_changes_and_deletes() {
    use dashboard_base::prepared::frame_changes::FrameChangeTracker;
    use replay_data::rows::{RowCodec, TableRows, WeatherProvinceRow};
    use replay_frame::frame::{Frame, TableUpdate};
    let table = TableId::WeatherProvinces;
    let mask = field_mask(
        table,
        &[
            "temperature.temperature",
            "temperature.drift",
            "temperature.staged",
            "temperature.working",
        ],
    );
    assert_ne!(mask, u64::MAX);
    let old = WeatherProvinceRow {
        id: 3,
        ..Default::default()
    };
    let mut unrelated = old.clone();
    unrelated.snow += 1;
    let mut anchor = WeatherProvinceRow {
        id: 7,
        ..Default::default()
    };
    let before = anchor.clone();
    anchor.temperature =
        replay_data::rows::ProvinceTemperature::anchored(replay_data::temperature::State {
            temperature: 17,
            ..Default::default()
        });
    let mut frame = frame(11, Some(vec![row(71)]));
    frame.tables[table.index()] =
        replay_frame::frame::TableView::Complete(TableRows::WeatherProvinces(vec![
            unrelated.clone(),
            anchor.clone(),
        ]))
        .into();
    let mut prior = frame.clone();
    prior.hour = 10;
    prior.tables[table.index()] =
        replay_frame::frame::TableView::Complete(TableRows::WeatherProvinces(vec![
            old.clone(),
            before.clone(),
            WeatherProvinceRow {
                id: 11,
                ..Default::default()
            },
        ]))
        .into();
    let raw = Frame {
        branch: frame.branch,
        hour: frame.hour,
        keyframe: false,
        metrics: frame.metrics.clone(),
        tables: std::array::from_fn(|index| {
            if index == table.index() {
                TableUpdate::Delta {
                    baseline: 10,
                    changed: vec![
                        unrelated.delta(Some(&old)).unwrap().unwrap(),
                        anchor.delta(Some(&before)).unwrap().unwrap(),
                    ],
                    deleted: vec![11],
                }
            } else {
                TableUpdate::Unchanged { baseline: 10 }
            }
        }),
    };
    let mut tracker = FrameChangeTracker::new();
    tracker.observe(&prior, FormatVersion::CURRENT);
    let changes = tracker.observe_updates(&frame, raw, FormatVersion::CURRENT);
    assert_eq!(keys(&changes, table, mask).unwrap(), Some(vec![7, 11]));
    assert_eq!(
        keys(&changes, table, u64::MAX).unwrap(),
        Some(vec![3, 7, 11])
    );
}
