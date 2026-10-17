use super::*;
use crate::products::parts_race_prepared::test_fixtures::frame;
#[cfg(feature = "slow-tests")]
use crate::products::parts_race_prepared::test_fixtures::tag;
use dashboard_base::test_fixtures::{CAMPAIGN, records};
use serde_json::json;

#[test]
fn army_prepared_enables_detailed_inventory_when_fresh_and_resumed() {
    let fresh = Prepared::default();
    assert!(fresh.state.inventory.is_some());
    let resumed: Prepared = serde_json::from_slice(&serde_json::to_vec(&fresh).unwrap()).unwrap();
    assert!(resumed.state.inventory.is_some());
}

#[test]
#[cfg(feature = "slow-tests")]
fn army_progressive_sampler_matches_observer_every_key_at_5000_countries() {
    let frames = [frame(6, 5000), frame(7, 5000)];
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames.to_vec())]).unwrap();
    let mut prepared = Prepared::default();
    for frame in &frames {
        prepared
            .observe(
                frame,
                &replay,
                &dashboard_base::prepared::frame_changes::FrameChangeTracker::new()
                    .observe(frame, replay.format),
            )
            .unwrap();
        let sampled = sample(frame, None).unwrap();
        assert_eq!(sampled.len(), 5000 * 8);
        assert_eq!(
            sampled,
            sample_rows(&prepared.scan, &prepared.state, frame.hour, None).unwrap()
        );
        for ((tag, field), (identity, value)) in &sampled {
            assert!(identity.is_some());
            assert_eq!(
                *value,
                prepared.values[field.parse::<usize>().unwrap()]
                    .state(prepared.tags[tag])
                    .and_then(|state| state.last)
                    .filter(|run| run.last_hour == frame.hour)
                    .map(|run| run.value)
            );
        }
    }
    assert_eq!(
        sample(&frames[0], Some(&[("absent".into(), "0".into())])).unwrap(),
        BTreeMap::from([(("absent".into(), "0".into()), (None, None))])
    );
    assert_eq!(
        sample(&frames[1], Some(&[(tag(0), "6".into())])).unwrap()[&(tag(0), "6".into())].1,
        None,
        "a missing land row remains unknown locally"
    );
}

#[test]
fn phase_b_army_work_is_flat_over_hours_history_and_one_changed_row() {
    use dashboard_base::prepared::{frame_changes::FrameChangeTracker, observer_tests};
    use replay::{frame::TableView, rows::TableRows};
    let measure = |history: usize, hours: i32| -> io::Result<observer_tests::Work> {
        let mut first = frame(100, 128);
        for table in [
            TableId::DivisionCensus,
            TableId::Divisions,
            TableId::DivisionLandInputs,
        ] {
            match &mut *first.tables[table.index()] {
                TableView::Complete(TableRows::DivisionCensus(rows)) => rows.truncate(1),
                TableView::Complete(TableRows::Divisions(rows)) => rows.truncate(1),
                TableView::Complete(TableRows::DivisionLandInputs(rows)) => rows.truncate(1),
                _ => unreachable!(),
            }
        }
        let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![first.clone()])])?;
        let mut state = Prepared::default();
        let mut tracker = FrameChangeTracker::new();
        for index in 0..history {
            first.hour = 100 + index as i32;
            if let TableView::Complete(TableRows::Countries(rows)) =
                &mut *first.tables[TableId::Countries.index()]
            {
                rows[0].tag = format!("retired / {index}").into_bytes();
            }
            state.observe(&first, &replay, &tracker.observe(&first, replay.format))?;
        }
        let work = observer_tests::unchanged_frames(
            state.state.work,
            &first,
            hours,
            replay.format,
            |frame, changes| {
                state.observe(frame, &replay, changes)?;
                Ok(state.state.work)
            },
        )?;
        assert_eq!(work, observer_tests::Work::default());
        first.hour += hours;
        tracker.observe(&first, replay.format);
        first.hour += 1;
        if let TableView::Complete(TableRows::Divisions(rows)) =
            &mut *first.tables[TableId::Divisions.index()]
        {
            rows[0].hit_points += 1;
        }
        let before = state.state.work;
        state.observe(&first, &replay, &tracker.observe(&first, replay.format))?;
        let after = state.state.work;
        let changed = observer_tests::Work {
            row_visits: after.row_visits - before.row_visits,
            history_visits: after.history_visits - before.history_visits,
            emitted: after.emitted - before.emitted,
            ..Default::default()
        };
        assert!(
            changed.row_visits <= 1 && changed.history_visits <= 1 && changed.emitted <= 10,
            "{changed:?}"
        );
        Ok(changed)
    };
    observer_tests::assert_flat(
        observer_tests::scaled_work(8, |hours| measure(1, hours as i32)).unwrap(),
    );
    observer_tests::assert_flat(
        observer_tests::scaled_work(8, |history| measure(history, 8)).unwrap(),
    );
}

