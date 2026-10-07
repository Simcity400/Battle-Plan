use super::*;
use replay::{
    frame::TableView,
    rows::{
        AttritionBorder, AttritionModifierBreakdownRow, AttritionSupply, AttritionTerrain,
        AttritionWeather, ModifierBreakdownNode, ModifierDefinitionRow, TableId, TableRows,
    },
};

fn input() -> ArmyAttritionInputs {
    let mut sources = [Some(None); engine::carmy_attrition::SOURCE_COUNT];
    sources[COUNTRY_SOURCE] = Some(Some(AttritionModifierSource {
        accumulator: 1,
        key: 1,
        depth: 2,
        scale: 100_000,
    }));
    ArmyAttritionInputs {
        province: Some(Some(5)),
        upkeep: Some(1_000),
        terrain: Some(None),
        state: Some(None),
        supply: Some(AttritionSupply {
            penalty: Some(0),
            attrition_factor: Some(0),
        }),
        country_upkeep_modifier: Some(0),
        moving_state: Some(None),
        weather: Some(None),
        training: Some(false),
        training_attrition: Some(None),
        border: Some(None),
        country_attrition_modifier: Some(0),
        sources,
    }
}

fn frame(trees: Vec<AttritionModifierBreakdownRow>) -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch: 0,
        hour: 0,
        metrics: Default::default(),
        tables: TableId::ALL.map(|table| TableView::Complete(TableRows::empty(table)).into()),
    };
    *frame.tables[TableId::AttritionModifierBreakdowns.index()] =
        TableView::Complete(TableRows::AttritionModifierBreakdowns(trees));
    *frame.tables[TableId::ModifierDefinitions.index()] =
        TableView::Complete(TableRows::ModifierDefinitions(vec![
            ModifierDefinitionRow {
                index: 1,
                flags: dashboard::modifier_text::PERCENT_TIMES_HUNDRED,
                precision: 2,
                ..Default::default()
            },
        ]));
    frame
}

fn root(value: i64) -> AttritionModifierBreakdownRow {
    AttritionModifierBreakdownRow {
        accumulator: 1,
        key: 1,
        nodes: Some(vec![ModifierBreakdownNode {
            value,
            ..Default::default()
        }]),
    }
}

fn render(input: &ArmyAttritionInputs, frame: &DecodedFrame, language: &str) -> Tooltip {
    render_with_ratio(input, frame, language, Ok(None))
}

fn render_with_ratio(
    input: &ArmyAttritionInputs,
    frame: &DecodedFrame,
    language: &str,
    border_ratio: Result<Option<i64>, String>,
) -> Tooltip {
    let mut loc: Localisation = [
        ("ATTRITION_TOTAL", "total $TOT|%2$"),
        ("ATTRITION_UPKEEP", "upkeep $VALUE|%2$"),
        ("ATTRITION_TERRAIN", "terrain $TERR$ $ATT|%2$"),
        ("ATTRITION_EQUIPMENT", "equipment $ATT|%2$"),
        ("ATTRITION_STATE", "state $ATT|%2$"),
        ("ATTRITION_SUPPLY", "supply $ATT$"),
        ("ATTRITION_UPKEEP_MODIFIERS", "upkeep modifiers"),
        ("ATTRITION_WEATHER_MODIFIERS", "weather modifiers"),
        ("ATTRITION_MULMOD_BY", "multiplied $MOD$: $WHY$"),
        ("TRAINING_ATTRITION", "training $VALUE$"),
        ("BORDER_WAR_HEADER", "border"),
        ("BORDER_WAR_ATTRITION", "border attrition $VAL|%2$"),
        ("terrain_key", "authored terrain"),
    ]
    .into_iter()
    .map(|(key, value)| (key.into(), value.into()))
    .collect();
    loc.language = language.into();
    let vision = Vision::sharing(1, []);
    let names = installed_content::division_names::Database::default();
    let units = Err("unused".into());
    let context = Context {
        nato: false,
        content: None,
        dig_in_factor: Err("unused".into()),
        loc: &loc,
        recorded_language: "english",
        icons: &crate::combat::binding::NoIcons,
        names: &names,
        units: &units,
        supply_grace: Err("unused".into()),
        comms_max_distance: Err("unused".into()),
        frame,
        vision: &vision,
        links: Ok(&[]),
    };
    let mut out = Tooltip::default();
    details(&context, input, &mut out, border_ratio);
    out
}

#[test]
fn known_border_gate_keeps_its_heading_when_a_later_factor_read_failed() {
    let mut input = input();
    input.border = Some(Some(AttritionBorder {
        state: Some(1),
        country: Some(1),
        factor: None,
    }));
    let out = render_with_ratio(
        &input,
        &frame(vec![root(0)]),
        "english",
        Err(missing("border.factor")),
    );
    assert_eq!(out.title, "upkeep 1.00%\nborder");
    assert_eq!(out.refused.values().sum::<usize>(), 2);
    assert!(
        out.refused
            .keys()
            .all(|reason| reason.contains("border.factor"))
    );
}

#[test]
fn fresh_attrition_total_uses_the_equipment_floor_but_text_keeps_the_unweighted_sum() {
    let mut input = input();
    input.terrain = Some(Some(AttritionTerrain {
        name: Some(b"terrain_key".to_vec()),
        attrition: Some(2_000),
        equipment: Some(-5_000),
    }));
    let out = render(&input, &frame(vec![root(0)]), "french");
    assert_eq!(
        out.title,
        "total 0.00%\nupkeep 1.00%\nterrain authored terrain 3.00%\nequipment -5.00%"
    );
    assert!(out.refused.is_empty(), "{:?}", out.refused);
}

