use super::*;
use replay_data::{
    rows::ArmyGroupMemberRow, rows::DivisionTemplateIconRow, rows::OrdersGroupCounterInputRow,
    rows::OrdersGroupMemberRow, rows::OrdersGroupRow, rows::Recorded, rows::SimulationFeatureRow,
    rows::TableId, rows::TableRows,
};
use replay_frame::frame::TableView;
#[cfg(test)]
use viewer_foundation::tooltip_text::NoIcons;

fn frame() -> DecodedFrame {
    DecodedFrame {
        branch: 0,
        hour: 0,
        metrics: Default::default(),
        tables: TableId::ALL.map(|table| TableView::Complete(TableRows::empty(table)).into()),
    }
}

fn with_context<R>(frame: &DecodedFrame, run: impl FnOnce(&Context<'_>) -> R) -> R {
    let loc: Localisation = [
        ("COMMS_MULT_TEXT", "$MULT|%0$"),
        ("COMMS_MULT_NO_HQ", "no HQ $MULT_TEXT$"),
        ("COMMS_MULT", "distance $MULT_TEXT$"),
        ("ABILITY_TOOLTIP_LATE_JOINER_MULT", "late $MULT|%0$"),
    ]
    .into_iter()
    .map(|(key, value)| (key.into(), value.into()))
    .collect();
    let vision = Vision::sharing(1, []);
    let names = content_reader_division_names::division_names::Database::default();
    let units = Err("unused".into());
    run(&Context {
        nato: false,
        diagnostics: true,
        content: None,
        reads: None,
        dig_in_factor: Err("unused".into()),
        loc: &loc,
        recorded_language: &loc.language,
        icons: &NoIcons,
        names: &names,
        units: &units,
        supply_grace: Err("unused".into()),
        comms_max_distance: Err("unused".into()),
        frame,
        vision: &vision,
        links: frame
            .complete::<tables::CountryLinks>()
            .map_err(|error| error.to_string()),
    })
}

fn hover() -> CounterHover {
    CounterHover {
        control: viewer_native_gui::counter_hovers::land::CounterControl::Ability {
            country: 1,
            ordinal: 0,
        },
        authored: None,
        rect: [0.0; 4],
        remaining: None,
        ours: false,
        coarse: false,
        hidden: false,
        railway_gun: false,
        members: vec![1, 2].into(),
        timing: vec![].into(),
    }
}

#[test]
fn tooltip_commander_is_the_first_army_even_when_the_icon_has_another_caster() {
    let mut frame = frame();
    let first = 3 << 32 | 7;
    let last = 4 << 32 | 7;
    *frame.tables[TableId::Divisions.index()] = TableView::Complete(TableRows::Divisions(vec![
        DivisionRow {
            id: 1,
            orders_group: 10,
            ..Default::default()
        },
        DivisionRow {
            id: 2,
            orders_group: 20,
            ..Default::default()
        },
    ]));
    *frame.tables[TableId::OrdersGroups.index()] =
        TableView::Complete(TableRows::OrdersGroups(vec![
            OrdersGroupRow {
                handle: 10,
                leader: first,
                ..Default::default()
            },
            OrdersGroupRow {
                handle: 20,
                leader: last,
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::UnitLeaders.index()] =
        TableView::Complete(TableRows::UnitLeaders(vec![
            UnitLeaderRow {
                id: 3,
                reference_type: 7,
                ..Default::default()
            },
            UnitLeaderRow {
                id: 4,
                reference_type: 7,
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::ActiveAbilities.index()] =
        TableView::Complete(TableRows::ActiveAbilities(vec![ActiveAbilityRow {
            country: 1,
            ordinal: 0,
            caster: Some(last),
            ..Default::default()
        }]));
    with_context(&frame, |context| {
        assert_eq!(active_row(context, 1, 0).unwrap().caster, Some(last));
        assert_eq!(
            first_commander(context, &hover()).unwrap().unwrap().0,
            first
        );
    });
    *frame.tables[TableId::OrdersGroups.index()] =
        TableView::Complete(TableRows::OrdersGroups(vec![OrdersGroupRow {
            handle: 10,
            leader: 0,
            ..Default::default()
        }]));
    *frame.tables[TableId::UnitLeaders.index()] = TableView::Unavailable {
        reason: "unused".into(),
    };
    with_context(&frame, |context| {
        assert!(first_commander(context, &hover()).unwrap().is_none())
    });
}

#[test]
fn late_join_uses_first_matching_record_including_cooldown_and_skips_unused_count() {
    let mut frame = frame();
    *frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![
            CountryLinkRow {
                id: 1,
                canonical_slot: 1,
            },
            CountryLinkRow {
                id: 7,
                canonical_slot: 1,
            },
        ]));
    let reference = 3 << 32 | 7;
    *frame.tables[TableId::ActiveAbilities.index()] =
        TableView::Complete(TableRows::ActiveAbilities(vec![
            ActiveAbilityRow {
                country: 7,
                ordinal: 0,
                token: b"a".to_vec(),
                caster: Some(reference),
                phase: 1,
                division_count: 1,
                ..Default::default()
            },
            ActiveAbilityRow {
                country: 7,
                ordinal: 1,
                token: b"a".to_vec(),
                caster: Some(reference),
                phase: 0,
                division_count: 99,
                ..Default::default()
            },
        ]));
    let mut leader = UnitLeaderRow {
        owner: 1,
        category: 0,
        ..Default::default()
    };
    with_context(&frame, |context| {
        assert_eq!(
            late_join(context, reference, &leader, b"a", || Ok(4)).unwrap(),
            25_000
        );
        assert_eq!(
            late_join(context, reference, &leader, b"missing", || panic!(
                "unused count"
            ))
            .unwrap(),
            ONE
        );
    });
    leader.category = 2;
    with_context(&frame, |context| {
        assert_eq!(
            late_join(context, reference, &leader, b"a", || panic!(
                "category needs no count"
            ))
            .unwrap(),
            ONE
        )
    });
}

#[test]
fn late_join_keeps_the_signed_native_count_and_wrapping_numerator() {
    assert_eq!(late_join_multiplier(7, 3), ONE);
    assert_eq!(late_join_multiplier(-1, 0), -ONE);
    assert_eq!(late_join_multiplier(i32::MIN, 1), -30_280_924_062_904);
}

#[test]
fn fresh_formation_count_sums_compositions_recursively_and_keeps_repeated_children() {
    let mut frame = frame();
    *frame.tables[TableId::OrdersGroups.index()] =
        TableView::Complete(TableRows::OrdersGroups(vec![
            OrdersGroupRow {
                handle: 10,
                is_army_group: 1,
                ..Default::default()
            },
            OrdersGroupRow {
                handle: 20,
                ..Default::default()
            },
            OrdersGroupRow {
                handle: 30,
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::ArmyGroupMembers.index()] =
        TableView::Complete(TableRows::ArmyGroupMembers(vec![
            ArmyGroupMemberRow {
                group: 10,
                ordinal: 0,
                child: 20,
            },
            ArmyGroupMemberRow {
                group: 10,
                ordinal: 1,
                child: 30,
            },
            ArmyGroupMemberRow {
                group: 10,
                ordinal: 2,
                child: 20,
            },
        ]));
    *frame.tables[TableId::OrdersGroupMembers.index()] =
        TableView::Complete(TableRows::OrdersGroupMembers(vec![
            OrdersGroupMemberRow {
                group: 20,
                ordinal: 0,
                unit: 1,
            },
            OrdersGroupMemberRow {
                group: 20,
                ordinal: 1,
                unit: 2,
            },
            OrdersGroupMemberRow {
                group: 30,
                ordinal: 0,
                unit: 3,
            },
        ]));
    *frame.tables[TableId::Divisions.index()] = TableView::Complete(TableRows::Divisions(vec![
        DivisionRow {
            id: 1,
            template: 1000,
            ..Default::default()
        },
        DivisionRow {
            id: 2,
            template: 1001,
            ..Default::default()
        },
        DivisionRow {
            id: 3,
            template: 1001,
            ..Default::default()
        },
    ]));
    *frame.tables[TableId::DivisionTemplateIcons.index()] =
        TableView::Complete(TableRows::DivisionTemplateIcons(vec![
            DivisionTemplateIconRow {
                template: 1000,
                formation_division_count: Recorded::Value(Some(i32::MAX)),
                model_unreadable: 1,
                ..Default::default()
            },
            DivisionTemplateIconRow {
                template: 1001,
                formation_division_count: Recorded::Value(Some(2)),
                ..Default::default()
            },
        ]));
    let leader = UnitLeaderRow {
        formation_reference: Recorded::Value(Some(10)),
        ..Default::default()
    };
    with_context(&frame, |context| {
        assert_eq!(formation_count(context, &leader).unwrap(), 4)
    });
    *frame.tables[TableId::OrdersGroupMembers.index()] =
        TableView::Complete(TableRows::OrdersGroupMembers(vec![OrdersGroupMemberRow {
            group: 20,
            ordinal: 0,
            unit: 999,
        }]));
    with_context(&frame, |context| {
        assert!(
            formation_count(context, &leader)
                .unwrap_err()
                .contains("Divisions division 999")
        )
    });
    *frame.tables[TableId::ArmyGroupMembers.index()] =
        TableView::Complete(TableRows::ArmyGroupMembers(vec![ArmyGroupMemberRow {
            group: 10,
            ordinal: 0,
            child: 10,
        }]));
    with_context(&frame, |context| {
        assert!(
            formation_count(context, &leader)
                .unwrap_err()
                .contains("recursive group cycle")
        )
    });
    let null = UnitLeaderRow {
        formation_reference: Recorded::Value(Some(0)),
        ..Default::default()
    };
    with_context(&frame, |context| {
        assert_eq!(formation_count(context, &null).unwrap(), 0)
    });
}

#[test]
fn communications_reads_only_the_selected_native_distance_suffix() {
    let mut input = OrdersGroupCounterInputRow {
        hq_distance: Recorded::Value(Some(3)),
        invasion_source_distance: Recorded::NotRecorded,
        ..Default::default()
    };
    assert_eq!(communication_distance(&input, 2).unwrap(), 3);
    assert!(
        communication_distance(&input, 3)
            .unwrap_err()
            .contains("invasion_source_distance")
    );
    input.hq_distance = Recorded::Value(None);
    input.invasion_source_distance = Recorded::Value(Some(-1));
    assert_eq!(communication_distance(&input, 3).unwrap(), -1);
    assert!(
        communication_distance(&input, 2)
            .unwrap_err()
            .contains("hq_distance")
    );
    let mut frame = frame();
    let leader = UnitLeaderRow {
        character_type: 9,
        character_id: 11,
        ..Default::default()
    };
    *frame.tables[TableId::UnitLeaders.index()] =
        TableView::Complete(TableRows::UnitLeaders(vec![leader.clone()]));
    *frame.tables[TableId::SimulationFeatures.index()] =
        TableView::Complete(TableRows::SimulationFeatures(vec![
            SimulationFeatureRow::default(),
        ]));
    with_context(&frame, |context| {
        assert_eq!(communications(context, &leader).unwrap(), "")
    });
    *frame.tables[TableId::SimulationFeatures.index()] = TableView::Unavailable {
        reason: "unused".into(),
    };
    let leader = UnitLeaderRow {
        category: 2,
        ..leader
    };
    *frame.tables[TableId::UnitLeaders.index()] =
        TableView::Complete(TableRows::UnitLeaders(vec![leader.clone()]));
    with_context(&frame, |context| {
        assert_eq!(communications(context, &leader).unwrap(), "")
    });
}

#[test]
fn communications_resolves_the_character_role_and_refuses_an_ambiguous_direct_link() {
    let mut frame = frame();
    let original = UnitLeaderRow {
        id: 3,
        reference_type: 7,
        character_type: 9,
        character_id: 11,
        ..Default::default()
    };
    *frame.tables[TableId::UnitLeaders.index()] =
        TableView::Complete(TableRows::UnitLeaders(vec![
            original.clone(),
            UnitLeaderRow {
                id: 4,
                reference_type: 7,
                character_type: 9,
                character_id: 11,
                ..Default::default()
            },
        ]));
    with_context(&frame, |context| {
        assert!(
            communication_leader(context, &original)
                .unwrap_err()
                .contains("more than one unit leaders row")
        )
    });
    *frame.tables[TableId::UnitLeaders.index()] = TableView::Unavailable {
        reason: "unused".into(),
    };
    with_context(&frame, |context| {
        assert!(
            communication_leader(context, &UnitLeaderRow::default())
                .unwrap()
                .is_none()
        )
    });
}

#[test]
fn communications_uses_final_define_entry_for_negative_or_large_distance() {
    assert_eq!(communication_value(0, &[ONE, 80_000, 60_000]).unwrap(), ONE);
    assert_eq!(
        communication_value(1, &[ONE, 80_000, 60_000]).unwrap(),
        80_000
    );
    assert_eq!(
        communication_value(-1, &[ONE, 80_000, 60_000]).unwrap(),
        60_000
    );
    assert_eq!(
        communication_value(i32::MAX, &[ONE, 80_000, 60_000]).unwrap(),
        60_000
    );
    assert!(communication_value(0, &[]).unwrap_err().contains("empty"));
    with_context(&frame(), |context| {
        assert_eq!(
            communication_text(context.loc, 60_000, false).unwrap(),
            "distance -40%"
        );
        assert_eq!(
            communication_text(context.loc, 60_000, true).unwrap(),
            "no HQ -40%"
        );
    });
}

#[test]
fn independent_communications_failure_keeps_late_join_and_known_body() {
    with_context(&frame(), |context| {
        let mut out = Tooltip {
            title: "name\n\nknown effect\n".into(),
            body: "authored description".into(),
            ..Default::default()
        };
        finish(
            context,
            &mut out,
            Err("unreadable HQ branch".into()),
            Ok(25_000),
        );
        assert_eq!(out.title, "name\n\nknown effect\n\nlate 75%");
        assert_eq!(out.body, "authored description");
        assert_eq!(
            out.refused,
            BTreeMap::from([(
                "counter ability communications: unreadable HQ branch".into(),
                1
            )])
        );
        let mut out = Tooltip {
            title: "name\n\nknown effect".into(),
            ..Default::default()
        };
        finish(
            context,
            &mut out,
            communication_text(context.loc, 60_000, true),
            Err("unreadable count".into()),
        );
        assert_eq!(out.title, "name\n\nknown effect\nno HQ -40%");
        assert_eq!(
            out.refused,
            BTreeMap::from([("counter ability late join: unreadable count".into(), 1)])
        );
        let mut out = Tooltip {
            title: "name\n\nknown effect\n".into(),
            body: "\nauthored description\n\nsecond paragraph\n".into(),
            ..Default::default()
        };
        finish(context, &mut out, Ok(String::new()), Ok(ONE));
        assert_eq!(out.title, "name\n\nknown effect");
        assert_eq!(out.body, "\nauthored description\n\nsecond paragraph\n");
        assert!(out.refused.is_empty());
    });
}
