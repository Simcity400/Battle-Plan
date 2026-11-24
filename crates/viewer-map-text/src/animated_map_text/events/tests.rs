use super::*;
use replay_data::{
    rows::CountryLinkRow, rows::CountryRow, rows::CountryVisibilityRow, rows::LexicalTokenRow,
    rows::MapEventBuildingDamage, rows::MapEventCriticalBombardment, rows::MapEventFamilyHealth,
    rows::MapEventGeneralCapture, rows::MapEventIntelEntry, rows::MapEventParadrop,
    rows::MapEventResistance, rows::TableId, rows::TableRows,
};
use replay_frame::frame::TableView;

fn present(
    consumer: &mut Consumer,
    context: &Context<'_>,
    intervals: &[MapEventsRow],
    table_complete: bool,
    continuity: Continuity,
) -> Batch {
    consumer.present(
        &Prepared::new(context, intervals, table_complete, |province| {
            Some(([province as f32, 0.0, 0.0], [0.0; 4]))
        }),
        continuity,
    )
}

fn frame(hour: i32) -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch: 1,
        hour,
        metrics: Default::default(),
        tables: std::array::from_fn(|index| {
            TableView::Complete(TableRows::empty(TableId::ALL[index])).into()
        }),
    };
    *frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![
            CountryLinkRow {
                id: 10,
                canonical_slot: 1,
            },
            CountryLinkRow {
                id: 11,
                canonical_slot: 1,
            },
            CountryLinkRow {
                id: 20,
                canonical_slot: 2,
            },
            CountryLinkRow {
                id: 30,
                canonical_slot: 3,
            },
        ]));
    *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![
        CountryRow {
            id: 10,
            canonical_slot: 1,
            ..Default::default()
        },
        CountryRow {
            id: 20,
            canonical_slot: 2,
            ..Default::default()
        },
        CountryRow {
            id: 30,
            canonical_slot: 3,
            ..Default::default()
        },
    ]));
    *frame.tables[TableId::CountryVisibility.index()] =
        TableView::Complete(TableRows::CountryVisibility(vec![
            CountryVisibilityRow {
                country: 10,
                unit_modifier: 0,
            },
            CountryVisibilityRow {
                country: 20,
                unit_modifier: 0,
            },
        ]));
    frame
}

fn content() -> Content {
    Content {
        buildings: TokenKeys::new(["Syn_Mill".to_owned()].iter()),
    }
}

fn loc() -> Localisation {
    [
        ("INTEL_DIVISION_REVEALED", "Reveal"),
        ("OVERRUN_POPUP", "Overrun"),
        ("CRITICAL_BOMBARDMENT_DAMAGE", "Critical"),
        ("CAPTURED_GENERAL_MAP_TEXT", "Captured"),
        ("PROV_TEXT_UNITS_PARACHUTED", "Landed"),
        ("PROV_TEXT_UNITS_PARACHUTED_WITH_LOSSES", "Losses"),
        ("ACTIVITY_TEXT", "Activity [Root.GetName]"),
        ("EMPTY_ACTIVITY", ""),
        ("Syn_Mill", "Mill"),
        ("Syn_Mill_plural", "Wrong plural"),
        ("BUILDING_BOMBED", "Bombed $BUILDING$"),
        ("BUILDING_SABOTAGED", "Sabotaged $BUILDING$"),
        ("BUILDING_DAMAGE_REASON_COLLATERAL", "Collateral $BUILDING$"),
        ("BUILDING_REPAIRED", "Repaired $BUILDING$"),
        ("BUILDING_SCORCHED", "Scorched $BUILDING$"),
    ]
    .into_iter()
    .map(|(key, text)| (key.to_owned(), text.to_owned()))
    .collect()
}

fn event(sequence: u64, hour: i32, payload: MapEventPayload) -> MapEvent {
    MapEvent {
        sequence,
        hour,
        payload,
    }
}

fn critical(sequence: u64, hour: i32) -> MapEvent {
    event(
        sequence,
        hour,
        MapEventPayload::CriticalShoreBombardment(MapEventCriticalBombardment { province: 7 }),
    )
}