#[test]
fn phase_b_army_deferred_native_runs_match_eager_after_checkpoints_resume_append_gaps_and_aliases()
{
    use dashboard_base::prepared::{frame_changes::FrameChangeTracker, observer_tests};
    use replay::{
        frame::{FrameDecoder, FrameEncoder, TableView},
        rows::TableRows,
    };
    let initial = frame(100, 3);
    let replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![initial.clone()])]).unwrap();
    let mut lazy = Scan::default();
    let mut native = Scan::default();
    let mut lazy_state = scan_state();
    let mut native_state = scan_state();
    let mut tracker = FrameChangeTracker::new();
    let mut encoder = FrameEncoder::default();
    let mut decoder = FrameDecoder::default();
    for hour in [100, 101, 102, 103, 104, 105, 106, 107, 108, 111, 112, 113] {
        let mut next = initial.clone();
        next.hour = hour;
        if hour >= 106
            && let TableView::Complete(TableRows::Countries(rows)) =
                &mut *next.tables[TableId::Countries.index()]
        {
            rows[0].tag = b"renamed / first".to_vec();
        }
        if hour == 107 {
            *next.tables[TableId::Divisions.index()] = TableView::Unavailable {
                reason: "synthetic refusal".into(),
            };
        }
        let raw = encoder.encode(next.clone(), hour == 100).unwrap();
        let next = decoder.advance(&raw).unwrap().clone();
        native_state.step(&mut native, &raw, &next, &replay);
        let changes = tracker.observe(&next, replay.format);
        let raw = super::super::super::parts_race_prepared::updates_changed(
            &next,
            &[
                TableId::Divisions,
                TableId::DivisionTemplateIcons,
                TableId::DivisionLandInputs,
            ],
            Some(&changes),
            lazy_state.applied,
        );
        lazy_state.step_changes(&mut lazy, raw.as_ref(), &next, &replay, Some(&changes));
        if hour == 103 || hour == 108 {
            lazy.seal();
            lazy = serde_json::from_slice(&serde_json::to_vec(&lazy).unwrap()).unwrap();
            lazy_state = scan_state();
            tracker.invalidate();
        }
    }
    lazy.seal();
    let projection = |scan: &Scan| {
        json!({"unreadable":scan.unreadable,"composition":scan.composition_unreadable,
        "errors":scan.invalid_table_reads,"tracks":scan.tracks.iter().map(|(tag, track)|
            (tag,json!({"runs":track.runs,"segments":track.segments,"mix":track.mix}))).collect::<BTreeMap<_,_>>()})
    };
    observer_tests::assert_reply_parity(&projection(&lazy), &projection(&native)).unwrap();
}

/// A complete fixture table's rows.
fn rows<T: replay::rows::TableSchema>(frame: &mut DecodedFrame) -> &mut Vec<T::Row> {
    match &mut *frame.tables[T::ID.index()] {
        replay::frame::TableView::Complete(rows) => rows.rows_mut::<T>().expect("fixture rows"),
        _ => unreachable!("fixture tables are complete"),
    }
}