#[test]
fn attrition_tree_failure_preserves_its_heading_sibling_terms_and_known_total() {
    let mut input = input();
    input.country_upkeep_modifier = Some(10_000);
    input.sources[UPKEEP_SOURCE] = Some(Some(AttritionModifierSource {
        accumulator: 2,
        key: 1,
        depth: -1,
        scale: 100_000,
    }));
    input.training_attrition = Some(Some(500));
    let out = render(
        &input,
        &frame(vec![
            root(0),
            AttritionModifierBreakdownRow {
                accumulator: 2,
                key: 1,
                nodes: None,
            },
        ]),
        "english",
    );
    assert_eq!(
        out.title,
        "total 1.50%\nupkeep 1.00%\nupkeep modifiers\ntraining \u{11}R+0.5%\u{11}!"
    );
    assert_eq!(out.refused.values().sum::<usize>(), 1);
}

#[test]
fn independent_heat_factor_failure_keeps_weather_and_winter_tree_text() {
    let mut input = input();
    input.weather = Some(Some(AttritionWeather {
        acclimatization: Some(100_000),
        attrition: Some(2_000),
        heat: Some(Some(AttritionThermal {
            base: 2_000,
            country_factor: None,
            leader_factor: Some(None),
        })),
        winter: Some(Some(AttritionThermal {
            base: 3_000,
            country_factor: Some(0),
            leader_factor: Some(None),
        })),
    }));
    for index in [
        WEATHER_SOURCE,
        HEAT_SOURCE,
        HEAT_SOURCE + 1,
        WINTER_SOURCE,
        WINTER_SOURCE + 1,
    ] {
        input.sources[index] = Some(Some(AttritionModifierSource {
            accumulator: index as u64 + 10,
            key: 1,
            depth: -1,
            scale: 100_000,
        }));
    }
    let mut trees = vec![root(0)];
    for (index, name, value) in [
        (WEATHER_SOURCE, b"weather".as_slice(), 2_000),
        (HEAT_SOURCE, b"heat", 2_000),
        (HEAT_SOURCE + 1, b"country", 0),
        (WINTER_SOURCE, b"winter", 3_000),
        (WINTER_SOURCE + 1, b"country", 0),
    ] {
        trees.push(AttritionModifierBreakdownRow {
            accumulator: index as u64 + 10,
            key: 1,
            nodes: Some(vec![ModifierBreakdownNode {
                value,
                name: name.to_vec(),
                ..Default::default()
            }]),
        });
    }
    let out = render(&input, &frame(trees), "english");
    assert!(
        out.title
            .contains("weather modifiers\n    weather: \u{11}R+2.00%\u{11}!")
    );
    assert!(out.title.contains(" - winter: \u{11}R+3.00%\u{11}!"));
    assert!(!out.title.starts_with("total"));
    assert_eq!(out.refused.values().sum::<usize>(), 1);
    assert!(
        out.refused
            .keys()
            .next()
            .unwrap()
            .contains("attrition total")
    );
}

#[test]
fn country_breakdown_depth_two_keeps_interior_label_and_language_fault_is_text_local() {
    let mut input = input();
    input.country_attrition_modifier = Some(10_000);
    let mut source = root(10_000);
    source.nodes = Some(vec![
        ModifierBreakdownNode {
            children: 1,
            value: 10_000,
            ..Default::default()
        },
        ModifierBreakdownNode {
            children: 1,
            value: 10_000,
            name: b"branch".to_vec(),
        },
        ModifierBreakdownNode {
            children: 1,
            value: 10_000,
            name: b"interior".to_vec(),
        },
        ModifierBreakdownNode {
            children: 0,
            value: 10_000,
            name: b"deep leaf".to_vec(),
        },
    ]);
    let out = render(&input, &frame(vec![source.clone()]), "english");
    assert_eq!(
        out.title,
        "total 1.10%\nupkeep 1.00%\nmultiplied 1.10: interior: \u{11}R+10.00%\u{11}!"
    );
    let out = render(&input, &frame(vec![source]), "french");
    assert!(out.title.starts_with("total 1.10%\nupkeep 1.00%"));
    assert_eq!(out.refused.values().sum::<usize>(), 1);
}

#[test]
fn completed_attrition_trims_authored_boundaries_when_the_total_is_refused() {
    let mut input = input();
    input.upkeep = Some(0);
    input.country_attrition_modifier = None;
    let mut source = root(10_000);
    source.nodes.as_mut().unwrap()[0].name = b"\n\t\r commander\tname".to_vec();
    let out = render(&input, &frame(vec![source]), "english");
    assert_eq!(out.title, "commander\tname: \u{11}R+10.00%\u{11}!");
    assert_eq!(out.refused.values().sum::<usize>(), 2);
    assert!(out.refused.keys().all(|reason| {
        reason.contains("attrition country multiplier") || reason.contains("attrition total")
    }));
}

#[test]
fn absent_location_skips_the_whole_native_hover_and_zero_coloured_percent_has_a_plus() {
    let mut input = input();
    input.province = Some(None);
    let out = render(&input, &frame(vec![]), "english");
    assert!(out.title.is_empty());
    assert!(out.refused.is_empty());
    assert_eq!(coloured_percent(0, 1), "\u{11}Y+0.0%\u{11}!");
}