fn interval(from: i32, to: i32, first: u64, next: u64, events: Vec<MapEvent>) -> MapEventsRow {
    MapEventsRow {
        epoch: 17,
        from_hour: from,
        to_hour: to,
        first_sequence: first,
        next_sequence: next,
        health: [MapEventFamilyHealth {
            installed: true,
            enabled: true,
            complete: true,
            ..Default::default()
        }; MapEventFamily::COUNT],
        events,
    }
}

fn unit(levels: &[i8]) -> MapEventUnitIntel {
    MapEventUnitIntel {
        owner: 20,
        anchor_province: 7,
        visibility_province: 9,
        fog: true,
        intel: levels
            .iter()
            .map(|&level| MapEventIntelEntry { country: 1, level })
            .collect(),
    }
}

#[test]
fn signed_edge_intel_uses_first_duplicate_and_visibility_anchor() {
    let frame = frame(40);
    let content = content();
    let loc = loc();
    let mut vision = Vision::sharing(1, []);
    vision.set_visibility([(7, 255), (9, 0)]);
    let context = Context::new(&frame, &vision, &content, &loc);
    for (level, admitted) in [
        (i8::MIN, false),
        (-1, false),
        (0, false),
        (1, true),
        (2, true),
        (i8::MAX, true),
    ] {
        assert_eq!(
            context.unit_visible(&unit(&[level])).unwrap(),
            admitted,
            "{level}"
        );
    }
    assert!(!context.unit_visible(&unit(&[0, 2])).unwrap());
    assert!(context.unit_visible(&unit(&[2, 0])).unwrap());
    assert!(!context.unit_visible(&unit(&[])).unwrap());
}

#[test]
fn province_max_and_signed_modifier_preserve_threshold149() {
    let mut frame = frame(40);
    let content = content();
    let loc = loc();
    let mut vision = Vision::sharing(1, []);
    vision.set_visibility([(9, 150)]);
    assert!(
        Context::new(&frame, &vision, &content, &loc)
            .unit_visible(&unit(&[-1]))
            .unwrap()
    );
    vision.set_visibility([(9, 0)]);
    for (modifier, visible) in [(-100_000, false), (-40_000, true), (-40_400, false)] {
        *frame.tables[TableId::CountryVisibility.index()] =
            TableView::Complete(TableRows::CountryVisibility(vec![CountryVisibilityRow {
                country: 10,
                unit_modifier: modifier,
            }]));
        assert_eq!(
            Context::new(&frame, &vision, &content, &loc)
                .unit_visible(&unit(&[2]))
                .unwrap(),
            visible
        );
    }
}

#[test]
fn fog_off_does_not_demand_missing_surviving_unit_or_actor_census() {
    let mut frame = frame(40);
    let content = content();
    let loc = loc();
    *frame.tables[TableId::Countries.index()] = TableView::Unavailable {
        reason: "not recorded".into(),
    };
    let vision = Vision::default();
    let context = Context::new(&frame, &vision, &content, &loc);
    assert!(context.unit_visible(&unit(&[])).unwrap());
    assert!(context.paradrop_visible(true, 9999, 7).unwrap());
}

#[test]
fn missing_fog_can_admit_a_known_scaled_positive_but_never_invent_a_negative() {
    assert_eq!(unit_admission(250, None, 0, true), Some(true));
    assert_eq!(unit_admission(0, None, 0, true), None);
    assert_eq!(unit_admission(150, Some(0), 0, false), Some(true));
    assert_eq!(unit_admission(0, Some(0), 0, false), None);
    for modifier in [i64::MIN, i64::MAX, -100_000, 0] {
        if let Some(admitted) = unit_admission(150, None, modifier, false) {
            assert!(
                (150..=255).all(|base| (visibility::unit_byte(base, modifier) > 149) == admitted)
            );
        }
    }
}

#[test]
fn unknown_sharing_has_only_two_bases_even_when_signed_scaling_wraps() {
    let modifier = 49_999_999_900_000;
    assert_eq!(visibility::unit_byte(150, modifier), 0);
    assert_eq!(visibility::unit_byte(250, modifier), 0);
    assert_eq!(visibility::unit_byte(200, modifier), 255);
    assert_eq!(unit_admission(150, Some(0), modifier, false), Some(false));
}