fn division(frame: &mut DecodedFrame, id: u64) -> &mut DivisionRow {
    rows::<tables::Divisions>(frame)
        .iter_mut()
        .find(|row| row.id == id)
        .expect("fixture division")
}

fn land(frame: &mut DecodedFrame, id: u64) -> &mut replay::rows::DivisionLandInputsRow {
    rows::<tables::DivisionLandInputs>(frame)
        .iter_mut()
        .find(|row| row.division == id)
        .expect("fixture land inputs")
}

fn census(frame: &mut DecodedFrame, country: u32) -> &mut DivisionCensusRow {
    rows::<tables::DivisionCensus>(frame)
        .iter_mut()
        .find(|row| row.country == country)
        .expect("fixture census row")
}

/// The recording codec: each input hour is encoded and decoded as the
/// preparation pipeline reads it, with its recorded updates.
struct Recorded {
    encoder: replay::frame::FrameEncoder,
    decoder: replay::frame::FrameDecoder,
}
impl Recorded {
    fn new() -> Self {
        Self {
            encoder: replay::frame::FrameEncoder::new(),
            decoder: replay::frame::FrameDecoder::new(),
        }
    }
    fn next(
        &mut self,
        input: &DecodedFrame,
        keyframe: bool,
    ) -> (DecodedFrame, Arc<replay::frame::Frame>) {
        let wire = self.encoder.encode(input.clone(), keyframe).unwrap();
        let decoded = self.decoder.advance(&wire).unwrap().clone();
        (decoded, Arc::new(wire))
    }
}

fn assert_one_division_hour_observes_only_its_country(count: u32) {
    use dashboard_base::prepared::{frame_changes::FrameChangeTracker, observer_tests::Work};
    let mut input = frame(100, count);
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![input.clone()])]).unwrap();
    let mut recorded = Recorded::new();
    let mut tracker = FrameChangeTracker::new();
    let mut prepared = Prepared::default();
    // The hour's work, its divisions read from changed rows, and its
    // country key comparisons.
    let mut step = |prepared: &mut Prepared, input: &DecodedFrame| {
        let keys = |prepared: &Prepared| {
            prepared
                .state
                .keys
                .as_ref()
                .map_or(0, dashboard_text::metadata::CountryKeys::row_visits)
        };
        let before = (prepared.state.work, prepared.row_hours, keys(prepared));
        let (decoded, wire) = recorded.next(input, false);
        let changes = tracker.observe_updates(&decoded, wire, replay.format);
        prepared.observe(&decoded, &replay, &changes).unwrap();
        let work = prepared.state.work;
        (
            Work {
                row_visits: work.row_visits - before.0.row_visits,
                history_visits: work.history_visits - before.0.history_visits,
                emitted: work.emitted - before.0.emitted,
                ..Default::default()
            },
            prepared.row_hours - before.1,
            keys(prepared) == before.2,
        )
    };
    let (first, _, _) = step(&mut prepared, &input);
    assert_eq!(
        first.history_visits, count as usize,
        "the first hour observes every country"
    );

    input.hour = 101;
    division(&mut input, u64::from(count / 2) + 1).hit_points += DivisionRow::HIT_POINTS_STEP.raw();
    let (work, row_hours, _) = step(&mut prepared, &input);
    assert_eq!(
        row_hours, 1,
        "the divisions are read from their changed rows"
    );
    assert_eq!(work.row_visits, 1, "only the changed division row is read");
    assert_eq!(work.history_visits, 1, "only its country is observed again");
    assert_eq!(work.emitted, 10, "only its country's series are written");

    // A field no total reads.
    input.hour = 102;
    division(&mut input, u64::from(count / 3) + 1).organisation +=
        DivisionRow::ORGANISATION_STEP.raw();
    let (work, row_hours, _) = step(&mut prepared, &input);
    assert_eq!(row_hours, 1);
    assert_eq!(
        (work.row_visits, work.history_visits, work.emitted),
        (1, 0, 0),
        "an organisation change observes no country"
    );

    input.hour = 103;
    census(&mut input, 100_000 + count / 4).ungrouped += 1;
    let (work, _, _) = step(&mut prepared, &input);
    assert_eq!(work.row_visits, 1, "only the changed census row is read");
    assert_eq!(work.history_visits, 1);

    // A country field no identity reads keeps the country keys uncompared.
    input.hour = 104;
    rows::<tables::Countries>(&mut input)[(count / 5) as usize].political_power_raw = Some(7);
    division(&mut input, u64::from(count / 6) + 1).hit_points -= DivisionRow::HIT_POINTS_STEP.raw();
    let (work, row_hours, keys_kept) = step(&mut prepared, &input);
    assert!(keys_kept, "the country keys are not compared again");
    assert_eq!(row_hours, 1);
    assert_eq!((work.row_visits, work.history_visits), (1, 1));

    input.hour = 105;
    land(&mut input, u64::from(count / 7) + 1).manpower[0].amount += 1;
    let (work, _, _) = step(&mut prepared, &input);
    assert_eq!(work.row_visits, 1, "only the changed land row is read");
    assert_eq!(work.history_visits, 1);
}

