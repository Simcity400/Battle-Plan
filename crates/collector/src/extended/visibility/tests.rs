use super::*;

fn fixture_capacities() -> Capacities {
    Capacities {
        countries: 8,
        states: 8,
        links: 8,
        wars: 8,
        war_links: 8,
        naval_access: 8,
        ..Default::default()
    }
}

#[test]
fn cold_visibility_reserves_only_native_singletons() {
    let captured = Captured::with_capacities(&Capacities::default());
    for capacity in [
        captured.countries.rows.capacity(),
        captured.states.rows.capacity(),
        captured.links.rows.capacity(),
        captured.wars.rows.capacity(),
        captured.war_links.first.capacity(),
        captured.naval_access.rows.capacity(),
    ] {
        assert_eq!(capacity, 0);
    }
    assert_eq!(captured.dlc_mask.rows.len(), 1);
    assert_eq!(captured.local_player.rows.len(), 1);
}

#[test]
fn appended_feature_failures_preserve_features_and_recover_through_the_codec() {
    use replay::frame::{DecodedFrame, Frame, FrameDecoder, FrameEncoder, Metrics};
    struct Features(Option<[u8; 2]>, Option<u8>);
    impl Memory for Features {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let mask = 0x1234_5678_u32.to_le_bytes();
            let human = [7];
            let manager = 0x2000_u64.to_le_bytes();
            let vtable = (cweather_manager::VTABLE_RVA as u64).to_le_bytes();
            let bytes: &[u8] = if address == engine::dlc_feature_state::MASK_RVA {
                &mask
            } else if address == native::INSTANT_DECISIONS_RVA {
                let Some(instant) = &self.1 else {
                    return false;
                };
                std::slice::from_ref(instant)
            } else if address == native::HUMAN_AI_RVA {
                &human
            } else if address == 0x1000 + engine::cgame_state::WEATHER_MANAGER {
                &manager
            } else if address == 0x2000 {
                &vtable
            } else if address == 0x2000 + cweather_manager::SUSPEND_FLAGS {
                let Some(flags) = &self.0 else {
                    return false;
                };
                flags
            } else {
                return false;
            };
            if output.len() != bytes.len() {
                return false;
            }
            output.copy_from_slice(bytes);
            true
        }
    }
    let mut captured = Captured::with_capacities(&fixture_capacities());
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    for (hour, (flags, instant)) in [
        (Some([3, 255]), Some(253)),
        (None, Some(253)),
        (Some([0, 0]), None),
        (None, None),
        (Some([0, 0]), Some(0)),
    ]
    .into_iter()
    .enumerate()
    {
        captured.capture(&Features(flags, instant), 0, 0x1000, hour as i32, hour == 0);
        assert_eq!(
            captured.weather_suspend_failure(),
            flags
                .is_none()
                .then_some("weather suspension flags unreadable")
        );
        let mut tables = std::array::from_fn(|_| TableView::Unavailable {
            reason: "fixture".into(),
        });
        tables[TableId::SimulationFeatures.index()] = captured.tables()[0].clone();
        let frame = encoder
            .encode(
                DecodedFrame {
                    branch: 1,
                    hour: hour as i32,
                    tables: tables.map(Into::into),
                    metrics: Metrics::default(),
                },
                false,
            )
            .unwrap();
        let mut bytes = Vec::new();
        frame.encode(&mut bytes).unwrap();
        let wire = Frame::decode(&mut replay::codec::Decoder::new(&bytes)).unwrap();
        let decoded = decoder.apply(&wire).unwrap();
        assert_eq!(
            decoded.complete::<tables::SimulationFeatures>().unwrap(),
            &[SimulationFeatureRow {
                dlc_mask: 0x1234_5678,
                human_ai: 7,
                weather_suspend: replay::rows::Recorded::Value(flags),
                instant_decisions: replay::rows::Recorded::Value(instant),
                runtime_scalars: replay::rows::Recorded::Value(
                    replay::rows::PoliticalRuntimeScalars {
                        sample_hour: hour as i32,
                        ..Default::default()
                    },
                ),
            }]
        );
    }
}

#[test]
fn local_player_switch_observer_failure_and_recovery_round_trip() {
    use replay::frame::{DecodedFrame, Frame, FrameDecoder, FrameEncoder, Metrics};
    let mut captured = Captured::with_capacities(&fixture_capacities());
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    // Unchanged, a controller change, an observer view, no view, a failed
    // read and recovery, each decoded through the codec.
    let samples = [
        Some([4_999_u32, 0]),
        Some([4_999, 0]),
        Some([5_000, 0]),
        Some([0, 4_999]),
        Some([0, 0]),
        None,
        Some([5_000, 0]),
    ];
    for (hour, sample) in samples.into_iter().enumerate() {
        let bytes: Vec<_> = sample
            .into_iter()
            .flatten()
            .flat_map(u32::to_le_bytes)
            .collect();
        let memory = engine::memory::SliceMemory {
            base: 0x1000 + engine::cgame_state::LOCAL_PLAYER_COUNTRY,
            bytes: &bytes,
        };
        captured.capture(&memory, 0, 0x1000, hour as i32, hour == 0);
        let mut tables = std::array::from_fn(|_| TableView::Unavailable {
            reason: "fixture".into(),
        });
        tables[TableId::LocalPlayer.index()] = captured.tables()[6].clone();
        let frame = encoder
            .encode(
                DecodedFrame {
                    branch: 1,
                    hour: hour as i32,
                    tables: tables.map(Into::into),
                    metrics: Metrics::default(),
                },
                false,
            )
            .unwrap();
        let mut bytes = Vec::new();
        frame.encode(&mut bytes).unwrap();
        let wire = Frame::decode(&mut replay::codec::Decoder::new(&bytes)).unwrap();
        let decoded = decoder.apply(&wire).unwrap();
        match sample {
            Some([country, observer_country]) => assert_eq!(
                decoded.complete::<tables::LocalPlayer>().unwrap(),
                &[LocalPlayerRow {
                    country,
                    observer_country
                }]
            ),
            None => assert!(decoded.complete::<tables::LocalPlayer>().is_err()),
        }
    }
    captured.fail_capture(9, false, "worker stopped");
    assert_eq!(
        captured.outcomes()[6],
        Outcome::Unavailable {
            reason: "worker stopped"
        }
    );
}

