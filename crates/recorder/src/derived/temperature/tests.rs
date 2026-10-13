use super::*;
use replay::{
    frame::{Metrics, TableView},
    rows::{ProvinceTemperature, TableId, TableRows, TemperaturePass},
    temperature::{PERIOD_DEFAULT_DATE, Period, Province, Settings, State},
};

const HOUR: i32 = 43_800_000 + 1936 * 8_760 + 100 * 24;

fn content() -> Content {
    let province = |active, neighbours: &[u32]| Province {
        active,
        neighbours: neighbours.to_vec(),
        region: active.then_some(1),
        terrain: Ok(None),
    };
    Content {
        provinces: vec![
            province(false, &[]),
            province(true, &[2]),
            province(true, &[1, 3]),
            province(true, &[2]),
        ],
        periods: [(
            1,
            Ok(vec![Period {
                start: PERIOD_DEFAULT_DATE,
                end: PERIOD_DEFAULT_DATE + 364 * 24,
                minimum: -1_000_000,
                maximum: 3_500_000,
            }]),
        )]
        .into_iter()
        .collect(),
        settings: Settings {
            variation: 200_000,
            max_change: 600_000,
            smoothing: 30_000,
            change_smoothing: 50_000,
            provinces_per_update: 2,
            terrain: Vec::new(),
        },
    }
}

fn frame(hour: i32, rows: Vec<WeatherProvinceRow>) -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch: 1,
        hour,
        metrics: Metrics::default(),
        tables: std::array::from_fn(|_| TableView::Unavailable {
            reason: "outside the temperature check".into(),
        })
        .map(Into::into),
    };
    *frame.tables[TableId::WeatherProvinces.index()] =
        TableView::Complete(TableRows::WeatherProvinces(rows));
    frame
}

fn anchored(states: &[State], pass: TemperaturePass) -> Vec<WeatherProvinceRow> {
    states
        .iter()
        .enumerate()
        .map(|(slot, &state)| WeatherProvinceRow {
            id: slot as u32,
            temperature: ProvinceTemperature::anchored(state),
            temperature_pass: Recorded::Value((slot == 0).then(|| pass.clone())),
            ..Default::default()
        })
        .collect()
}

fn start() -> Vec<State> {
    [0, 500_000, 900_000, -200_000]
        .into_iter()
        .map(|temperature| State {
            temperature,
            drift: 10_000,
            staged: temperature,
            working: 10_000,
        })
        .collect()
}