#[test]
fn unavailable_or_truncated_modifier_is_unknown_and_shared_vision_is_not_own_country() {
    let mut frame = frame(40);
    let content = content();
    let loc = loc();
    let mut vision = Vision::sharing(1, [2]);
    vision.set_visibility([(9, 0)]);
    for view in [
        TableView::Unavailable {
            reason: "not recorded".into(),
        },
        TableView::Truncated {
            rows: TableRows::CountryVisibility(vec![]),
            missing: 1,
        },
    ] {
        *frame.tables[TableId::CountryVisibility.index()] = view;
        assert!(
            Context::new(&frame, &vision, &content, &loc)
                .unit_visible(&unit(&[2]))
                .is_err()
        );
        let own = MapEventUnitIntel {
            owner: 11,
            ..unit(&[])
        };
        assert!(
            Context::new(&frame, &vision, &content, &loc)
                .unit_visible(&own)
                .unwrap()
        );
    }
}

#[test]
fn complete_sparse_modifier_proves_positive_slot_zero_but_excludes_reserved_slot() {
    let mut frame = frame(40);
    let content = content();
    let loc = loc();
    *frame.tables[TableId::CountryVisibility.index()] =
        TableView::Complete(TableRows::CountryVisibility(vec![]));
    let mut vision = Vision::sharing(1, []);
    vision.set_visibility([(9, 0)]);
    let context = Context::new(&frame, &vision, &content, &loc);
    assert_eq!(context.modifier(), Ok(0));
    assert!(context.unit_visible(&unit(&[2])).unwrap());
    let reserved = Vision::sharing(0, []);
    assert!(
        Context::new(&frame, &reserved, &content, &loc)
            .modifier()
            .is_err()
    );
    let absent = Vision::sharing(999, []);
    assert!(
        Context::new(&frame, &absent, &content, &loc)
            .modifier()
            .is_err()
    );
}

#[test]
fn known_zero_native_base_precedes_unknown_modifier() {
    let mut frame = frame(40);
    let content = content();
    let loc = loc();
    *frame.tables[TableId::CountryVisibility.index()] = TableView::Unavailable {
        reason: "not recorded".into(),
    };
    let mut vision = Vision::sharing(1, []);
    vision.set_visibility([(9, 0)]);
    let context = Context::new(&frame, &vision, &content, &loc);
    for level in [i8::MIN, -1, 0] {
        assert_eq!(context.unit_visible(&unit(&[level])), Ok(false));
    }
}

#[test]
fn general_owner_or_captor_canonical_match_settles_independent_unknown_actor() {
    let frame = frame(40);
    let content = content();
    let loc = loc();
    let vision = Vision::sharing(1, []);
    let context = Context::new(&frame, &vision, &content, &loc);
    let payload = |owner, captor| {
        MapEventPayload::GeneralCaptured(MapEventGeneralCapture {
            owner,
            captor,
            province: 7,
        })
    };
    for operands in [(11, 9999), (9999, 11)] {
        let popup = context
            .popup(&event(1, 40, payload(operands.0, operands.1)))
            .unwrap()
            .unwrap();
        assert_eq!(popup.text, b"Captured");
    }
    assert!(
        context
            .popup(&event(1, 40, payload(20, 30)))
            .unwrap()
            .is_none()
    );
    assert!(context.popup(&event(1, 40, payload(9999, 30))).is_err());
}

#[test]
fn critical_has_no_owner_or_fog_filter_and_null_anchor_is_refused() {
    let mut frame = frame(40);
    let content = content();
    let loc = loc();
    *frame.tables[TableId::Countries.index()] = TableView::Unavailable {
        reason: "not recorded".into(),
    };
    let vision = Vision::sharing(1, []);
    let context = Context::new(&frame, &vision, &content, &loc);
    assert_eq!(
        context.popup(&critical(1, 40)).unwrap().unwrap().text,
        b"Critical"
    );
    assert!(
        context
            .popup(&event(
                1,
                40,
                MapEventPayload::CriticalShoreBombardment(MapEventCriticalBombardment {
                    province: 0
                })
            ))
            .is_err()
    );
}