#[test]
fn growth_sizes_country_rows_from_the_live_slot_count() {
    let mut capacity = Capacities::default();
    let mut captured = Captured::with_capacities(&capacity);
    captured.slots = 5_000;
    captured.grow(&mut capacity);
    assert_eq!(
        (
            capacity.countries,
            capacity.links,
            capacity.wars,
            capacity.war_links,
            capacity.naval_access
        ),
        (5_000, 5_000, 5_000, 5_000, 5_000)
    );
    assert!(captured.countries.rows.len() >= 5_000);
}

#[test]
fn one_failed_input_leaves_the_others_and_the_hourly_features_fail_too() {
    let mut captured = Captured::with_capacities(&fixture_capacities());
    captured.fail_capture(1, false, "worker stopped");
    let outcomes = captured.outcomes();
    // SimulationFeatures is hourly: `human_ai` can be toggled at runtime.
    // NavalRegionAccess is daily: off its slot a failure is a skip.
    assert!(outcomes[..5].iter().all(|outcome| *outcome
        == Outcome::Unavailable {
            reason: "worker stopped"
        }));
    assert_eq!(
        outcomes[5],
        Outcome::Unavailable {
            reason: replay::cadence::SKIPPED_REASON
        }
    );
    captured.links.outcome = Outcome::Rows { len: 1 };
    captured.links.rows[0] = crelation_status::LinkRow {
        country: 5,
        target: 9,
        links: crelation_status::LINK_ATTACHE,
        transport_subject: None,
    };
    let [_, _, _, TableView::Complete(rows), _, _, _] = captured.tables() else {
        panic!("links table stays complete");
    };
    assert_eq!(rows.len(), 1);
}

#[test]
fn war_rows_convert_their_arena_spans_and_refuse_one_outside_it() {
    let mut captured = Captured::with_capacities(&fixture_capacities());
    captured.war_links.first[..3].copy_from_slice(&[7, 8, 9]);
    captured.war_links.len = 3;
    captured.wars.rows[0] = crelation_status::WarRow {
        country: 4,
        can_occupy_non_war: 1,
        enemies_read: true,
        enemies_start: 1,
        enemies_len: 2,
        ..Default::default()
    };
    captured.wars.outcome = Outcome::Rows { len: 1 };
    let TableView::Complete(replay::rows::TableRows::CountryWars(rows)) = captured.wars_table()
    else {
        panic!("war table stays complete");
    };
    assert_eq!(
        rows,
        [CountryWarRow {
            country: 4,
            can_occupy_non_war: 1,
            enemies_read: 1,
            enemies: vec![8, 9],
            ordered: Default::default(),
        }]
    );
    captured.wars.rows[0].enemies_len = 3;
    assert!(matches!(
        captured.wars_table(),
        TableView::Unavailable { .. }
    ));
}

#[test]
fn ordered_enemy_suffix_preserves_current_occurrences_and_refuses_independently() {
    let mut captured = Captured::with_capacities(&fixture_capacities());
    captured.war_links.first[..4].copy_from_slice(&[7, 0, 7, 9]);
    captured.war_links.len = 4;
    captured.wars.rows[0] = crelation_status::WarRow {
        country: 4,
        can_occupy_non_war: 0,
        ordered: Some((0, 4)),
        ordered_hour: Some(47),
        ..Default::default()
    };
    captured.wars.outcome = Outcome::Rows { len: 1 };
    let read = |captured: &Captured| {
        let TableView::Complete(replay::rows::TableRows::CountryWars(rows)) = captured.wars_table()
        else {
            panic!("ordered refusal must preserve the independent legacy row");
        };
        rows
    };
    let rows = read(&captured);
    assert_eq!(rows[0].enemies_read, 0);
    assert!(rows[0].enemies.is_empty());
    assert_eq!(
        rows[0].ordered,
        replay::rows::Recorded::Value(Some(replay::rows::OrderedEnemyInputs {
            sample_hour: 47,
            entries: vec![7, 0, 7, 9],
        }))
    );

    // Known empty and a failed arena/hour are different current observations.
    captured.wars.rows[0].ordered = Some((4, 0));
    assert_eq!(
        read(&captured)[0].ordered,
        replay::rows::Recorded::Value(Some(replay::rows::OrderedEnemyInputs {
            sample_hour: 47,
            entries: Vec::new(),
        }))
    );
    captured.wars.rows[0].ordered = Some((1, 4));
    assert_eq!(
        read(&captured)[0].ordered,
        replay::rows::Recorded::Value(None)
    );
    captured.wars.rows[0].ordered = Some((0, 4));
    captured.wars.rows[0].ordered_hour = None;
    assert_eq!(
        read(&captured)[0].ordered,
        replay::rows::Recorded::Value(None)
    );
}
