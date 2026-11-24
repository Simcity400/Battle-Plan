use super::*;

fn states(temperatures: &[i64]) -> Vec<State> {
    temperatures
        .iter()
        .map(|&temperature| State {
            temperature,
            drift: temperature / 10,
            staged: temperature,
            working: temperature / 10,
        })
        .collect()
}

fn words(cursor: i32, seed: u32, gates: Option<[u8; 2]>) -> Words {
    Words {
        cursor,
        window: 2,
        seed,
        gates,
    }
}

const CLEAR: Option<[u8; 2]> = Some([0, 0]);

fn observe(
    tracker: &mut Tracker,
    hour: i32,
    read: Option<(Words, &[State])>,
) -> Option<(Vec<i64>, TemperaturePass)> {
    let mut buffer = read.map(|(_, states)| states.to_vec());
    let read = read.map(|(words, _)| words).zip(buffer.as_mut());
    tracker.observe(hour, read).unwrap().map(|(anchor, pass)| {
        (
            anchor.iter().map(|state| state.temperature).collect(),
            pass.clone(),
        )
    })
}

#[test]
fn proven_hours_extend_the_anchor_and_anything_else_starts_a_new_one() {
    let mut tracker = Tracker::default();
    let first = states(&[0, 10, 20, 30, 40]);
    let (anchor, pass) = observe(&mut tracker, 100, Some((words(4, 7, CLEAR), &first))).unwrap();
    assert_eq!(anchor, [0, 10, 20, 30, 40]);
    assert_eq!(
        pass,
        TemperaturePass {
            anchor: 100,
            cursor: 4,
            anchor_seed: None,
            hours: Vec::new(),
            witness: replay_data::rows::TemperatureWitness::new(witness([0, 10, 20, 30, 40])),
        }
    );
    let second = states(&[1, 10, 20, 30, 41]);
    let (anchor, pass) = observe(&mut tracker, 101, Some((words(1, 9, CLEAR), &second))).unwrap();
    assert_eq!(anchor, [0, 10, 20, 30, 40]);
    assert_eq!(pass.hours, [Some(9)]);
    assert_eq!(
        pass.witness,
        replay_data::rows::TemperatureWitness::new(witness([1, 10, 20, 30, 41]))
    );
    let (_, pass) = observe(
        &mut tracker,
        102,
        Some((words(1, 9, Some([1, 0])), &second)),
    )
    .unwrap();
    assert_eq!(
        (pass.anchor, pass.hours),
        (100, vec![Some(9), None])
    );
    let third = states(&[1, 11, 20, 31, 41]);
    let (anchor, pass) = observe(&mut tracker, 103, Some((words(3, 5, CLEAR), &third))).unwrap();
    assert_eq!(anchor, [1, 11, 20, 31, 41]);
    assert_eq!(
        (pass.anchor, pass.cursor, pass.anchor_seed, pass.hours.len()),
        (103, 3, None, 0)
    );
    let (_, pass) = observe(&mut tracker, 104, Some((words(4, 6, CLEAR), &third))).unwrap();
    assert_eq!(pass.anchor, 104);
    let (_, pass) = observe(&mut tracker, 105, Some((words(4, 8, Some([0, 1])), &third))).unwrap();
    assert_eq!(pass.anchor, 105);
    let (_, pass) = observe(&mut tracker, 106, Some((words(4, 8, None), &third))).unwrap();
    assert_eq!(pass.anchor, 106);
    let (_, pass) = observe(&mut tracker, 108, Some((words(1, 2, CLEAR), &third))).unwrap();
    assert_eq!(pass.anchor, 108);
    assert!(observe(&mut tracker, 109, None).is_none());
    let (_, pass) = observe(&mut tracker, 110, Some((words(3, 2, CLEAR), &third))).unwrap();
    assert_eq!(pass.anchor, 110);
}