#[test]
fn army_hour_changing_one_division_observes_only_its_country() {
    assert_one_division_hour_observes_only_its_country(200);
}

#[test]
#[cfg(feature = "slow-tests")]
fn army_hour_changing_one_of_5000_divisions_observes_only_its_country() {
    assert_one_division_hour_observes_only_its_country(5000);
}

#[test]
fn army_field_masks_name_every_field_they_read() {
    assert_eq!(DIVISION_FIELDS.count_ones(), 4);
    assert_eq!(TEMPLATE_FIELDS.count_ones(), 2);
    assert_eq!(LAND_FIELDS.count_ones(), 1);
    assert_eq!(HOLDER_FIELDS.count_ones(), 2);
    let organisation = (0..64)
        .position(|field| TableId::Divisions.field_name(field) == Some("organisation"))
        .unwrap();
    assert_eq!(*DIVISION_FIELDS & (1 << organisation), 0);
}

/// Every series section and the catalogue a flush published, by tag key.
fn published(
    fixture: &dashboard_base::prepared::series::tests::Fixture,
    tags: &BTreeMap<String, u64>,
    last: i32,
) -> serde_json::Value {
    let flag = AtomicBool::new(false);
    let runs = |section: String, keys: &[u64]| {
        let mut reader = Reader::new(&fixture.store, &fixture.key, section, &flag).unwrap();
        keys.iter()
            .map(|&key| reader.runs(key, 100, last).unwrap())
            .collect::<Vec<_>>()
    };
    let actors: Vec<u64> = tags.values().copied().collect();
    let positions: Vec<u64> = (0..2 * (tags.len() as u64 + 1)).collect();
    let mut parts =
        Reader::new(&fixture.store, &fixture.key, "army-parts-1".into(), &flag).unwrap();
    let events: Vec<Vec<_>> = actors
        .iter()
        .map(|&key| {
            (100..=last)
                .map(|hour| parts.latest_event(key, hour).unwrap())
                .collect()
        })
        .collect();
    let catalogue = series::read_catalogue::<Scan>(&fixture.store, &fixture.key, "army", 1)
        .unwrap()
        .map(|(keys, scan)| {
            json!({
                "keys": keys,
                "measures": [scan.divisions, scan.strength, scan.manpower, scan.battalions,
                    scan.inventory_fallback],
                "unreadable": [scan.unreadable, scan.composition_unreadable],
                "errors": scan.invalid_table_reads,
                "tracks": scan.tracks.iter().map(|(tag, track)| (tag.clone(), json!({
                    "runs": track.runs, "segments": track.segments, "mix": track.mix,
                }))).collect::<BTreeMap<_, _>>(),
            })
        });
    json!({
        "values": (0..8)
            .map(|index| runs(format!("army-values-1-{index}"), &actors))
            .collect::<Vec<_>>(),
        "ranks": (0..8)
            .map(|index| runs(format!("army-ranks-1-{index}"), &positions))
            .collect::<Vec<_>>(),
        "coverage": runs("army-coverage-1".into(), &actors),
        "parts": runs("army-parts-1".into(), &actors),
        "events": events,
        "catalogue": catalogue,
    })
}