#[test]
fn paradrop_uses_loss_key_and_positive_fog_without_guessing_missing_relations() {
    let mut frame = frame(40);
    let content = content();
    let loc = loc();
    *frame.tables[TableId::WarRelations.index()] = TableView::Unavailable {
        reason: "not recorded".into(),
    };
    let mut vision = Vision::sharing(1, []);
    vision.set_visibility([(7, 1)]);
    let context = Context::new(&frame, &vision, &content, &loc);
    for (lost, expected) in [(false, b"Landed".as_slice()), (true, b"Losses".as_slice())] {
        let popup = context
            .popup(&event(
                1,
                40,
                MapEventPayload::ParadropLanding(MapEventParadrop {
                    province: 7,
                    country: 20,
                    lost,
                    fog: true,
                }),
            ))
            .unwrap()
            .unwrap();
        assert_eq!(popup.text, expected);
    }
    vision.set_visibility([(7, 0)]);
    assert!(
        Context::new(&frame, &vision, &content, &loc)
            .paradrop_visible(true, 20, 7)
            .is_err()
    );
}

#[test]
fn positive_paradrop_fog_does_not_demand_viewer_country_resolution() {
    let mut frame = frame(40);
    let content = content();
    let loc = loc();
    *frame.tables[TableId::Countries.index()] = TableView::Unavailable {
        reason: "not recorded".into(),
    };
    let mut vision = Vision::sharing(1, []);
    vision.set_visibility([(7, 1)]);
    assert_eq!(
        Context::new(&frame, &vision, &content, &loc).paradrop_visible(true, 999, 7),
        Ok(true)
    );
    vision.set_visibility([(7, 0)]);
    assert!(
        Context::new(&frame, &vision, &content, &loc)
            .paradrop_visible(true, 999, 7)
            .is_err()
    );
}

#[test]
fn resistance_is_ordinary_localisation_and_matches_overlord() {
    let frame = frame(40);
    let content = content();
    let loc = loc();
    let vision = Vision::sharing(1, []);
    let context = Context::new(&frame, &vision, &content, &loc);
    let payload = |key: &[u8]| {
        MapEventPayload::ResistanceActivity(MapEventResistance {
            province: 7,
            controller: 20,
            overlord: 11,
            key: key.to_vec(),
        })
    };
    assert_eq!(
        context
            .popup(&event(1, 40, payload(b"ACTIVITY_TEXT")))
            .unwrap()
            .unwrap()
            .text,
        b"Activity [Root.GetName]"
    );
    assert!(
        context
            .popup(&event(1, 40, payload(b"EMPTY_ACTIVITY")))
            .unwrap()
            .is_none()
    );
    assert_eq!(
        context
            .popup(&event(1, 40, payload(b"missing_activity_key")))
            .unwrap()
            .unwrap()
            .text,
        b"missing_activity_key"
    );
}

#[test]
fn resistance_preserves_signed_positive_viewer_gate() {
    let mut frame = frame(40);
    let content = content();
    let loc = loc();
    let raw = i32::MIN as u32;
    *frame.tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(vec![CountryRow {
            id: raw,
            canonical_slot: 1,
            ..Default::default()
        }]));
    let vision = Vision::sharing(1, []);
    let context = Context::new(&frame, &vision, &content, &loc);
    let observation = event(
        1,
        40,
        MapEventPayload::ResistanceActivity(MapEventResistance {
            province: 7,
            controller: raw,
            overlord: raw,
            key: b"ACTIVITY_TEXT".to_vec(),
        }),
    );
    assert!(context.popup(&observation).unwrap().is_none());
}