#[test]
fn an_anchor_lasts_two_keyframe_intervals_and_keeps_its_proven_pass() {
    let mut tracker = Tracker::default();
    let mut records = states(&[5, 6, 7]);
    let mut cursor = 0;
    let start = 1_000;
    observe(
        &mut tracker,
        start,
        Some((words(cursor, 0, CLEAR), &records)),
    )
    .unwrap();
    for offset in 1..=ANCHOR_HOURS {
        records[cursor as usize].temperature += 1;
        cursor = (cursor + 2) % 3;
        let (_, pass) = observe(
            &mut tracker,
            start + offset,
            Some((words(cursor, offset as u32, CLEAR), &records)),
        )
        .unwrap();
        if offset < ANCHOR_HOURS {
            assert_eq!(pass.anchor, start);
            assert_eq!(pass.hours.len(), offset as usize);
        } else {
            assert_eq!(pass.anchor, start + offset);
            assert_eq!(pass.anchor_seed, Some(offset as u32));
            assert!(pass.hours.is_empty());
        }
    }
}

#[test]
fn recorded_passes_derive_every_read_hour_on_playback() {
    use replay_data::{
        rows::ProvinceTemperature, rows::Recorded, rows::WeatherProvinceRow, temperature::Content,
        temperature::Memo, temperature::PERIOD_DEFAULT_DATE, temperature::Period,
        temperature::Province, temperature::Settings, temperature::Simulation,
    };
    let count = 7_u32;
    let content = Content {
        provinces: (0..count)
            .map(|slot| Province {
                active: slot != 0,
                neighbours: vec![(slot + 1) % count, (slot + count - 1) % count],
                region: (slot % 3 != 0).then_some(1),
                terrain: Ok(Some(1)),
            })
            .collect(),
        periods: [(
            1,
            Ok(vec![Period {
                start: PERIOD_DEFAULT_DATE,
                end: PERIOD_DEFAULT_DATE + 364 * 24,
                minimum: -300_000,
                maximum: 300_000,
            }]),
        )]
        .into_iter()
        .collect(),
        settings: Settings {
            variation: 200_000,
            max_change: 600_000,
            smoothing: 30_000,
            change_smoothing: 50_000,
            provinces_per_update: 3,
            terrain: vec![replay_data::temperature::TerrainModifier {
                terrain: 1,
                value: -50_000,
            }],
        },
    };
    let start = replay_data::temperature::EPOCH + 1936 * 8_760;
    let mut game = Simulation::new(
        start,
        0,
        (0..count)
            .map(|slot| State {
                temperature: i64::from(slot) * 70_000 - 200_000,
                drift: 0,
                staged: i64::from(slot) * 70_000 - 200_000,
                working: 0,
            })
            .collect(),
    );
    let mut tracker = Tracker::default();
    let mut memo = Memo::default();
    let mut seed = 0x1234_5678_u32;
    let hours = 300 + ANCHOR_HOURS + 10;
    let mut anchors = 0;
    for offset in 0..hours {
        let hour = start + offset;
        let suspended = offset % 97 == 5;
        if offset > 0 && !suspended {
            seed = seed.wrapping_add(0x9e37_79b9 ^ offset as u32);
            game.update(&content, hour, seed).unwrap();
        }
        if offset == 300 {
            for state in &mut game.states[1..] {
                state.temperature += 1;
                state.staged += 1;
            }
        }
        game.hour = hour;
        let words = Words {
            cursor: game.cursor,
            window: content.settings.provinces_per_update,
            seed,
            gates: Some(if suspended { [0, 1] } else { [0, 0] }),
        };
        let mut read = game.states.clone();
        let (anchor, pass) = tracker
            .observe(hour, Some((words, &mut read)))
            .unwrap()
            .unwrap();
        anchors += usize::from(pass.anchor == hour);
        let rows: Vec<WeatherProvinceRow> = anchor
            .iter()
            .enumerate()
            .map(|(slot, &state)| WeatherProvinceRow {
                id: slot as u32,
                temperature: ProvinceTemperature::anchored(state),
                temperature_pass: Recorded::Value((slot == 0).then(|| pass.clone())),
                ..Default::default()
            })
            .collect();
        let derived = memo.states(1, hour, &rows, Ok(&content)).unwrap();
        assert_eq!(derived, &game.states[..], "hour {offset}");
    }
    assert_eq!(anchors, 3);
}