/// A scan track's runs of one channel as the series hold them: equal
/// adjacent values join unless a run starts a new segment.
fn track_runs(track: &Track, value: impl Fn(&RawRun) -> Option<i128>) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    for raw in &track.runs {
        let Some(amount) = value(raw) else {
            continue;
        };
        let starts = raw.starts
            || runs
                .last()
                .is_none_or(|last| last.last_hour.checked_add(1) != Some(raw.first));
        match runs.last_mut() {
            Some(last) if !starts && last.value == amount => last.last_hour = raw.last,
            _ => runs.push(Run::new(raw.first, raw.last, amount, starts)),
        }
    }
    runs
}

#[test]
fn army_rows_observation_matches_whole_tables_and_scan_across_keyframe_gap_and_resume() {
    use dashboard_base::prepared::frame_changes::FrameChangeTracker;
    use replay::rows::{
        CountryLinkRow, DivisionLandInputsRow, DivisionManpowerContributor,
        DivisionTemplateIconRow, LexicalTokenRow, TableRows, TemplateSubunit,
    };
    let flag = AtomicBool::new(false);
    let step = DivisionRow::HIT_POINTS_STEP.raw();
    let mut input = frame(100, 6);
    *input.tables[TableId::LexicalTokens.index()] =
        replay::frame::TableView::Complete(TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: 1,
                literal: b"infantry".to_vec(),
            },
            LexicalTokenRow {
                token: 2,
                literal: b"artillery".to_vec(),
            },
        ]));
    rows::<tables::DivisionTemplateIcons>(&mut input).push(DivisionTemplateIconRow {
        template: 2,
        subunits: vec![
            TemplateSubunit {
                token: 1,
                count: 2,
                local: 0,
            },
            TemplateSubunit {
                token: 2,
                count: 1,
                local: 0,
            },
        ],
        ..Default::default()
    });
    let mut inputs = Vec::new();
    let (mut stashed_divisions, mut stashed_census) = (None, None);
    for hour in 100..=140 {
        input.hour = hour;
        match hour {
            101 => division(&mut input, 1).hit_points += step,
            // Fields no total reads.
            102 => division(&mut input, 2).organisation += DivisionRow::ORGANISATION_STEP.raw(),
            103 => division(&mut input, 3).expeditionary_owner = 100_004,
            104 => land(&mut input, 4).manpower[0].amount += 7,
            105 => land(&mut input, 5).naval_transfer = true,
            106 => rows::<tables::DivisionTemplateIcons>(&mut input)[0].subunits[0].count = 4,
            107 => rows::<tables::DivisionTemplateIcons>(&mut input)[0].custom_icon = 9,
            108 => {
                rows::<tables::Divisions>(&mut input).push(DivisionRow {
                    id: 7,
                    roster_owner: 100_001,
                    logical_owner: 100_001,
                    expeditionary_owner: u32::MAX,
                    template: 2,
                    hit_points: 3 * step,
                    ..Default::default()
                });
                rows::<tables::DivisionLandInputs>(&mut input).push(DivisionLandInputsRow {
                    division: 7,
                    manpower: vec![DivisionManpowerContributor {
                        country: 100_001,
                        amount: 50,
                    }],
                    ..Default::default()
                });
                census(&mut input, 100_001).ungrouped += 1;
            }
            109 => {
                rows::<tables::Divisions>(&mut input).retain(|row| row.id != 2);
                rows::<tables::DivisionLandInputs>(&mut input).retain(|row| row.division != 2);
                census(&mut input, 100_001).grouped -= 1;
            }
            110 => rows::<tables::Countries>(&mut input)[2].political_power_raw = Some(5),
            111 => census(&mut input, 100_003).ungrouped = 4,
            // A keyframe.
            112 => division(&mut input, 1).hit_points += step,
            113 => {
                division(&mut input, 4).hit_points -= step;
                division(&mut input, 5).hit_points += step;
            }
            115 => rows::<tables::LexicalTokens>(&mut input)[0].literal = b"line_infantry".to_vec(),
            116 => rows::<tables::Countries>(&mut input)[5].tag = b"renamed / fifth".to_vec(),
            117 => {
                division(&mut input, 6).hit_points += step;
                division(&mut input, 7).template = 1;
            }
            118 | 119 | 121 => division(&mut input, 3).hit_points += step,
            // An hour without a recording.
            120 => continue,
            122 => {
                stashed_divisions = Some(std::mem::take(rows::<tables::Divisions>(&mut input)));
                *input.tables[TableId::Divisions.index()] = replay::frame::TableView::Unavailable {
                    reason: "synthetic missing divisions".into(),
                };
            }
            123 => {
                *input.tables[TableId::Divisions.index()] = replay::frame::TableView::Complete(
                    TableRows::Divisions(stashed_divisions.take().unwrap()),
                );
                division(&mut input, 1).hit_points += step;
            }
            124 => {
                stashed_census = Some(std::mem::take(rows::<tables::DivisionCensus>(&mut input)));
                *input.tables[TableId::DivisionCensus.index()] =
                    replay::frame::TableView::Unavailable {
                        reason: "synthetic missing census".into(),
                    };
            }
            125 => {
                *input.tables[TableId::DivisionCensus.index()] = replay::frame::TableView::Complete(
                    TableRows::DivisionCensus(stashed_census.take().unwrap()),
                );
                census(&mut input, 100_000).grouped += 1;
            }
            // A second link to the first country, then its census row.
            126 => rows::<tables::CountryLinks>(&mut input).push(CountryLinkRow {
                id: 200_000,
                canonical_slot: 0,
            }),
            127 => rows::<tables::DivisionCensus>(&mut input).push(DivisionCensusRow {
                country: 200_000,
                grouped: 2,
                ..Default::default()
            }),
            128 => {
                rows::<tables::DivisionTemplateIcons>(&mut input).retain(|row| row.template != 2)
            }
            129 => division(&mut input, 6).roster_owner = 100_002,
            130 => rows::<tables::DivisionCensus>(&mut input).retain(|row| row.country != 200_000),
            131 => {
                division(&mut input, 1).hit_points += step;
                division(&mut input, 3).organisation += DivisionRow::ORGANISATION_STEP.raw();
            }
            132..=140 => {
                division(&mut input, 4).hit_points += if hour % 2 == 0 { step } else { -step };
            }
            _ => {}
        }
        inputs.push((input.clone(), hour == 112));
    }
    let replay = Arc::new(
        Replay::from_records(
            CAMPAIGN.into(),
            &[records(
                1,
                inputs.iter().map(|(frame, _)| frame.clone()).collect(),
            )],
        )
        .unwrap(),
    );
    let fixtures = [
        dashboard_base::prepared::series::tests::Fixture::new(),
        dashboard_base::prepared::series::tests::Fixture::new(),
    ];
    // Observed from changed rows, and from the whole tables every hour.
    let mut observers = [
        Prepared::default(),
        Prepared {
            whole_tables: true,
            ..Default::default()
        },
    ];
    let mut trackers = [FrameChangeTracker::new(), FrameChangeTracker::new()];
    let mut recorded = Recorded::new();
    for (input, keyframe) in &inputs {
        let (decoded, wire) = recorded.next(input, *keyframe);
        for (observer, tracker) in observers.iter_mut().zip(&mut trackers) {
            let changes = tracker.observe_updates(&decoded, Arc::clone(&wire), replay.format);
            observer.observe(&decoded, &replay, &changes).unwrap();
        }
        if ![106, 113, 123, 129, 140].contains(&decoded.hour) {
            continue;
        }
        for (observer, fixture) in observers.iter_mut().zip(&fixtures) {
            let mut writer = fixture.writer();
            observer.flush(&mut writer, 1, &flag).unwrap();
            writer.commit().unwrap();
        }
        let [incremental, whole] = &observers;
        assert_eq!(
            incremental.tags, whole.tags,
            "keys are allocated in one order"
        );
        assert_eq!(incremental.mix_ids, whole.mix_ids);
        assert_eq!(
            published(&fixtures[0], &incremental.tags, decoded.hour),
            published(&fixtures[1], &whole.tags, decoded.hour),
            "published through hour {}",
            decoded.hour
        );
        // Resume both from their checkpoints.
        for (observer, tracker) in observers.iter_mut().zip(&mut trackers) {
            let (whole_tables, row_hours) = (observer.whole_tables, observer.row_hours);
            *observer = serde_json::from_slice(&serde_json::to_vec(&*observer).unwrap()).unwrap();
            observer.whole_tables = whole_tables;
            observer.row_hours = row_hours;
            tracker.invalidate();
        }
    }
    assert!(
        observers[0].row_hours >= 15,
        "most hours read their divisions from changed rows, not {}",
        observers[0].row_hours
    );
    assert_eq!(observers[1].row_hours, 0);
    // The published series hold the scan's runs.
    let oracle = super::super::scan(&replay, 1, true, &flag).unwrap();
    let tags = &observers[0].tags;
    for (tag, track) in &oracle.tracks {
        let key = tags[tag];
        for index in 0..8 {
            let mut reader = Reader::new(
                &fixtures[0].store,
                &fixtures[0].key,
                format!("army-values-1-{index}"),
                &flag,
            )
            .unwrap();
            assert_eq!(
                reader.runs(key, 100, 140).unwrap(),
                track_runs(track, |raw| raw.totals.values[index]),
                "{tag} channel {index}"
            );
        }
        let mut coverage = Reader::new(
            &fixtures[0].store,
            &fixtures[0].key,
            "army-coverage-1".into(),
            &flag,
        )
        .unwrap();
        assert_eq!(
            coverage.runs(key, 100, 140).unwrap(),
            track_runs(track, |_| Some(1)),
            "{tag} coverage"
        );
    }
    let catalogue = series::read_catalogue::<Scan>(&fixtures[0].store, &fixtures[0].key, "army", 1)
        .unwrap()
        .unwrap()
        .1;
    assert_eq!(
        (
            catalogue.unreadable,
            catalogue.composition_unreadable,
            &catalogue.invalid_table_reads
        ),
        (
            oracle.unreadable,
            oracle.composition_unreadable,
            &oracle.invalid_table_reads
        )
    );
}