#[test]
fn building_unknown_or_ambiguous_content_is_refused() {
    let mut frame = frame(40);
    let loc = loc();
    let vision = Vision::sharing(1, []);
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 77,
            literal: b"syn_mill".to_vec(),
        }]));
    let observation = event(
        1,
        40,
        MapEventPayload::BuildingDamage(MapEventBuildingDamage {
            building_type: 77,
            province: 7,
            controller: 10,
            reason: MapEventBuildingReason::Collateral,
        }),
    );
    for buildings in [
        TokenKeys::new(["Syn_Mill".to_owned(), "SYN_MILL".to_owned()].iter()),
        TokenKeys::default(),
    ] {
        let content = Content { buildings };
        assert!(
            Context::new(&frame, &vision, &content, &loc)
                .popup(&observation)
                .is_err()
        );
    }
}

#[test]
fn paused_same_hour_boundaries_own_new_sequences_and_preserve_identical_operations() {
    let frame = frame(40);
    let content = content();
    let loc = loc();
    let vision = Vision::sharing(1, []);
    let context = Context::new(&frame, &vision, &content, &loc);
    let mut consumer = Consumer::default();
    let first = interval(40, 40, 1, 3, vec![critical(1, 40), critical(2, 40)]);
    let batch = present(
        &mut consumer,
        &context,
        std::slice::from_ref(&first),
        true,
        Continuity::Continue,
    );
    assert!(batch.reset);
    assert_eq!(batch.popups.len(), 2);
    assert_eq!(batch.coverage, [true; MapEventFamily::COUNT]);
    assert!(
        present(
            &mut consumer,
            &context,
            &[first],
            true,
            Continuity::Continue
        )
        .popups
        .is_empty()
    );
    let second = interval(40, 40, 3, 4, vec![critical(3, 40)]);
    let batch = present(
        &mut consumer,
        &context,
        &[second],
        true,
        Continuity::Continue,
    );
    assert!(!batch.reset);
    assert_eq!(batch.popups[0].sequence, 3);
}

#[test]
fn a_number_in_flight_at_a_seal_is_presented_when_a_later_interval_lists_it() {
    let frame_40 = frame(40);
    let frame_41 = frame(41);
    let content = content();
    let loc = loc();
    let vision = Vision::sharing(1, []);
    let mut consumer = Consumer::default();
    let mut sealed = interval(39, 40, 1, 3, vec![critical(1, 40)]);
    for health in &mut sealed.health {
        health.complete = false;
        health.faults.insert(MapEventFault::InFlight);
    }
    let mut late = interval(40, 41, 3, 4, vec![critical(2, 40), critical(3, 41)]);
    for health in &mut late.health {
        health.complete = false;
        health.faults.insert(MapEventFault::InFlight);
    }
    let first = present(
        &mut consumer,
        &Context::new(&frame_40, &vision, &content, &loc),
        std::slice::from_ref(&sealed),
        true,
        Continuity::Continue,
    );
    assert_eq!(first.popups.len(), 1);
    assert_eq!(first.explosions.len(), 1);
    let context = Context::new(&frame_41, &vision, &content, &loc);
    let second = present(
        &mut consumer,
        &context,
        std::slice::from_ref(&late),
        true,
        Continuity::Continue,
    );
    assert_eq!(
        second
            .popups
            .iter()
            .map(|popup| popup.sequence)
            .collect::<Vec<_>>(),
        [2, 3]
    );
    assert_eq!(
        second
            .explosions
            .iter()
            .map(|impact| (impact.id, impact.province, impact.position[0]))
            .collect::<Vec<_>>(),
        [(2, 7, 7.0), (3, 7, 7.0)]
    );
    consumer.interrupt();
    let again = present(&mut consumer, &context, &[late], true, Continuity::Continue);
    assert!(again.popups.is_empty() && again.explosions.is_empty());
}

#[test]
fn unknown_health_keeps_positive_events_and_never_becomes_empty_proof() {
    let frame = frame(40);
    let content = content();
    let loc = loc();
    let vision = Vision::sharing(1, []);
    let context = Context::new(&frame, &vision, &content, &loc);
    let mut consumer = Consumer::default();
    let mut row = interval(39, 40, 1, 2, vec![critical(1, 39)]);
    row.health = [MapEventFamilyHealth::default(); MapEventFamily::COUNT];
    let batch = present(&mut consumer, &context, &[row], true, Continuity::Continue);
    assert!(batch.reset);
    assert_eq!(batch.popups.len(), 1);
    assert_eq!(batch.coverage, [false; MapEventFamily::COUNT]);
    assert_eq!(batch.refusals.len(), 7);
}

