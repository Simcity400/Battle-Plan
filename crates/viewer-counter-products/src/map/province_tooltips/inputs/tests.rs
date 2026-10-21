use super::*;
use replay_data::{
    rows::*,
    temperature::{self, Province, Settings, State},
};
use replay_frame::frame::{Metrics, TableView};

fn frame() -> DecodedFrame {
    DecodedFrame {
        branch: 1,
        hour: 100,
        tables: std::array::from_fn(|_| {
            TableView::Unavailable {
                reason: "fixture".into(),
            }
            .into()
        }),
        metrics: Metrics::default(),
    }
}

fn put(frame: &mut DecodedFrame, rows: TableRows) {
    let index = rows.table().index();
    frame.tables[index] = TableView::Complete(rows).into();
}

fn weather(hours: &[Option<u32>], expected: &[i64; 3]) -> Arc<DecodedFrame> {
    let mut frame = frame();
    frame.hour += hours.len() as i32;
    put(
        &mut frame,
        TableRows::WeatherProvinces(
            [0, 100, 300]
                .into_iter()
                .enumerate()
                .map(|(id, value)| WeatherProvinceRow {
                    id: id as u32,
                    temperature: ProvinceTemperature::anchored(State {
                        temperature: value,
                        staged: value,
                        ..State::default()
                    }),
                    temperature_pass: Recorded::Value((id == 0).then(|| TemperaturePass {
                        anchor: 100,
                        cursor: 1,
                        anchor_seed: None,
                        hours: hours.to_vec(),
                        checksum: temperature::checksum(*expected),
                    })),
                    ..Default::default()
                })
                .collect(),
        ),
    );
    Arc::new(frame)
}

fn temperature_content() -> Content {
    Content {
        provinces: [(false, vec![]), (true, vec![2]), (true, vec![1])]
            .into_iter()
            .map(|(active, neighbours)| Province {
                active,
                neighbours,
                region: None,
                terrain: Ok(None),
            })
            .collect(),
        settings: Settings {
            smoothing: 50_000,
            provinces_per_update: 1,
            ..Default::default()
        },
        ..Default::default()
    }
}

#[test]
fn certified_temperature_inputs_match_fixed_neighbor_passes_and_synchronous_fallback() {
    let content = temperature_content();
    let prepared = TemperaturesByFrame::default();
    let synchronous = TemperaturesByFrame::default();
    let mut hours = Vec::new();
    for (seed, expected) in [
        (Some(10), [0, 200, 300]),
        (None, [0, 200, 300]),
        (Some(20), [0, 200, 250]),
        (Some(30), [0, 200, 250]),
        (Some(40), [0, 225, 250]),
    ] {
        hours.push(seed);
        let frame = weather(&hours, &expected);
        let rows = frame.complete::<tables::WeatherProvinces>().unwrap();
        prepared.prepare(&frame, Ok(&content));
        let first = prepared.ready(&frame).unwrap().unwrap();
        prepared.prepare(&frame, Ok(&content));
        assert!(Arc::ptr_eq(
            &first,
            &prepared.ready(&frame).unwrap().unwrap()
        ));
        for (slot, expected) in expected.into_iter().enumerate() {
            assert_eq!(
                prepared.at(&frame, rows, Ok(&content), slot as u32),
                Ok(expected)
            );
            assert_eq!(
                synchronous.at(&frame, rows, Ok(&content), slot as u32),
                Ok(expected)
            );
        }
        assert_eq!(
            prepared.at(&frame, rows, Ok(&content), 3),
            Err("province weather absent".into())
        );
    }
    // A seek and another branch cannot borrow a same-hour frame's certificate.
    let mut sought = (*weather(&[], &[0, 100, 300])).clone();
    sought.branch = 2;
    let sought = Arc::new(sought);
    assert!(prepared.ready(&sought).is_none());
    prepared.prepare(&sought, Ok(&content));
    assert_eq!(&*prepared.ready(&sought).unwrap().unwrap(), &[0, 100, 300]);
    // Dead frame entries are reclaimed by the next builder, not by the window.
    assert_eq!(prepared.ready.lock().unwrap().len(), 1);
}

#[test]
fn certified_temperature_checksum_failures_remain_refusals_even_after_a_success() {
    let content = temperature_content();
    let inputs = TemperaturesByFrame::default();
    let good = weather(&[Some(10)], &[0, 200, 300]);
    inputs.prepare(&good, Ok(&content));
    let bad = weather(&[Some(10)], &[0, 201, 300]);
    assert!(inputs.ready(&bad).is_none());
    inputs.prepare(&bad, Ok(&content));
    let reason = Err("derived temperatures at hour 101 differ from the recorded checksum".into());
    let rows = bad.complete::<tables::WeatherProvinces>().unwrap();
    assert_eq!(inputs.at(&bad, rows, Ok(&content), 1), reason);
    let fresh = TemperaturesByFrame::default();
    assert_eq!(fresh.at(&bad, rows, Ok(&content), 1), reason);
    // A checksum failure must not poison a different immutable frame's result.
    assert_eq!(
        inputs.at(
            &good,
            good.complete::<tables::WeatherProvinces>().unwrap(),
            Ok(&content),
            1,
        ),
        Ok(200)
    );
}