#[test]
fn restored_army_mix_resumes_what_continuous_observation_publishes() {
    // One country fields one division of template 1 (three battalions). At
    // hour 102, the first hour after the commit, the template gains one. The
    // restored observer's fresh inventory then reaches the same composition
    // version the saved track holds.
    let mut input = frame(100, 1);
    let mut frames = Vec::new();
    for hour in 100..104 {
        input.hour = hour;
        if hour == 102 {
            rows::<tables::DivisionTemplateIcons>(&mut input)[0].subunits[0].count = 4;
        }
        // Unchanged tables keep their allocation, as the decoder carries them.
        frames.push(input.clone());
    }
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap();
    let flag = AtomicBool::new(false);
    dashboard_base::prepared::observer_tests::assert_restore_continues(
        replay.format,
        &[&frames[..2], &frames[2..]],
        Prepared::default,
        |observer, frame, changes, _| observer.observe(frame, &replay, changes).unwrap(),
        |observer, writer, ordinal| {
            if ordinal == 1 {
                let mix = observer
                    .scan
                    .tracks
                    .values()
                    .next()
                    .and_then(|track| track.mix.last())
                    .unwrap();
                assert_eq!(
                    (mix.first, &mix.parts[0]),
                    (
                        102,
                        &Some(BTreeMap::from([("synthetic battalion".to_owned(), 4)]))
                    ),
                    "the edited template opens a composition at the first hour after the commit"
                );
            }
            observer.flush(writer, 1, &flag).unwrap();
            writer
                .write_at(
                    "army-continuation-1",
                    0,
                    serde_json::to_vec(&*observer).unwrap().as_slice(),
                )
                .unwrap();
        },
        |fixture| {
            serde_json::from_reader(
                fixture
                    .store
                    .read(&fixture.key, "army-continuation-1", 0)
                    .unwrap()
                    .unwrap(),
            )
            .unwrap()
        },
    );
}