#[test]
fn nested_critical_and_collateral_are_both_retained_in_sequence_order() {
    let mut frame = frame(40);
    let content = content();
    let loc = loc();
    let vision = Vision::sharing(1, []);
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 77,
            literal: b"syn_mill".to_vec(),
        }]));
    let context = Context::new(&frame, &vision, &content, &loc);
    let row = interval(
        40,
        40,
        1,
        3,
        vec![
            critical(1, 40),
            event(
                2,
                40,
                MapEventPayload::BuildingDamage(MapEventBuildingDamage {
                    building_type: 77,
                    province: 7,
                    controller: 10,
                    reason: MapEventBuildingReason::Collateral,
                }),
            ),
        ],
    );
    let batch = present(
        &mut Consumer::default(),
        &context,
        &[row],
        true,
        Continuity::Continue,
    );
    assert_eq!(
        batch
            .popups
            .iter()
            .map(|popup| (popup.sequence, popup.text.as_slice()))
            .collect::<Vec<_>>(),
        vec![
            (1, b"Critical".as_slice()),
            (2, b"Collateral Mill".as_slice())
        ]
    );
}

#[test]
fn missing_interval_breaks_continuity_and_cannot_reemit_a_prior_singleton() {
    let frame = frame(40);
    let content = content();
    let loc = loc();
    let vision = Vision::sharing(1, []);
    let context = Context::new(&frame, &vision, &content, &loc);
    let mut consumer = Consumer::default();
    let row = interval(40, 40, 1, 2, vec![critical(1, 40)]);
    assert_eq!(
        present(
            &mut consumer,
            &context,
            std::slice::from_ref(&row),
            true,
            Continuity::Continue
        )
        .popups
        .len(),
        1
    );
    let missing = present(&mut consumer, &context, &[], false, Continuity::Continue);
    assert!(missing.reset);
    assert_eq!(missing.coverage, [false; MapEventFamily::COUNT]);
    assert!(
        present(&mut consumer, &context, &[row], true, Continuity::Continue)
            .popups
            .is_empty()
    );
}

#[test]
fn failed_same_hour_recovery_preserves_fence_but_failed_explicit_seek_restarts_it() {
    let frame = frame(40);
    let content = content();
    let loc = loc();
    let vision = Vision::sharing(1, []);
    let context = Context::new(&frame, &vision, &content, &loc);
    let at = (frame.branch, frame.hour);
    let row = interval(40, 40, 1, 2, vec![critical(1, 40)]);
    let prepared = Prepared::new(&context, std::slice::from_ref(&row), true, |_| {
        Some(([0.0; 3], [0.0; 4]))
    });
    for seek in [false, true] {
        let mut presenter = Presenter::default();
        let continuity = presenter.begin(at, false);
        assert_eq!(presenter.present(&prepared, continuity).popups.len(), 1);
        assert_eq!(presenter.fail(at, seek), seek);
        let continuity = presenter.begin(at, false);
        let batch = presenter.present(&prepared, continuity);
        assert_eq!(batch.popups.len(), usize::from(seek));
        assert_eq!(batch.explosions.len(), usize::from(seek));
    }
}

#[test]
fn toggling_fog_on_an_hour_already_presented_replays_nothing() {
    let frame = frame(40);
    let content = content();
    let loc = loc();
    let fogged = Vision::sharing(1, []);
    let clear = Vision::sharing(1, []).without_fog();
    let row = interval(40, 40, 1, 2, vec![critical(1, 40)]);
    let locate = |_| Some(([0.0; 3], [0.0; 4]));
    let at = (frame.branch, frame.hour);
    let mut presenter = Presenter::default();
    let first = Prepared::new(
        &Context::new(&frame, &fogged, &content, &loc),
        std::slice::from_ref(&row),
        true,
        locate,
    );
    let continuity = presenter.begin(at, false);
    assert_eq!(presenter.present(&first, continuity).popups.len(), 1);
    let toggled = Prepared::new(
        &Context::new(&frame, &clear, &content, &loc),
        std::slice::from_ref(&row),
        true,
        locate,
    );
    let continuity = presenter.begin(at, false);
    let batch = presenter.present(&toggled, continuity);
    assert!(!batch.reset);
    assert!(batch.popups.is_empty() && batch.explosions.is_empty());
    let other = Vision::sharing(2, []);
    let changed = Prepared::new(
        &Context::new(&frame, &other, &content, &loc),
        std::slice::from_ref(&row),
        true,
        locate,
    );
    let continuity = presenter.begin(at, false);
    assert!(presenter.present(&changed, continuity).reset);
}

