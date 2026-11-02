use super::*;
use replay_data::{
    rows::ProvinceTemperature, rows::TableId, rows::TableRows, rows::TemperaturePass, temperature,
    temperature::PERIOD_DEFAULT_DATE, temperature::Period, temperature::Province,
    temperature::Settings, temperature::State,
};
use replay_frame::{frame::Metrics, frame::TableView};

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
fn anchored_frames_must_reproduce_their_witness_and_proven_anchors_their_words() {
    let content = content();
    let states = start();
    let mut simulation = Simulation::new(HOUR, 1, states.clone());
    simulation.advance(&content, Some(7)).unwrap();
    let after_one = simulation.clone();
    simulation.advance(&content, Some(8)).unwrap();
    let pass = |anchor, hours: Vec<Option<u32>>, anchor_seed, cursor, witness| TemperaturePass {
        anchor,
        cursor,
        anchor_seed,
        hours,
        witness: replay_data::rows::TemperatureWitness::new(witness),
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
                    temperature::witness(states.iter().map(|s| s.temperature)),
                ),
            ),
        ),
        frame(
            HOUR + 1,
            anchored(
                &states,
                pass(HOUR, vec![Some(7)], None, 1, after_one.witness()),
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
                    simulation.witness(),
                ),
            ),
        ),
    ];
    let mut checker = Checker::new(Ok(content.clone()));
    for frame in &frames {
        checker.observe(frame);
    }
    let stats = &checker.stats;
    assert_eq!(
        (
            stats.anchored_hours,
            stats.witness_equal,
            stats.anchors_compared
        ),
        (3, 3, 1)
    );
    assert_eq!((stats.anchor_rows_equal, stats.mismatches()), (4, 0));
    // The first frame's anchor has no hour before it.
    assert_eq!(stats.anchors_unproven, 1);
    // A wrong witness and an anchor that differs from the pass are mismatches.
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
    let mut checker = Checker::new(Ok(content.clone()));
    for frame in &wrong {
        checker.observe(frame);
    }
    // The hour before the second anchor failed its witness, so that anchor is unproven
    // like the first frame's.
    assert_eq!(checker.stats.witness_mismatches, 2);
    assert_eq!(checker.stats.anchors_unproven, 2);
    let mut checker = Checker::new(Ok(content));
    wrong[1] = frames[1].clone();
    for frame in &wrong {
        checker.observe(frame);
    }
    assert_eq!(checker.stats.anchor_rows_differing, 1);
    let mut stats = Stats::default();
    compare_anchor(
        &mut stats,
        &frames[2],
        &anchored(&simulation.states[..3], pass(HOUR + 2, vec![], None, 0, 0)),
        &simulation,
        simulation.cursor,
    );
    assert_eq!(stats.anchor_rows_differing, 1);
}