#[test]
fn certified_temperature_inputs_keep_whole_map_and_pass_refusals() {
    let content = temperature_content();
    for (kind, expected) in [
        (0, "WeatherProvinces slots are not dense"),
        (1, "WeatherProvinces temperature not recorded"),
        (
            2,
            "the collector did not read this hour's temperature pass whole",
        ),
        (
            3,
            "the temperature pass records 0 hours after its anchor 100, the frame is hour 101",
        ),
    ] {
        let mut frame = (*weather(&[Some(10)], &[0, 200, 300])).clone();
        let mut rows = frame
            .complete::<tables::WeatherProvinces>()
            .unwrap()
            .to_vec();
        match kind {
            0 => rows[2].id = 7,
            1 => rows[2].temperature = ProvinceTemperature::NOT_RECORDED,
            2 => rows[0].temperature_pass = Recorded::Value(None),
            _ => {
                if let Recorded::Value(Some(pass)) = &mut rows[0].temperature_pass {
                    pass.hours.clear();
                }
            }
        }
        put(&mut frame, TableRows::WeatherProvinces(rows));
        let frame = Arc::new(frame);
        let inputs = TemperaturesByFrame::default();
        inputs.prepare(&frame, Ok(&content));
        let rows = frame.complete::<tables::WeatherProvinces>().unwrap();
        assert_eq!(
            inputs.at(&frame, rows, Ok(&content), 1),
            Err(expected.into())
        );
        let fallback = TemperaturesByFrame::default();
        assert_eq!(
            fallback.at(&frame, rows, Ok(&content), 1),
            Err(expected.into())
        );
    }
    let frame = weather(&[None], &[0, 100, 300]);
    let inputs = TemperaturesByFrame::default();
    inputs.prepare(&frame, Err("content refused"));
    assert_eq!(
        inputs.at(
            &frame,
            frame.complete::<tables::WeatherProvinces>().unwrap(),
            Err("content refused"),
            1,
        ),
        Err("temperature content: content refused".into())
    );
}

fn countries(count: u32) -> DecodedFrame {
    let mut frame = frame();
    let mut countries: Vec<_> = (1..=count)
        .map(|id| CountryRow {
            id,
            canonical_slot: id,
            tag: b"AAA".to_vec(),
            ..Default::default()
        })
        .collect();
    countries.push(CountryRow {
        id: count + 1,
        canonical_slot: 2,
        tag: b"BBB".to_vec(),
        ..Default::default()
    });
    put(&mut frame, TableRows::Countries(countries));
    put(
        &mut frame,
        TableRows::CountryLinks(
            (1..=count + 1)
                .map(|id| CountryLinkRow {
                    id,
                    canonical_slot: if id == count + 1 { 2 } else { id },
                })
                .collect(),
        ),
    );
    put(
        &mut frame,
        TableRows::CountryAppearance(
            (1..=count + 1)
                .map(|country| CountryAppearanceRow {
                    country,
                    cosmetic_tag: if country == 1 {
                        b"SUB".to_vec()
                    } else {
                        Vec::new()
                    },
                    overlord: if country == 1 { (count + 1) as i32 } else { 0 },
                    ..Default::default()
                })
                .collect(),
        ),
    );
    put(
        &mut frame,
        TableRows::Politics(
            (1..=count + 1)
                .map(|country| PoliticsRow {
                    country,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    put(&mut frame, TableRows::PoliticalParties(Vec::new()));
    put(
        &mut frame,
        TableRows::Autonomy(vec![AutonomyRow {
            subject: 1,
            state: b"autonomy_puppet".to_vec(),
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableRows::ReservedCountryNaming(vec![ReservedCountryNamingRow {
            tag: b"ZER".to_vec(),
            built: 3,
            ..Default::default()
        }]),
    );
    frame
}

#[test]
fn demand_local_country_names_keep_aliases_overlords_reserved_zero_and_refusals_at_5000() {
    let loc: Localisation = [
        ("AAA", "First"),
        ("BBB", "Second"),
        ("BBB_ADJ", "Secondish"),
        ("SUB_BBB_subject", "$OVERLORDADJ$ realm"),
        ("ZER", "Zero"),
        ("ZER_DEF", "the zero"),
    ]
    .into_iter()
    .map(|(key, value)| (key.into(), value.into()))
    .collect();
    let mut frame = countries(5_000);
    {
        let names = Names::new(&frame, &loc);
        assert_eq!(names.name(1), Ok("Secondish realm".into()));
        assert_eq!(names.name(2), Ok("Second".into()));
        assert_eq!(names.tag(2, "absent"), Ok("BBB".into()));
        assert_eq!(names.name(0), Ok("Zero".into()));
        assert_eq!(names.name_with(0, "_DEF"), Ok("the zero".into()));
        assert_eq!(names.name_with(0, "_ADJ"), Ok(String::new()));
        assert_eq!(names.tag(6_000, "absent"), Err("absent".into()));
        assert_eq!(
            names.name(6_000),
            Err("country slot 6000 has no naming operands".into())
        );
    }
    frame.hour += 1;
    frame.tables[TableId::Politics.index()] = TableView::Unavailable {
        reason: "fixture".into(),
    }
    .into();
    {
        let names = Names::new(&frame, &loc);
        assert_eq!(
            names.name(1),
            Err("Politics is incomplete this hour".into())
        );
        assert_eq!(
            names.tag(1, "absent"),
            Err("Politics is incomplete this hour".into())
        );
        assert_eq!(names.name(0), Ok("Zero".into()));
    }
}