#[test]
fn every_empty_presented_hour_updates_the_continuity_boundary() {
    let mut frame = frame(40);
    let content = content();
    let loc = loc();
    let vision = Vision::sharing(1, []);
    let mut consumer = Consumer::default();
    present(
        &mut consumer,
        &Context::new(&frame, &vision, &content, &loc),
        &[interval(39, 40, 1, 1, vec![])],
        true,
        Continuity::Continue,
    );
    frame.hour = 41;
    let empty = present(
        &mut consumer,
        &Context::new(&frame, &vision, &content, &loc),
        &[interval(40, 41, 1, 1, vec![])],
        true,
        Continuity::Continue,
    );
    assert!(!empty.reset);
    frame.hour = 42;
    let next = present(
        &mut consumer,
        &Context::new(&frame, &vision, &content, &loc),
        &[interval(41, 42, 1, 2, vec![critical(1, 42)])],
        true,
        Continuity::Continue,
    );
    assert!(!next.reset);
    assert_eq!(next.popups.len(), 1);
}

#[test]
fn interval_gap_resets_but_preserves_independently_valid_current_positives() {
    let mut frame = frame(40);
    let content = content();
    let loc = loc();
    let vision = Vision::sharing(1, []);
    let mut consumer = Consumer::default();
    present(
        &mut consumer,
        &Context::new(&frame, &vision, &content, &loc),
        &[interval(39, 40, 1, 2, vec![])],
        true,
        Continuity::Continue,
    );
    frame.hour = 45;
    let next = present(
        &mut consumer,
        &Context::new(&frame, &vision, &content, &loc),
        &[interval(44, 45, 5, 6, vec![critical(5, 45)])],
        true,
        Continuity::Continue,
    );
    assert!(next.reset);
    assert_eq!(next.popups.len(), 1);
    assert!(!next.refusals.is_empty());
}

#[test]
fn malformed_or_carried_interval_is_unknown_not_positive_or_negative_coverage() {
    let frame = frame(40);
    let content = content();
    let loc = loc();
    let vision = Vision::sharing(1, []);
    let context = Context::new(&frame, &vision, &content, &loc);
    for row in [
        interval(39, 39, 1, 2, vec![critical(1, 39)]),
        interval(41, 40, 1, 2, vec![critical(1, 40)]),
        interval(39, 40, 1, 2, vec![critical(2, 40)]),
        interval(39, 40, 1, 3, vec![critical(2, 40), critical(1, 40)]),
    ] {
        let batch = present(
            &mut Consumer::default(),
            &context,
            &[row],
            true,
            Continuity::Continue,
        );
        assert!(batch.reset);
        assert!(batch.popups.is_empty());
        assert_eq!(batch.coverage, [false; MapEventFamily::COUNT]);
    }
}

#[test]
fn epoch_and_explicit_seek_restart_viewer_players_without_sequence_collision() {
    let frame = frame(40);
    let content = content();
    let loc = loc();
    let vision = Vision::sharing(1, []);
    let context = Context::new(&frame, &vision, &content, &loc);
    let mut consumer = Consumer::default();
    let row = interval(39, 40, 50, 51, vec![critical(50, 40)]);
    present(
        &mut consumer,
        &context,
        std::slice::from_ref(&row),
        true,
        Continuity::Continue,
    );
    let seek = present(&mut consumer, &context, &[row], true, Continuity::Reset);
    assert!(seek.reset);
    assert_eq!(seek.popups.len(), 1);
    let mut loaded = interval(40, 40, 1, 2, vec![critical(1, 40)]);
    loaded.epoch = 18;
    let load = present(
        &mut consumer,
        &context,
        &[loaded],
        true,
        Continuity::Continue,
    );
    assert!(load.reset);
    assert_eq!(load.popups[0].sequence, 1);
}