#[test]
fn anchored_frames_must_reproduce_their_checksum_and_proven_anchors_their_words() {
    let content = content();
    let states = start();
    let mut simulation = Simulation::new(HOUR, 1, states.clone());
    simulation.advance(&content, Some(7)).unwrap();
    let after_one = simulation.clone();
    simulation.advance(&content, Some(8)).unwrap();
    let pass = |anchor, hours: Vec<Option<u32>>, anchor_seed, cursor, checksum| TemperaturePass {
        anchor,
        cursor,
        anchor_seed,
        hours,
        checksum,
    };
    let frames = [
        frame(
            HOUR,
            anchored(
                &states,
                pass(
                    HOUR,
                    vec![],
                    None,
                    1,
                    temperature::checksum(states.iter().map(|s| s.temperature)),
                ),
            ),
        ),
        frame(
            HOUR + 1,
            anchored(
                &states,
                pass(HOUR, vec![Some(7)], None, 1, after_one.checksum()),
            ),
        ),
        // A periodic anchor proven by the hour before: its words are the pass's.
        frame(
            HOUR + 2,
            anchored(
                &simulation.states,
                pass(
                    HOUR + 2,
                    vec![],
                    Some(8),
                    simulation.cursor,
                    simulation.checksum(),
                ),
            ),
        ),
    ];
    let mut checker = Checker::new(Ok(content.clone()));
    for frame in &frames {
        checker.observe(frame, &Ok([0, 0]));
    }
    let stats = &checker.stats;
    assert_eq!(
        (
            stats.anchored_hours,
            stats.checksum_equal,
            stats.anchors_compared
        ),
        (3, 3, 1)
    );
    assert_eq!((stats.anchor_rows_equal, stats.mismatches()), (4, 0));
    // The first frame's anchor has no hour before it.
    assert_eq!(stats.anchors_unproven, 1);
    // A wrong checksum and an anchor that differs from the pass are mismatches.
    let mut wrong = frames.clone();
    let mut differing = simulation.states.clone();
    differing[2].drift += 1;
    *wrong[1].tables[TableId::WeatherProvinces.index()] = TableView::Complete(
        TableRows::WeatherProvinces(anchored(&states, pass(HOUR, vec![Some(7)], None, 1, 0))),
    );
    *wrong[2].tables[TableId::WeatherProvinces.index()] =
        TableView::Complete(TableRows::WeatherProvinces(anchored(
            &differing,
            pass(HOUR + 2, vec![], Some(8), simulation.cursor, 0),
        )));
    let mut checker = Checker::new(Ok(content));
    for frame in &wrong {
        checker.observe(frame, &Ok([0, 0]));
    }
    // The hour before the second anchor failed its checksum, so that anchor is unproven
    // like the first frame's.
    assert_eq!(checker.stats.checksum_mismatches, 2);
    assert_eq!(checker.stats.anchors_unproven, 2);
    let mut checker = Checker::new(Ok(content_with_window(2)));
    wrong[1] = frames[1].clone();
    for frame in &wrong {
        checker.observe(frame, &Ok([0, 0]));
    }
    assert_eq!(checker.stats.anchor_rows_differing, 1);
}

fn content_with_window(width: i32) -> Content {
    let mut content = content();
    content.settings.provinces_per_update = width;
    content
}

fn hourly(temperatures: &[i64]) -> Vec<WeatherProvinceRow> {
    temperatures
        .iter()
        .enumerate()
        .map(|(slot, &temperature)| WeatherProvinceRow {
            id: slot as u32,
            temperature: ProvinceTemperature::historical(temperature),
            temperature_pass: Recorded::NotRecorded,
            ..Default::default()
        })
        .collect()
}

#[test]
fn hourly_temperatures_change_only_in_the_rotating_window() {
    let content = content();
    let hours = [
        hourly(&[0, 500_000, 900_000, -200_000]),
        // The window of two from cursor 1: slots 1 and 2.
        hourly(&[0, 600_000, 800_000, -200_000]),
        // Then slots 3 and 0; slot 0 is inactive.
        hourly(&[0, 600_000, 800_000, -100_000]),
    ];
    let mut checker = Checker::new(Ok(content.clone()));
    for (offset, rows) in hours.iter().enumerate() {
        checker.observe(&frame(HOUR + offset as i32, rows.clone()), &Ok([0, 0]));
    }
    assert_eq!(checker.stats.mismatches(), 0);
    assert_eq!(checker.stats.bounded_compared, 3);
    // Slot 1 changing with slot 3 fits no window that followed the first hour's.
    let mut outside = hours.clone();
    outside[2] = hourly(&[0, 700_000, 800_000, -100_000]);
    let mut checker = Checker::new(Ok(content.clone()));
    for (offset, rows) in outside.iter().enumerate() {
        checker.observe(&frame(HOUR + offset as i32, rows.clone()), &Ok([0, 0]));
    }
    assert_eq!(checker.stats.window_mismatches, 1);
    // Nothing changes while the manager is suspended; an inactive province never changes.
    let mut checker = Checker::new(Ok(content));
    checker.observe(&frame(HOUR, hours[0].clone()), &Ok([0, 0]));
    checker.observe(
        &frame(HOUR + 1, hourly(&[1, 500_000, 900_000, -200_000])),
        &Ok([1, 0]),
    );
    assert_eq!(checker.stats.suspended_changed, 1);
}
