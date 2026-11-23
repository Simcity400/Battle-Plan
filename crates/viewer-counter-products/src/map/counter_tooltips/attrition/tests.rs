use super::*;
use crate::map::division_eta::test_support::put_modifier_definitions;
use replay_data::{
    rows::CountryCounterInputRow, rows::CountryLinkRow, rows::CountryMovementModifiers,
    rows::CountryRow, rows::LexicalTokenRow, rows::MovementDefines,
    rows::ProvinceRow, rows::Recorded, rows::SimulationFeatureRow, rows::StateFlagRow,
    rows::TableId, rows::TableRows, rows::WeatherProvinceRow,
};
use replay_frame::frame::TableView;
#[cfg(test)]
use viewer_foundation::tooltip_text::NoIcons;

fn put(frame: &mut DecodedFrame, rows: TableRows) {
    let table = rows.table();
    *frame.tables[table.index()] = TableView::Complete(rows);
}

fn status(frame: &mut DecodedFrame) -> &mut DivisionStatusRow {
    let TableView::Complete(TableRows::DivisionStatus(rows)) =
        &mut *frame.tables[TableId::DivisionStatus.index()]
    else {
        unreachable!("the fixture holds complete status rows");
    };
    &mut rows[0]
}

fn frame(province: u32, path_total: u32, path: Vec<u32>) -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch: 0,
        hour: 0,
        metrics: Default::default(),
        tables: TableId::ALL.map(|table| TableView::Complete(TableRows::empty(table)).into()),
    };
    put(
        &mut frame,
        TableRows::Divisions(vec![DivisionRow {
            id: 1,
            province,
            logical_owner: 100,
            roster_owner: 100,
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableRows::DivisionStatus(vec![DivisionStatusRow {
            division: 1,
            attrition_value: 2_500,
            attrition_threshold: 0,
            supply_ratio: Some(100_000),
            supply_grace_maximum: Recorded::Value(100_000),
            supply_grace_remaining: Recorded::Value(0),
            out_of_supply_days: Recorded::Value(Some(0)),
            commander_modifiers: Recorded::Value(Some([None; 5])),
            weather_factor: Recorded::Value(Some(100_000)),
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableRows::CountryLinks(vec![CountryLinkRow {
            id: 100,
            canonical_slot: 4,
        }]),
    );
    put(
        &mut frame,
        TableRows::Countries(vec![CountryRow {
            id: 100,
            canonical_slot: 4,
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableRows::CountryCounterInputs(vec![CountryCounterInputRow {
            country: 100,
            movement_modifiers: Recorded::Value(Some(CountryMovementModifiers::default())),
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableRows::SimulationFeatures(vec![SimulationFeatureRow {
            movement_defines: Recorded::Value(MovementDefines {
                supply_days: Some(30),
                supply_grace: Some(72),
                out_of_supply_speed: Some(-80_000),
                out_of_supply_attrition: Some(20_000),
                ..Default::default()
            }),
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableRows::DivisionMovement(vec![DivisionMovementRow {
            division: 1,
            path_total,
            path,
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableRows::Provinces(
            [(5, 3), (7, 0), (8, 0)]
                .into_iter()
                .map(|(id, state)| ProvinceRow {
                    id,
                    state,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    put(
        &mut frame,
        TableRows::StateFlags(vec![StateFlagRow {
            state: 3,
            border_conflict_active: Recorded::Value(0),
            ..Default::default()
        }]),
    );
    put(
        &mut frame,
        TableRows::WarOwnedRelations(vec![replay_data::rows::WarOwnedRelationRow {
            owner: 100,
            retired: 1,
            ..Default::default()
        }]),
    );
    use engine_executable::modifier_keys::{
        ATTRITION, HEAT_ATTRITION, UNIT_UPKEEP_ATTRITION_FACTOR, WINTER_ATTRITION,
    };
    put_modifier_definitions(
        &mut frame,
        replay_derive::fixed_layout::MOVEMENT
            .keys
            .iter()
            .copied()
            .chain([ATTRITION, HEAT_ATTRITION, WINTER_ATTRITION, UNIT_UPKEEP_ATTRITION_FACTOR]),
    );
    frame
}

fn content() -> Content {
    Content {
        training: 5_000,
        border_factor: 10_000,
        supply_threshold: 35_000,
        terrains: vec![
            Terrain {
                name: b"plains".to_vec(),
                attrition: None,
            },
            Terrain {
                name: b"mountain".to_vec(),
                attrition: Some(30_000),
            },
        ],
        sub_units: Err("the content's sub-unit database is unavailable".into()),
    }
}

fn render_with(frame: &DecodedFrame, lookups: Result<&Lookups<'_>, String>) -> Tooltip {
    let loc: Localisation = [
        ("ATTRITION_TOTAL", "total $TOT|%2$\n"),
        ("ATTRITION_UPKEEP", "upkeep $VALUE|%2$"),
        ("ATTRITION_TERRAIN", "terrain $TERR$ $ATT|%2$"),
        ("ATTRITION_EQUIPMENT", "equipment $ATT|%2$"),
        ("ATTRITION_SUPPLY", "supply $ATT$"),
        ("TRAINING_ATTRITION", "train $VALUE$"),
        ("BORDER_WAR_HEADER", "border war"),
        ("BORDER_WAR_ATTRITION", "border $VAL|%1$"),
        ("mountain", "Mountain"),
    ]
    .into_iter()
    .map(|(key, value)| (key.into(), value.into()))
    .collect();
    let vision = Vision::sharing(1, []);
    let names = content_reader_division_names::division_names::Database::default();
    let units = Err("unused".into());
    let context = Context {
        nato: false,
        diagnostics: true,
        content: None,
        reads: None,
        dig_in_factor: Err("unused".into()),
        loc: &loc,
        recorded_language: "english",
        icons: &NoIcons,
        names: &names,
        units: &units,
        supply_grace: Ok(72),
        comms_max_distance: Err("unused".into()),
        frame,
        vision: &vision,
        links: Ok(&[]),
    };
    let mut out = Tooltip::default();
    append_with(&context, lookups, 1, &mut out);
    out
}

fn render(frame: &DecodedFrame) -> Tooltip {
    render_with(frame, Err("the counter content is unavailable".into()))
}

fn rendered(frame: &DecodedFrame) -> Tooltip {
    let content = content();
    let terrain = |province: u32| -> Result<i32, String> {
        Ok(match province {
            7 => 2,
            8 => 1,
            _ => 0,
        })
    };
    let border = |country: u32, state: u32| -> Result<i64, String> {
        assert_eq!((country, state), (100, 3));
        Ok(40_000)
    };
    let lookups = Lookups {
        content: &content,
        terrain: &terrain,
        border: &border,
    };
    render_with(frame, Ok(&lookups))
}

fn composition(frame: &mut DecodedFrame, unreadable: u8) {
    const MARINE: u32 = 900_002;
    const INFANTRY: u32 = 900_003;
    put(
        frame,
        TableRows::DivisionTemplateIcons(vec![replay_data::rows::DivisionTemplateIconRow {
            template: 0,
            subunits: [MARINE, INFANTRY]
                .into_iter()
                .map(|token| replay_data::rows::TemplateSubunit {
                    token,
                    local: 0,
                    count: 1,
                })
                .collect(),
            model_unreadable: unreadable,
            ..Default::default()
        }]),
    );
    let mut tokens = frame
        .complete::<replay_data::rows::tables::LexicalTokens>()
        .map(<[_]>::to_vec)
        .unwrap_or_default();
    tokens.retain(|row| row.token < MARINE);
    tokens.extend(
        [(MARINE, &b"marine"[..]), (INFANTRY, b"infantry")]
            .into_iter()
            .map(|(token, literal)| LexicalTokenRow {
                token,
                literal: literal.to_vec(),
            }),
    );
    put(frame, TableRows::LexicalTokens(tokens));
}

fn rendered_with_sub_units(frame: &DecodedFrame) -> Tooltip {
    let mut units = content_reader_script::sub_units::SubUnits::default();
    engine_executable::executable_tables::scope(Some(content_reader_script::test_game::tables()), || {
        units.read(
            b"sub_units={ marine={special_forces=yes type=infantry} infantry={type=infantry} }",
            "common/units/test.txt",
        );
    });
    let content = Content {
        sub_units: Ok(std::sync::Arc::new(units)),
        ..content()
    };
    let terrain = |province: u32| -> Result<i32, String> {
        Ok(match province {
            7 => 2,
            8 => 1,
            _ => 0,
        })
    };
    let border = |_: u32, _: u32| -> Result<i64, String> { Ok(40_000) };
    let lookups = Lookups {
        content: &content,
        terrain: &terrain,
        border: &border,
    };
    render_with(frame, Ok(&lookups))
}

fn refused(out: &Tooltip, line: &str) -> bool {
    out.refused
        .keys()
        .any(|key| key.starts_with(&format!("counter {line}:")))
}

#[test]
fn a_moving_unit_shows_its_upkeep_and_route_terrain() {
    let mut frame = frame(5, 2, vec![7, 8]);
    status(&mut frame).attrition_threshold = 10_000;
    let out = rendered(&frame);
    assert_eq!(
        out.title,
        "total 2.50%\nupkeep 10.00%\nterrain Mountain 40.00%"
    );
    for line in [
        "attrition equipment",
        "attrition upkeep modifiers",
        "attrition country modifiers",
    ] {
        assert!(refused(&out, line), "{line}: {:?}", out.refused);
    }
    for line in [
        "attrition state",
        "attrition supply",
        "attrition terrain",
        "attrition weather",
    ] {
        assert!(!refused(&out, line), "{line}: {:?}", out.refused);
    }
    let mut plains = frame.clone();
    put(
        &mut plains,
        TableRows::DivisionMovement(vec![DivisionMovementRow {
            division: 1,
            path_total: 1,
            path: vec![8],
            ..Default::default()
        }]),
    );
    assert_eq!(
        rendered(&plains).title,
        "total 2.50%\nupkeep 10.00%\nterrain plains 10.00%"
    );
    let mut redeploying = frame.clone();
    let TableView::Complete(TableRows::DivisionMovement(rows)) =
        &mut *redeploying.tables[TableId::DivisionMovement.index()]
    else {
        unreachable!("the fixture holds complete movement rows");
    };
    rows[0].strategic_redeployment = 1;
    assert_eq!(rendered(&redeploying).title, "total 2.50%\nupkeep 10.00%");
}

#[test]
fn training_and_border_war_lines_follow_the_native_order() {
    let mut frame = frame(5, 0, vec![]);
    status(&mut frame).exercising = 1;
    put(
        &mut frame,
        TableRows::StateFlags(vec![StateFlagRow {
            state: 3,
            border_conflict_active: Recorded::Value(1),
            ..Default::default()
        }]),
    );
    let out = rendered(&frame);
    assert_eq!(
        out.title,
        "total 2.50%\ntrain \u{11}R+5.0%\u{11}!\nborder war\nborder 4.0%"
    );
    assert!(refused(&out, "attrition state"));
    assert!(!refused(&out, "attrition weather"));
    assert!(!refused(&out, "attrition training"));
}

#[test]
fn the_supply_line_follows_the_recorded_supply_and_days() {
    let mut frame = frame(5, 0, vec![]);
    let out = rendered(&frame);
    assert_eq!(out.title, "total 2.50%");
    assert!(!refused(&out, "attrition supply"), "{:?}", out.refused);
    status(&mut frame).supply_ratio = None;
    status(&mut frame).out_of_supply_days = Recorded::Value(Some(15));
    status(&mut frame).supply_ratio_exact = Recorded::Value(Some(10_000));
    status(&mut frame).supply_grace_exact = Recorded::Value(Some(0));
    let out = rendered(&frame);
    assert_eq!(out.title, "total 2.50%\nsupply \u{11}R+7.14%\u{11}!");
    put(
        &mut frame,
        TableRows::CountryCounterInputs(vec![CountryCounterInputRow {
            country: 100,
            movement_modifiers: Recorded::Value(Some(CountryMovementModifiers {
                out_of_supply_factor: Some(-50_000),
                ..Default::default()
            })),
            ..Default::default()
        }]),
    );
    assert_eq!(
        rendered(&frame).title,
        "total 2.50%\nsupply \u{11}R+3.57%\u{11}!"
    );
    let factors = |frame: &mut DecodedFrame, country: Option<i64>, special: Option<i64>| {
        put(
            frame,
            TableRows::CountryCounterInputs(vec![CountryCounterInputRow {
                country: 100,
                movement_modifiers: Recorded::Value(Some(CountryMovementModifiers {
                    out_of_supply_factor: country,
                    special_forces_out_of_supply_factor: special,
                    ..Default::default()
                })),
                ..Default::default()
            }]),
        );
    };
    composition(&mut frame, 0);
    factors(&mut frame, Some(-25_000), None);
    let country = rendered_with_sub_units(&frame).title;
    factors(&mut frame, None, Some(-50_000));
    let special = rendered_with_sub_units(&frame);
    assert!(
        !refused(&special, "attrition supply"),
        "{:?}",
        special.refused
    );
    assert_eq!(special.title, country);
    composition(&mut frame, 1);
    let unreadable = rendered_with_sub_units(&frame);
    assert!(
        unreadable
            .refused
            .keys()
            .any(|key| key.starts_with("counter attrition supply:")
                && key.contains("composition was unreadable")),
        "{:?}",
        unreadable.refused
    );
    status(&mut frame).supply_ratio = Some(10_000);
    status(&mut frame).out_of_supply_days = Recorded::Value(Some(0));
    status(&mut frame).supply_ratio_exact = Recorded::Value(None);
    status(&mut frame).supply_grace_exact = Recorded::Value(None);
    let zero_days = rendered(&frame);
    assert!(
        !refused(&zero_days, "attrition supply"),
        "{:?}",
        zero_days.refused
    );
    status(&mut frame).out_of_supply_days = Recorded::Value(Some(15));
    let missing_exact = rendered(&frame);
    assert!(
        missing_exact.refused.contains_key(
            "counter attrition supply: the division's exact supply was not recorded with its days out of supply"
        ),
        "{:?}",
        missing_exact.refused
    );
    status(&mut frame).out_of_supply_days = Recorded::Value(None);
    let missing_days = rendered(&frame);
    assert!(
        missing_days.refused.contains_key(
            "counter attrition supply: the division's days out of supply were refused when recorded"
        ),
        "{:?}",
        missing_days.refused
    );
}

#[test]
fn a_moving_unit_without_weather_attrition_has_no_weather_lines() {
    let mut frame = frame(5, 2, vec![7, 8]);
    put(
        &mut frame,
        TableRows::WeatherProvinces(vec![WeatherProvinceRow {
            id: 7,
            modifiers: Recorded::Value(Some(Default::default())),
            ..Default::default()
        }]),
    );
    assert!(!refused(&rendered(&frame), "attrition weather"));
    put(
        &mut frame,
        TableRows::WeatherProvinces(vec![WeatherProvinceRow {
            id: 7,
            modifiers: Recorded::Value(Some(replay_data::rows::ProvinceWeatherModifiers {
                entries: vec![replay_data::rows::ProvinceModifierEntry {
                    key: engine_executable::modifier_keys::WINTER_ATTRITION,
                    value: 10_000,
                }],
            })),
            ..Default::default()
        }]),
    );
    assert!(refused(&rendered(&frame), "attrition weather"));
    status(&mut frame).weather_factor = Recorded::Value(Some(0));
    assert!(!refused(&rendered(&frame), "attrition weather"));
}

#[test]
fn missing_content_refuses_the_lines_it_feeds() {
    let mut frame = frame(5, 2, vec![7]);
    status(&mut frame).exercising = 0;
    let out = render(&frame);
    assert_eq!(out.title, "total 2.50%");
    assert!(refused(&out, "attrition terrain"));
    assert!(refused(&out, "attrition supply"));
}

#[test]
fn the_total_needs_a_location_or_a_route_province() {
    let out = render(&frame(0, 0, vec![]));
    assert!(out.title.is_empty());
    assert!(out.refused.is_empty());
    assert_eq!(render(&frame(0, 1, vec![4])).title, "total 2.50%");
    let clipped = render(&frame(0, 2, vec![]));
    assert!(clipped.title.is_empty());
    assert_eq!(clipped.refused.len(), 1);
    assert!(
        clipped
            .refused
            .keys()
            .all(|key| key.starts_with("counter attrition total:"))
    );
}

#[test]
fn a_missing_status_row_refuses_the_total_by_name() {
    let mut frame = frame(5, 0, vec![]);
    put(&mut frame, TableRows::DivisionStatus(vec![]));
    let out = render(&frame);
    assert!(out.title.is_empty());
    assert_eq!(
        out.refused
            .get("counter attrition total: DivisionStatus division 1 is absent")
            .copied(),
        Some(1)
    );
}

#[test]
fn shared_membership_and_country_roots_replace_blanket_counter_refusals() {
    use replay_data::{
        rows::DivisionPanelSourceNode, rows::DivisionPanelSourceRow, rows::DivisionPanelSourceValue,
    };
    let mut frame = frame(5, 1, vec![7]);
    let row = |kind, owner, ordinal, values| {
        std::sync::Arc::new(DivisionPanelSourceRow {
            kind,
            owner,
            ordinal,
            node: Some(DivisionPanelSourceNode {
                values,
                ..Default::default()
            }),
            ..Default::default()
        })
    };
    put(
        &mut frame,
        TableRows::DivisionPanelSources(vec![
            row(0, 100, 0, Vec::new()),
            row(
                3,
                70,
                2,
                vec![DivisionPanelSourceValue { key: 0, raw: 4_000 }],
            ),
            row(5, 1, 0, Vec::new()),
            row(5, 1, 1, vec![DivisionPanelSourceValue { key: 0, raw: 70 }]),
            row(5, 1, 4, vec![DivisionPanelSourceValue { key: 0, raw: 70 }]),
        ]),
    );
    let out = rendered(&frame);
    assert!(out.title.contains("equipment 8.00%"), "{}", out.title);
    for line in [
        "attrition equipment",
        "attrition upkeep modifiers",
        "attrition country modifiers",
    ] {
        assert!(!refused(&out, line), "{line}: {:?}", out.refused);
    }
}

#[test]
fn zero_supply_days_attrition_uses_exact_operands_and_proves_historical_threshold() {
    let mut frame = frame(5, 0, vec![]);
    put(
        &mut frame,
        TableRows::SimulationFeatures(vec![SimulationFeatureRow {
            movement_defines: Recorded::Value(MovementDefines {
                supply_days: Some(0),
                supply_grace: Some(72),
                out_of_supply_speed: Some(-80_000),
                out_of_supply_attrition: Some(20_000),
                ..Default::default()
            }),
            ..Default::default()
        }]),
    );
    status(&mut frame).out_of_supply_days = Recorded::Value(Some(0));
    status(&mut frame).supply_ratio = None;
    status(&mut frame).supply_ratio_exact = Recorded::Value(Some(10_500));
    status(&mut frame).supply_grace_exact = Recorded::Value(Some(0));
    let exact = rendered(&frame);
    assert!(!refused(&exact, "attrition supply"), "{:?}", exact.refused);
    assert!(exact.title.contains("supply "), "{}", exact.title);
    status(&mut frame).supply_ratio = Some(35_000);
    status(&mut frame).supply_ratio_exact = Recorded::Value(None);
    status(&mut frame).supply_grace_exact = Recorded::Value(None);
    let historical_proven = rendered(&frame);
    assert!(
        !refused(&historical_proven, "attrition supply"),
        "{:?}",
        historical_proven.refused
    );
    assert!(!historical_proven.title.contains("supply "), "{}", historical_proven.title);
    status(&mut frame).supply_ratio = Some(34_000);
    let historical_refused = rendered(&frame);
    assert!(
        historical_refused.refused.keys().any(|key| key.contains(
            replay_derive::eta_derive::frame_terms::SUPPLY_DAYS_ZERO
        )),
        "{:?}",
        historical_refused.refused
    );
}