#[test]
fn fresh_context_same_hour_does_not_reemit_but_branch_and_perspective_reset_do() {
    let mut frame = frame(40);
    let content = content();
    let loc = loc();
    let vision = Vision::sharing(1, []);
    let mut consumer = Consumer::default();
    let row = interval(40, 40, 1, 2, vec![critical(1, 40)]);
    assert_eq!(
        present(
            &mut consumer,
            &Context::new(&frame, &vision, &content, &loc),
            std::slice::from_ref(&row),
            true,
            Continuity::Continue
        )
        .popups
        .len(),
        1
    );
    assert!(
        present(
            &mut consumer,
            &Context::new(&frame, &vision, &content, &loc),
            std::slice::from_ref(&row),
            true,
            Continuity::Continue
        )
        .popups
        .is_empty()
    );
    frame.branch = 2;
    assert_eq!(
        present(
            &mut consumer,
            &Context::new(&frame, &vision, &content, &loc),
            std::slice::from_ref(&row),
            true,
            Continuity::Continue
        )
        .popups
        .len(),
        1
    );
    let other_view = Vision::sharing(2, []);
    let batch = present(
        &mut consumer,
        &Context::new(&frame, &other_view, &content, &loc),
        &[row],
        true,
        Continuity::Continue,
    );
    assert!(batch.reset);
    assert_eq!(batch.popups.len(), 1);
}

#[test]
fn closed_display_round_trip_keeps_order_unknowns_refusals_and_impact_bits() {
    let family = MapEventFamily::ALL[0];
    let prepared = Prepared {
        view: 1500,
        interval: Ok(PreparedInterval {
            boundary: Boundary {
                branch: 7,
                epoch: 3,
                from_hour: 39,
                to_hour: 40,
                first: 9,
                next: 12,
            },
            in_flight: true,
            coverage: std::array::from_fn(|i| i % 2 == 0),
            coverage_refusals: vec!["synthetic gap B".into(), "synthetic gap A".into()],
            events: vec![
                PreparedEvent {
                    sequence: 11,
                    family,
                    popup: Err("synthetic popup refusal".into()),
                    explosion: Some(Err("synthetic anchor refusal".into())),
                },
                PreparedEvent {
                    sequence: 9,
                    family,
                    popup: Ok(Some(Popup {
                        sequence: 9,
                        family,
                        province: 44,
                        style: Style::SiegeInfo,
                        text: b"synthetic text".to_vec(),
                    })),
                    explosion: Some(Ok(Impact {
                        id: 9,
                        province: 44,
                        effect: Effect::CriticalBombardment,
                        position: [-0.0, f32::from_bits(0x7fc00123), 1.0],
                        bounds: [-1.0, 0.0, 2.0, 3.0],
                    })),
                },
                PreparedEvent {
                    sequence: 10,
                    family,
                    popup: Ok(None),
                    explosion: None,
                },
            ],
        }),
    };
    let before = prepared.display();
    let bytes = product_codec::encode(&before).unwrap();
    let restored = Prepared::from_display(product_codec::decode(&bytes).unwrap()).unwrap();
    assert_eq!(product_codec::encode(&restored.display()).unwrap(), bytes);
    assert_eq!(format!("{restored:?}"), format!("{prepared:?}"));
    let unavailable = Prepared::unavailable(1500, "synthetic source is unavailable".into());
    let wire = product_codec::encode(&unavailable.display()).unwrap();
    let restored = Prepared::from_display(product_codec::decode(&wire).unwrap()).unwrap();
    assert_eq!(format!("{restored:?}"), format!("{unavailable:?}"));
    let mut invalid = before;
    if let map_display_data::MapTextResult::Value { interval } = &mut invalid.interval {
        interval.coverage.pop();
    }
    assert!(Prepared::from_display(invalid).is_err());
}