#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "5000-country sampled window")]
fn unchanged_army_window_reuses_values_warnings_and_compositions_before_its_cut_moves() {
    use dashboard_base::prepared::{
        coverage::{Coverage, SCHEMA},
        query::{observations, progressive},
    };
    use dashboard_base::test_fixtures::{continuation_records, file_source};
    let first = records(1, vec![frame(0, 5000), frame(1, 5000)]);
    let replay = Replay::from_records(CAMPAIGN.into(), &[first]).unwrap();
    let mut refused = frame(7, 5000);
    *refused.tables[TableId::Divisions.index()] = replay::frame::TableView::Unavailable {
        reason: "synthetic window refusal".into(),
    };
    let replay = replay
        .append(&[continuation_records(
            &replay,
            1,
            vec![frame(6, 5000), refused],
        )])
        .unwrap();
    let (source, root) = file_source(replay);
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let params = ArmySizeParams {
        branch_id: "1".into(),
        tags: Vec::new(),
        top: Some(10),
        measure: ArmyMeasure::Battalions,
        count_for: ArmyAttribution::Owner,
        by_type: true,
        window: None,
    };
    let extent = |last_hour| Coverage {
        schema: SCHEMA,
        first_hour: 0,
        last_hour,
        complete: false,
    };
    let read = |coverage: &Coverage| {
        sampled_window(&source, &params, &context, Some(coverage), (6, 7), &flag).unwrap()
    };
    let first = read(&extent(0));
    let (again, refused) = progressive::without_block_decodes(|| read(&extent(1)));
    assert!(
        !refused,
        "preparation before the window repeated its extraction"
    );
    assert_eq!(first.0, again.0);
    assert_eq!(first.1, again.1);
    assert_eq!(
        serde_json::to_value(&first.3).unwrap(),
        serde_json::to_value(&again.3).unwrap()
    );
    assert_eq!(observations::usage(&source, "army-size:window").2, 1);
    assert_eq!(first.0.len(), 5000);
    assert!(
        !first.3.invalid_table_reads.is_empty(),
        "fixture must retain warning evidence"
    );
    assert!(
        !first.3.tracks.is_empty(),
        "cached composition metadata was lost"
    );
    let cut = read(&extent(6));
    assert_eq!(observations::usage(&source, "army-size:window").2, 2);
    assert!(cut.0.values().flatten().all(|run| run.first_hour >= 7));
    let mut done = extent(7);
    done.complete = true;
    let finished = read(&done);
    assert!(finished.0.is_empty());
    let (bytes, bound, _) = observations::usage(&source, "army-size:window");
    assert!(bytes <= bound);
    let (store, _) = dashboard_base::prepared::files(&source, None).unwrap();
    let _ = store.clear();
    let _ = std::fs::remove_dir_all(root);
}
