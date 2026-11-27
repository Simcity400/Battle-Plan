use super::*;
use content_reader_map::scripted_map_modes;
use content_reader_map::map::{CountryColors, ProvinceDefinition};
use replay_data::{
    rows::CapitulationRow, rows::CountryAppearanceRow, rows::CountryCounterInputRow,
    rows::CountryLinkRow, rows::CountryRow, rows::FactionMemberRow, rows::FactionRow,
    rows::IntelNetworkRow, rows::LexicalTokenRow, rows::PlayerRow, rows::ProvinceRow,
    rows::Recorded, rows::StateRow, rows::SubjectRow, rows::TableId, rows::TableRows,
};
use replay_frame::{frame::Metrics, frame::TableView};

fn with_empty_character_templates(
    mut facts: dashboard_text::triggers::ContentFacts<'_>,
) -> dashboard_text::triggers::ContentFacts<'_> {
    static ROLES: std::sync::OnceLock<dashboard_text::triggers::LeaderRoles> =
        std::sync::OnceLock::new();
    facts.leader_roles =
        Ok(ROLES.get_or_init(|| dashboard_text::triggers::LeaderRoles::new(Vec::new())));
    facts
}

fn frame_with_empty_country_variables() -> DecodedFrame {
    use replay_data::rows::{ScriptVariablesRow, tables};

    let mut frame = frame();
    let stores = frame
        .complete::<tables::Countries>()
        .unwrap()
        .iter()
        .map(|country| ScriptVariablesRow {
            scope_kind: 1,
            scope_id: country.id,
            seed: None.into(),
            entries: Vec::new(),
        })
        .collect();
    frame.tables[TableId::ScriptVariables.index()] =
        TableView::Complete(TableRows::ScriptVariables(stores)).into();
    frame.tables[TableId::ScriptVariableNames.index()] =
        TableView::Complete(TableRows::ScriptVariableNames(Vec::new())).into();
    frame.tables[TableId::ScriptVariableValues.index()] =
        TableView::Complete(TableRows::ScriptVariableValues(Vec::new())).into();
    frame
}

fn defines() -> ModeDefines {
    ModeDefines {
        factions_not_member: [0.7, 0.7, 0.7, 0.7],
        factions_member_alpha: 1.0,
        players_other_alpha: 0.15,
        selected_fill: 1.5,
        ideology_alpha: 1.0,
        country_channel: [0.0, 0.973],
        factions_channel: [0.0, 0.973],
        state_channel: [0.4, 0.973],
        diplomacy_channel: [0.0, 0.973],
        resistance_cutoff: 0.973,
        state_thickness: 5.0,
        diplomacy_thickness: 12.0,
        resistance_thickness: 5.0,
        scripted_thickness: 25.0,
        very_good_opinion: 47,
        very_bad_opinion: -47,
        resistance_colors: ColorList(vec![
            0.0, 0.0, 1.0, 0.0, 0.0,
            0.0, 1.0, 1.0, 0.0, 0.0,
            1.0, 1.0, 1.0, 0.0, 0.1,
            30.0, 1.0, 1.0, 0.0, 0.3,
            100.0, 1.0, 0.0, 0.0, 0.3,
        ]),
        compliance_colors: ColorList(vec![0.0, 0.3, 0.6, 0.6, 0.05, 100.0, 0.3, 0.9, 1.0, 0.5]),
        resistance_uninitialised: [1.0, 1.0, 1.0, 0.05],
        manpower_max: 20_000_000.0,
        manpower_colors: [[0.2, 0.2, 0.7, 0.18], [1.0, 0.125, 0.0, 0.6]],
        network_colors: [[0.1, 0.1, 0.5, 0.2], [0.4, 0.3, 0.9, 1.0]],
        quiet_network_colors: [[0.1, 0.5, 0.1, 0.2], [0.4, 0.9, 0.3, 1.0]],
        infrastructure_colors: [[1.0, 0.125, 0.0, 0.1], [0.1, 0.9, 0.1, 0.6]],
        configurable_slots: [4, 10],
        terrain: terrain::Defines {
            alpha: [0.5, 0.8],
            channel: [0.39, 0.973],
        },
    }
}

pub(super) fn content(scripted: MapModes) -> Content {
    Content {
        memberships: Default::default(),
        styles: Default::default(),
        scripts: Default::default(),
        id: viewer_scene_base::next_projection_id(),
        defines: defines(),
        ideologies: Arc::new(content_reader_key_names::ideologies::Ideologies::default()),
        category_colors: BTreeMap::from([
            (b"town".to_vec(), Ok(Some([0.0, 200.0 / 255.0, 0.0, 1.0]))),
            (b"rural".to_vec(), Ok(None)),
            (b"wasteland".to_vec(), Err("no colour".to_owned())),
        ]),
        scripted,
        faction_templates: Ok(Arc::new(Default::default())),
        recorded_language: "english".to_owned(),
        terrain: Err("the terrain database did not read".to_owned()),
        infrastructure: infrastructure::Content::default(),
        strategic: strategic::Content::fixture(),
    }
}

pub(super) fn map() -> MapData {
    let kinds = [
        (1, ProvinceKind::Land),
        (2, ProvinceKind::Land),
        (3, ProvinceKind::Land),
        (4, ProvinceKind::Land),
        (5, ProvinceKind::Sea),
    ];
    MapData {
        width: 5,
        height: 1,
        province_ids: vec![1, 2, 3, 4, 5],
        definitions: kinds
            .into_iter()
            .map(|(id, kind)| {
                (
                    id,
                    ProvinceDefinition {
                        id,
                        rgb: [id as u8, 0, 0],
                        kind,
                        continent: 0,
                    },
                )
            })
            .collect(),
        country_colors: CountryColors::default(),
    }
}

fn appearance(country: u32, rgba: [f32; 4]) -> CountryAppearanceRow {
    CountryAppearanceRow {
        country,
        red: rgba[0].to_bits(),
        green: rgba[1].to_bits(),
        blue: rgba[2].to_bits(),
        alpha: rgba[3].to_bits(),
        ..Default::default()
    }
}

pub(super) fn frame() -> DecodedFrame {
    let mut tables = std::array::from_fn(|_| TableView::Unavailable {
        reason: "not needed".into(),
    });
    let countries = [(100, 0, "GER"), (200, 1, "SOV"), (300, 2, "ITA")];
    tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        countries
            .iter()
            .map(|&(id, slot, tag)| CountryRow {
                id,
                canonical_slot: slot,
                tag: tag.as_bytes().to_vec(),
                ..Default::default()
            })
            .collect(),
    ));
    tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        countries
            .iter()
            .map(|&(id, slot, _)| CountryLinkRow {
                id,
                canonical_slot: slot,
            })
            .collect(),
    ));
    tables[TableId::CountryCounterInputs.index()] =
        TableView::Complete(TableRows::CountryCounterInputs(
            countries
                .iter()
                .map(|&(id, _, _)| CountryCounterInputRow {
                    country: id,
                    owned_state_count: 1,
                    ..Default::default()
                })
                .collect(),
        ));
    let province = |id, state, country| ProvinceRow {
        id,
        state,
        owner: country,
        controller: country,
        ..Default::default()
    };
    tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(vec![
        province(1, 1, 100),
        province(2, 1, 100),
        province(3, 2, 200),
        province(4, 3, 300),
        province(5, 0, 0),
    ]));
    tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(vec![
            appearance(100, [0.75, 0.5, 0.25, 1.0]),
            appearance(200, [0.5, 0.0, 0.25, 1.0]),
            appearance(300, [0.0, 0.5, 0.0, 1.0]),
        ]));
    tables[TableId::Autonomy.index()] = TableView::Complete(TableRows::Autonomy(Vec::new()));
    tables[TableId::Subjects.index()] = TableView::Complete(TableRows::Subjects(Vec::new()));
    tables[TableId::Factions.index()] =
        TableView::Complete(TableRows::Factions(vec![FactionRow {
            id: 9,
            color_red: 0.2f32.to_bits(),
            color_green: 0.4f32.to_bits(),
            color_blue: 0.6f32.to_bits(),
            color_alpha: 1.0f32.to_bits(),
            ..Default::default()
        }]));
    tables[TableId::FactionMembers.index()] = TableView::Complete(TableRows::FactionMembers(vec![
        FactionMemberRow {
            country: 100,
            faction: 9,
            ordinal: 0,
        },
        FactionMemberRow {
            country: 300,
            faction: 9,
            ordinal: 1,
        },
    ]));
    tables[TableId::States.index()] = TableView::Complete(TableRows::States(
        (1..=3)
            .map(|id| StateRow {
                id,
                category: Some(11).into(),
                ..Default::default()
            })
            .collect(),
    ));
    tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 11,
            literal: b"town".to_vec(),
        }]));
    tables[TableId::Players.index()] = TableView::Complete(TableRows::Players(vec![PlayerRow {
        id: 1,
        country: 200,
        ..Default::default()
    }]));
    tables[TableId::MapEvents.index()] = {
        use replay_data::{rows::MapEventFamily, rows::MapEventFamilyHealth, rows::MapEventsRow};
        TableView::Complete(TableRows::MapEvents(vec![MapEventsRow {
            from_hour: 42,
            to_hour: 42,
            first_sequence: 1,
            next_sequence: 1,
            health: [MapEventFamilyHealth {
                complete: true,
                enabled: true,
                installed: true,
                ..Default::default()
            }; MapEventFamily::COUNT],
            ..Default::default()
        }]))
    };
    DecodedFrame {
        branch: 1,
        hour: 42,
        tables: tables.map(Into::into),
        metrics: Metrics::default(),
    }
}

fn run(mode: MapMode, content: &Content, selected: Option<u32>) -> ModeFill {
    checked_fill(
        &mode,
        &Inputs {
            content,
            map: &map(),
            frame: &frame(),
            selected,
            world: None,
            colour: None,
        },
    )
    .unwrap()
}

#[test]
fn tokens_round_trip_and_unknown_tokens_are_refused() {
    for stock in Stock::ALL {
        assert_eq!(MapMode::parse(stock.token()), Some(MapMode::Stock(stock)));
    }
    let scripted = MapMode::Scripted("opinion_map_mode".into());
    assert_eq!(MapMode::parse(&scripted.token()), Some(scripted));
    assert_eq!(MapMode::parse("scripted:"), None);
    assert_eq!(MapMode::parse("political"), None);
}

#[test]
fn colour_lists_interpolate_between_the_rows_around_the_value() {
    let list = &defines().resistance_colors;
    assert_eq!(list.color(0.0), [0.0, 1.0, 0.0, 0.0]);
    assert_eq!(list.color(0.5), [1.0, 1.0, 0.0, 0.05]);
    assert_eq!(list.color(65.0), [1.0, 0.5, 0.0, 0.3]);
    assert_eq!(list.color(200.0), [1.0, 0.0, 0.0, 0.3]);
    assert_eq!(list.color(-5.0), [0.0, 1.0, 0.0, 0.0]);
    assert_eq!(ColorList(vec![0.0; 7]).color(1.0), [1.0; 4]);
}

#[test]
fn factions_colour_members_by_their_faction_and_others_by_the_define() {
    let fill = run(
        MapMode::Stock(Stock::Factions),
        &content(MapModes::default()),
        Some(0),
    );
    let draw = fill.draw.unwrap();
    assert_eq!(draw.channels, [Source::Country, Source::Empty]);
    let layer = draw.country.unwrap();
    assert_eq!(layer.region_of, [0, 1, 1, 2, 3, 0]);
    assert_eq!(layer.styles[&1].color, [51, 102, 153, 255]);
    assert_eq!(layer.styles[&3].color, [51, 102, 153, 255]);
    assert_eq!(layer.styles[&2].color, [178, 178, 178, 178]);
    assert_eq!(layer.thickness, None);
}

#[test]
fn states_take_their_category_colour_and_name_what_they_cannot_draw() {
    let with = |categories: [Recorded<Option<u32>>; 3]| {
        let mut frame = frame();
        frame.tables[TableId::States.index()] = TableView::Complete(TableRows::States(
            categories
                .into_iter()
                .zip(1..)
                .map(|(category, id)| StateRow {
                    id,
                    category,
                    ..Default::default()
                })
                .collect(),
        ))
        .into();
        frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(
                [(11, "Town"), (12, "rural"), (13, "wasteland")]
                    .into_iter()
                    .map(|(token, literal)| LexicalTokenRow {
                        token,
                        literal: literal.as_bytes().to_vec(),
                    })
                    .collect(),
            ))
            .into();
        checked_fill(
            &MapMode::Stock(Stock::States),
            &Inputs {
                content: &content(MapModes::default()),
                map: &map(),
                frame: &frame,
                selected: None,
                world: None,
                colour: None,
            },
        )
        .unwrap()
    };
    let fill = with([Some(11).into(), Some(12).into(), Some(13).into()]);
    let draw = fill.draw.unwrap();
    assert_eq!(draw.channels, [Source::Country, Source::Mode]);
    assert_eq!(draw.country, None);
    assert_eq!(
        draw.channel_constants(),
        [Some([0.0, 0.973]), Some([0.4, 0.973])]
    );
    let layer = draw.layer.unwrap();
    assert_eq!(layer.region_of, [0, 2, 2, 3, 0, 0]);
    assert_eq!(layer.styles[&2].color, [0, 200, 0, 255]);
    assert_eq!(layer.styles[&3].color, [0, 0, 255, 63]);
    assert!(!layer.styles.contains_key(&4));
    assert_eq!(layer.thickness, Some(5.0));
    assert_eq!(layer.channel, [0.4, 0.973]);
    assert_eq!(
        fill.warnings,
        [
            "The states map mode leaves 1 states unfilled whose category the content does not declare or colour"
        ]
    );
    let moved = with([Some(12).into(), Some(12).into(), None.into()]);
    let layer = moved.draw.unwrap().layer.unwrap();
    assert_eq!(layer.styles[&2].color, [0, 0, 255, 63]);
    assert_eq!(
        moved.warnings,
        ["The states map mode leaves 1 states unfilled whose category read was refused"]
    );
    let unrecorded = with([Recorded::NotRecorded, Some(12).into(), Some(12).into()]);
    assert_eq!(
        unrecorded.draw.unwrap().channels,
        [Source::Country, Source::Empty]
    );
}

#[test]
fn diplomacy_takes_the_first_relation_that_holds() {
    let defines = defines();
    let all = Relation {
        selected: false,
        at_war: true,
        same_faction: true,
        war_goal: Some(true),
        opinion: 99,
    };
    assert_eq!(all.color(&defines), Ok(Some(DIPLOMACY_WAR)));
    let faction = Relation {
        at_war: false,
        ..all
    };
    assert_eq!(faction.color(&defines), Ok(Some(DIPLOMACY_FACTION)));
    let goal = Relation {
        same_faction: false,
        ..faction
    };
    assert_eq!(goal.color(&defines), Ok(Some(DIPLOMACY_WAR_GOAL)));
    let good = Relation {
        war_goal: Some(false),
        ..goal
    };
    assert_eq!(good.color(&defines), Ok(Some(DIPLOMACY_GOOD)));
    assert_eq!(
        Relation {
            opinion: defines.very_good_opinion,
            ..good
        }
        .color(&defines),
        Ok(None)
    );
    assert_eq!(
        Relation {
            opinion: defines.very_bad_opinion - 1,
            ..good
        }
        .color(&defines),
        Ok(Some(DIPLOMACY_BAD))
    );
    assert_eq!(
        Relation {
            war_goal: None,
            ..good
        }
        .color(&defines),
        Err(())
    );
    assert_eq!(
        Relation {
            war_goal: None,
            ..faction
        }
        .color(&defines),
        Ok(Some(DIPLOMACY_FACTION))
    );
    assert_eq!(
        Relation {
            selected: true,
            ..all
        }
        .color(&defines),
        Ok(Some(DIPLOMACY_SELF))
    );
}

#[test]
fn manpower_blends_each_states_population_over_the_range_and_clamps_it() {
    let with = |populations: [Recorded<i32>; 3]| {
        let mut frame = frame();
        frame.tables[TableId::States.index()] = TableView::Complete(TableRows::States(
            populations
                .into_iter()
                .zip(1..)
                .map(|(population, id)| StateRow {
                    id,
                    population,
                    ..Default::default()
                })
                .collect(),
        ))
        .into();
        checked_fill(
            &MapMode::Stock(Stock::Manpower),
            &Inputs {
                content: &content(MapModes::default()),
                map: &map(),
                frame: &frame,
                selected: Some(0),
                world: None,
                colour: None,
            },
        )
        .unwrap()
    };
    let fill = with([
        Recorded::Value(5_000_000),
        Recorded::Value(40_000_000),
        Recorded::Value(-3),
    ]);
    let draw = fill.draw.unwrap();
    assert_eq!(draw.channels, [Source::Mode, Source::Country]);
    let layer = draw.layer.unwrap();
    assert_eq!(layer.styles[&2].color, pack([0.4, 0.18125, 0.525, 0.285]));
    assert_eq!(layer.styles[&3].color, pack([1.0, 0.125, 0.0, 0.6]));
    assert_eq!(layer.styles[&4].color, pack([0.2, 0.2, 0.7, 0.18]));
    assert_eq!(layer.gradient, Some(LEVEL_GRADIENT));
    assert_eq!(layer.thickness, Some(5.0));
    assert_eq!(layer.channel, [0.0, 0.973]);
    assert_eq!(&layer.region_of[1..6], &[2, 2, 3, 4, 0]);
    let unrecorded = with([
        Recorded::Value(1),
        Recorded::NotRecorded,
        Recorded::Value(1),
    ]);
    assert_eq!(
        unrecorded.draw.unwrap().channels,
        [Source::Empty, Source::Country]
    );
    assert_eq!(unrecorded.warnings.len(), 1);
}

#[test]
fn operatives_colour_the_selected_countrys_network_states_its_targets_control() {
    let network = |owner, target, state_id, strength, quiet| IntelNetworkRow {
        owner_country: owner,
        target_country: target,
        state_id,
        local_strength_raw: strength,
        local_quiet: quiet,
        ..Default::default()
    };
    let mut frame = frame();
    frame.tables[TableId::States.index()] = TableView::Complete(TableRows::States(
        [(1, 100), (2, 200), (3, 300)]
            .into_iter()
            .map(|(id, controller)| StateRow {
                id,
                controller,
                ..Default::default()
            })
            .collect(),
    ))
    .into();
    frame.tables[TableId::IntelNetworks.index()] =
        TableView::Complete(TableRows::IntelNetworks(vec![
            network(200, 100, 1, 7_250_000, 0),
            network(200, 100, 2, 10_000_000, 0),
            network(200, 300, 3, 3_000_000, 1),
            network(200, 300, 99, 3_000_000, 0),
            network(100, 200, 2, 10_000_000, 0),
        ]))
        .into();
    let content = content(MapModes::default());
    let run = |frame: &DecodedFrame, selected| {
        checked_fill(
            &MapMode::Stock(Stock::Operatives),
            &Inputs {
                content: &content,
                map: &map(),
                frame,
                selected,
                world: None,
                colour: None,
            },
        )
        .unwrap()
    };
    let fill = run(&frame, Some(1));
    let draw = fill.draw.unwrap();
    assert_eq!(draw.channels, [Source::Country, Source::Mode]);
    let layer = draw.layer.unwrap();
    assert_eq!(
        layer.styles[&2].color,
        pack([0.2576875, 0.205125, 0.71025, 0.6205])
    );
    assert_eq!(layer.styles[&4].color, pack([0.145, 0.56, 0.13, 0.32]));
    assert_eq!(layer.styles.len(), 2);
    assert_eq!(&layer.region_of[1..6], &[2, 2, 0, 4, 0]);
    assert_eq!(layer.gradient, Some(LEVEL_GRADIENT));
    assert_eq!(layer.thickness, Some(5.0));
    assert_eq!(layer.channel, [0.0, 0.973]);
    assert_eq!(fill.warnings.len(), 1, "{:?}", fill.warnings);
    assert!(
        !fill
            .warnings
            .iter()
            .any(|why| why.contains("icons are not drawn"))
    );
    let defines = defines();
    assert_eq!(
        network_color(&defines, 15_000_000, false),
        [0.4, 0.3, 0.9, 1.0]
    );
    assert_eq!(
        network_color(&defines, 10_000_000, true),
        [0.4, 0.9, 0.3, 1.0]
    );
    assert_eq!(network_color(&defines, 0, false), [0.1, 0.1, 0.5, 0.2]);
    let german = run(&frame, Some(0)).draw.unwrap().layer.unwrap();
    assert_eq!(german.styles.keys().copied().collect::<Vec<_>>(), [3]);
    let refused = run(&frame, None);
    assert_eq!(
        refused.draw.unwrap().channels,
        [Source::Country, Source::Empty]
    );
    assert_eq!(refused.warnings.len(), 1);
    let mut unavailable = frame;
    unavailable.tables[TableId::IntelNetworks.index()] = TableView::Unavailable {
        reason: "not recorded".into(),
    }
    .into();
    let refused = run(&unavailable, Some(1));
    assert_eq!(
        refused.draw.unwrap().channels,
        [Source::Country, Source::Empty]
    );
    assert!(
        refused
            .warnings
            .iter()
            .any(|why| why.contains("IntelNetworks"))
    );
}

#[test]
fn a_war_goal_marks_its_recipient_and_the_recipients_overlord() {
    let with = |recipients: Recorded<Option<Vec<u32>>>| {
        let mut frame = frame();
        frame.tables[TableId::Capitulation.index()] =
            TableView::Complete(TableRows::Capitulation(vec![CapitulationRow {
                country: 100,
                war_goal_recipients: recipients,
                ..Default::default()
            }]))
            .into();
        frame.tables[TableId::Subjects.index()] =
            TableView::Complete(TableRows::Subjects(vec![SubjectRow {
                overlord: 300,
                subject: 200,
                ..Default::default()
            }]))
            .into();
        let countries = Countries::read(&frame).unwrap();
        war_goal_slots(&frame, &countries, 100)
    };
    assert_eq!(
        with(Recorded::Value(Some(vec![200]))),
        Ok(BTreeSet::from([1, 2]))
    );
    assert_eq!(with(Recorded::Value(Some(Vec::new()))), Ok(BTreeSet::new()));
    assert_eq!(
        with(Recorded::NotRecorded),
        Err(WAR_GOALS_UNRECORDED.to_owned())
    );
    assert!(with(Recorded::Value(None)).is_err());
}

#[test]
fn each_mode_chooses_whose_names_the_map_writes_far_and_near() {
    let mut modes = MapModes::default();
    scripted_map_modes::parse_into(
        &"test.txt".into(),
        br#"scripted_map_modes = {
            spheres = { top = { type = game_map_mode_factions } far_text = faction near_text = state }
            plain = { top = { type = game_map_mode_states } far_text = none }
            unset = { top = { type = game_map_mode_states } }
            players = { top = { type = game_map_mode_players } far_text = player }
            close = { top = { type = game_map_mode_states } near_text = country }
        }"#,
        &mut modes,
    )
    .unwrap();
    let content = content(modes);
    let layers = |mode: MapMode| content.name_layers(&mode);
    let far = |mode: MapMode| layers(mode).far;
    assert_eq!(far(MapMode::Stock(Stock::Factions)), NameLayer::Factions);
    assert_eq!(far(MapMode::Stock(Stock::Default)), NameLayer::Countries);
    assert_eq!(far(MapMode::Stock(Stock::Resistance)), NameLayer::Countries);
    assert_eq!(far(MapMode::Stock(Stock::Players)), NameLayer::Players);
    assert_eq!(far(MapMode::Scripted("players".into())), NameLayer::Players);
    for stock in [Stock::StrategicAir, Stock::StrategicNavy] {
        assert_eq!(far(MapMode::Stock(stock)), NameLayer::StrategicRegions);
    }
    for stock in [
        Stock::Default,
        Stock::Supply,
        Stock::Resistance,
        Stock::Compliance,
        Stock::Resources,
    ] {
        assert_eq!(layers(MapMode::Stock(stock)).near, NameLayer::Nothing);
    }
    for stock in [
        Stock::States,
        Stock::Diplomacy,
        Stock::Infrastructure,
        Stock::Manpower,
        Stock::Ideology,
        Stock::Terrain,
        Stock::Operatives,
    ] {
        assert_eq!(layers(MapMode::Stock(stock)).near, NameLayer::States);
    }
    for stock in [
        Stock::Factions,
        Stock::Players,
        Stock::Raids,
        Stock::StrategicAir,
        Stock::StrategicNavy,
    ] {
        let names = layers(MapMode::Stock(stock));
        assert_eq!(names.near, names.far);
    }
    assert_eq!(
        layers(MapMode::Scripted("spheres".into())),
        NameLayers {
            far: NameLayer::Factions,
            near: NameLayer::States
        }
    );
    assert_eq!(far(MapMode::Scripted("plain".into())), NameLayer::Nothing);
    assert_eq!(
        layers(MapMode::Scripted("unset".into())),
        NameLayers {
            far: NameLayer::Nothing,
            near: NameLayer::Nothing
        }
    );
    assert_eq!(
        layers(MapMode::Scripted("close".into())),
        NameLayers {
            far: NameLayer::Nothing,
            near: NameLayer::Countries
        }
    );
}

#[test]
fn a_scripted_mode_shows_two_game_layers_or_one_in_both_channels() {
    let mut modes = MapModes::default();
    scripted_map_modes::parse_into(
        &"test.txt".into(),
        br#"scripted_map_modes = {
            both = { bottom = { type = game_map_mode_states } top = { type = game_map_mode_diplomacy } }
            twice = { bottom = { type = game_map_mode_states } top = { type = game_map_mode_states } }
        }"#,
        &mut modes,
    )
    .unwrap();
    let content = content(modes);
    let mut frame = frame();
    frame.tables[TableId::RelationOpinions.index()] =
        TableView::Complete(TableRows::RelationOpinions(Vec::new())).into();
    let run = |name: &str| {
        checked_fill(
            &MapMode::Scripted(name.into()),
            &Inputs {
                content: &content,
                map: &map(),
                frame: &frame,
                selected: Some(0),
                world: None,
                colour: None,
            },
        )
        .unwrap()
        .draw
        .unwrap()
    };
    let both = run("both");
    assert_eq!(both.channels, [Source::Mode, Source::Second]);
    assert!(both.layer.is_some() && both.second.is_some());
    assert_ne!(both.layer, both.second);
    let constants = both.channel_constants();
    assert_eq!(constants[0], both.layer.as_ref().map(|layer| layer.channel));
    assert_eq!(
        constants[1],
        both.second.as_ref().map(|layer| layer.channel)
    );
    let twice = run("twice");
    assert_eq!(twice.channels, [Source::Mode, Source::Mode]);
    assert!(twice.second.is_none());
}

#[test]
fn scripted_modes_delegate_game_layers_and_refuse_colour_scripts() {
    let mut modes = MapModes::default();
    scripted_map_modes::parse_into(
        &"test.txt".into(),
        br#"scripted_map_modes = {
            delegated = { top = { type = game_map_mode_factions } }
            scripted = { top = { type = country color = { set_temp_variable = { red = 1 } } } bottom = { type = state } }
            blank = { top = { type = none } }
            layered = { bottom = { type = game_map_mode_country } top = { type = game_map_mode_states } }
        }"#,
        &mut modes,
    )
    .unwrap();
    let content = content(modes);
    let delegated = run(MapMode::Scripted("delegated".into()), &content, Some(0))
        .draw
        .unwrap();
    assert_eq!(delegated.channels, [Source::Empty, Source::Country]);
    assert_eq!(
        delegated.country,
        run(MapMode::Stock(Stock::Factions), &content, Some(0))
            .draw
            .unwrap()
            .country
    );
    let scripted = run(MapMode::Scripted("scripted".into()), &content, Some(0));
    assert_eq!(scripted.draw.unwrap().channels, [Source::Empty; 2]);
    assert_eq!(scripted.warnings.len(), 2, "{:?}", scripted.warnings);
    let blank = run(MapMode::Scripted("blank".into()), &content, Some(0));
    assert_eq!(blank.draw.unwrap().channels, [Source::Empty; 2]);
    let layered = run(MapMode::Scripted("layered".into()), &content, Some(0))
        .draw
        .unwrap();
    assert_eq!(layered.channels, [Source::Country, Source::Mode]);
    assert!(layered.layer.is_some());
    let missing = run(MapMode::Scripted("absent".into()), &content, Some(0));
    assert!(missing.draw.is_none() && missing.warnings[0].contains("absent"));
}

#[test]
fn colour_scripts_paint_each_target_their_callback_colours() {
    viewer_scene_base::test_game::enter();
    use dashboard_text::triggers::{ContentFacts, World};
    let mut modes = MapModes::default();
    scripted_map_modes::parse_into(
        &"test.txt".into(),
        br#"scripted_map_modes = {
            painted = {
                top = {
                    type = country
                    thickness = yes
                    color = {
                        set_temp_variable = { alpha = 0.5 }
                        FROM = { tag = SOV }
                        set_temp_variable = { red = 2 }
                        set_temp_variable = { highlighted = 1 }
                        set_temp_variable = { thickness = 3 }
                    }
                }
                bottom = { type = state color = { set_temp_variable = { blue = 1 } } }
            }
            refused = { top = { type = country color = { has_stability > 0.5 } } }
        }"#,
        &mut modes,
    )
    .unwrap();
    let content = content(modes);
    let frame = frame();
    let names = content_reader_script::trigger_keys::TriggerNames {
        scripted: Default::default(),
        counts: Some(Default::default()),
    };
    let dlcs = BTreeSet::new();
    let aliases = Arc::default();
    let world = World::read(
        &frame,
        with_empty_character_templates(ContentFacts::content_only(&dlcs, &names, &aliases)),
    );
    let map = map();
    let run = |name: &str| {
        checked_fill(
            &MapMode::Scripted(name.into()),
            &Inputs {
                content: &content,
                map: &map,
                frame: &frame,
                selected: Some(0),
                world: Some(&world),
                colour: None,
            },
        )
        .unwrap()
    };
    let painted = run("painted");
    let draw = painted.draw.unwrap();
    assert_eq!(
        draw.channels,
        [Source::Mode, Source::Second],
        "{:?}",
        painted.warnings
    );
    let top = draw.second.unwrap();
    let painted_provinces: Vec<u32> = (0..top.region_of.len() as u32)
        .filter(|&province| top.region_of[province as usize] != 0)
        .collect();
    assert_eq!(painted_provinces, [3], "{:?}", painted.warnings);
    assert_eq!(
        top.styles.values().cloned().collect::<Vec<_>>(),
        [Style {
            color: pack([1.0, 0.0, 0.0, 0.5]),
            pulse: true,
            fill: 3.0,
        }]
    );
    assert_eq!((top.thickness, top.gradient), (Some(25.0), None));
    let bottom = draw.layer.unwrap();
    assert!(bottom.styles.is_empty());
    assert_eq!(bottom.gradient, Some(colour_script::FLAT_GRADIENT));
    let refused = run("refused");
    assert!(
        refused
            .warnings
            .iter()
            .any(|warning| warning.contains("country stability inputs")
                && warning.contains("3 targets")),
        "{:?}",
        refused.warnings
    );
}

#[test]
fn scripted_politics_refuses_uncertified_sharing_rosters_by_name_and_target_count() {
    use dashboard_text::triggers::{ContentFacts, World};
    use replay_data::{
        rows::CountryStabilityInputRow, rows::HumanControllerRow, rows::RaidCountryInputRow,
        rows::ResearchModifierRow, rows::SimulationFeatureRow, rows::TableSchema,
    };
    viewer_scene_base::test_game::enter();
    fn set<T: TableSchema>(f: &mut DecodedFrame, rows: Vec<T::Row>) {
        f.tables[T::ID.index()] = TableView::Complete(TableRows::from_rows::<T>(rows)).into();
    }
    let mut modes = MapModes::default();
    scripted_map_modes::parse_into(
        &"test.txt".into(),
        br#"scripted_map_modes = {
        politics = { update_daily = no top = { type = country color = {
            FROM = {
                set_temp_variable = { alpha = 1 }
                if = { limit = { has_stability > 0.5 has_war_support > 0.1 is_ai = yes }
                    set_temp_variable = { green = 1 }
                } else = { set_temp_variable = { red = 1 } }
                if = { limit = { is_in_tech_sharing_group = synthetic_sharing_group }
                    set_temp_variable = { red = 0 } set_temp_variable = { green = 0 }
                    set_temp_variable = { blue = 1 }
                }
            }
        } } }
    }"#,
        &mut modes,
    )
    .unwrap();
    let content = content(modes);
    let map = map();
    let mut f = frame();
    set::<tables::CountryStabilityInputs>(
        &mut f,
        [100, 200, 300]
            .into_iter()
            .map(|country| CountryStabilityInputRow {
                country,
                stability_raw: 70_000,
                war_support_raw: 30_000,
                stability_factor: Some(0),
                party_popularity_stability_factor: Some(0),
                max_stability: 100_000,
                war_support_factor: Some(0),
                max_war_support: 100_000,
                ..Default::default()
            })
            .collect(),
    );
    set::<tables::HumanControllers>(
        &mut f,
        [100, 200, 300]
            .into_iter()
            .map(|country| HumanControllerRow {
                country,
                count: i8::from(country == 100),
            })
            .collect(),
    );
    set::<tables::RaidCountryInputs>(
        &mut f,
        (0..3)
            .map(|canonical_slot| RaidCountryInputRow {
                canonical_slot,
                autonomous: Some(1),
                ..Default::default()
            })
            .collect(),
    );
    set::<tables::SimulationFeatures>(&mut f, vec![SimulationFeatureRow::default()]);
    let names = content_reader_script::trigger_keys::TriggerNames::default();
    let dlcs = BTreeSet::new();
    let aliases = Arc::default();
    let render = |f: &DecodedFrame| {
        let world = World::read(
            f,
            with_empty_character_templates(ContentFacts::content_only(&dlcs, &names, &aliases)),
        );
        checked_fill(
            &MapMode::Scripted("politics".into()),
            &Inputs {
                content: &content,
                map: &map,
                frame: f,
                selected: Some(0),
                world: Some(&world),
                colour: None,
            },
        )
        .unwrap()
    };
    for known_name in [false, true] {
        set::<tables::LexicalTokens>(
            &mut f,
            if known_name {
                vec![LexicalTokenRow {
                    token: 9,
                    literal: b"synthetic_sharing_group".to_vec(),
                }]
            } else {
                Vec::new()
            },
        );
        for missing_witness in [false, true] {
            if missing_witness {
                set::<tables::ResearchModifiers>(&mut f, Vec::new());
            } else {
                f.tables[TableId::ResearchModifiers.index()] = TableView::Unavailable {
                    reason: "shared research certificate poisoned".into(),
                }
                .into();
            }
            let refused = render(&f);
            let roster_refusals: Vec<_> = refused
                .warnings
                .iter()
                .filter(|why| why.contains("research modifiers"))
                .collect();
            if missing_witness {
                assert_eq!(roster_refusals.len(), 3, "{:?}", refused.warnings);
                for country in 0..3 {
                    assert_eq!(
                        roster_refusals
                            .iter()
                            .filter(|why| why.contains(&format!("of country {country}"))
                                && why.contains("1 targets"))
                            .count(),
                        1,
                        "{:?}",
                        refused.warnings
                    );
                }
            } else {
                assert_eq!(roster_refusals.len(), 1, "{:?}", refused.warnings);
                assert!(
                    roster_refusals[0].contains("3 targets"),
                    "{:?}",
                    refused.warnings
                );
            }
            let kept = refused.colour.unwrap();
            assert!(
                kept.layers[1]
                    .borders
                    .values()
                    .all(|border| border.style.is_none())
            );
        }
    }
    let mut roster = Vec::new();
    for country in [100, 200, 300] {
        roster.push(ResearchModifierRow {
            country,
            index: u32::MAX,
            kind: ResearchModifierRow::SHARING_ROSTER,
            raw: -1,
            count: u32::from(country == 100),
            ..Default::default()
        });
        if country == 100 {
            roster.push(ResearchModifierRow {
                country,
                index: u32::MAX,
                ordinal: 1,
                kind: ResearchModifierRow::SHARING_ROSTER,
                raw: 9,
                ..Default::default()
            });
        }
    }
    set::<tables::ResearchModifiers>(&mut f, roster);
    let painted = render(&f);
    assert!(painted.warnings.is_empty(), "{:?}", painted.warnings);
    let kept = painted.colour.unwrap();
    assert_eq!(kept.layers[1].borders.len(), 3);
    for (id, border) in &kept.layers[1].borders {
        let expected = if *id == colour_script::TargetId::Country(0) {
            [0.0, 0.0, 1.0, 1.0]
        } else {
            [0.0, 1.0, 0.0, 1.0]
        };
        assert_eq!(
            border.style.as_ref().map(|style| style.color),
            Some(pack(expected)),
            "{id:?}"
        );
    }
}

#[test]
fn scripted_callbacks_count_one_roster_input_across_iterator_and_array_reads() {
    use dashboard_text::triggers::{ContentFacts, World};
    viewer_scene_base::test_game::enter();
    let content = content(MapModes::default());
    let map = map();
    let mut f = frame_with_empty_country_variables();
    if let TableView::Complete(TableRows::CountryCounterInputs(rows)) =
        &mut *f.tables[TableId::CountryCounterInputs.index()]
    {
        for row in rows {
            row.owned_states = None.into();
        }
    }
    let names = content_reader_script::trigger_keys::TriggerNames::default();
    let dlcs = BTreeSet::new();
    let aliases = Arc::default();
    let world = World::read(
        &f,
        with_empty_character_templates(ContentFacts::content_only(&dlcs, &names, &aliases)),
    );
    let inputs = Inputs {
        content: &content,
        map: &map,
        frame: &f,
        selected: Some(0),
        world: Some(&world),
        colour: None,
    };
    let layer = scripted_map_modes::Layer {
        kind: LayerType::Country,
        color: content_reader_parser::parser::parse(
            br#"
            if = { limit = { FROM = { tag = SOV } }
                set_temp_variable = { red = owned_states^num }
            }
            OR = { any_owned_state = { always = yes } always = yes }
            set_temp_variable = { red = owned_states^num }
            set_temp_variable = { alpha = 1 }
        "#,
        )
        .unwrap()
        .items,
        ..Default::default()
    };
    let result =
        colour_script::borders("test", "top", &layer, &inputs, None, true, &mut Vec::new());
    assert_eq!(result.refusals.len(), 1, "{:?}", result.refusals);
    assert!(result.refusals[0].contains("country counter inputs, owned states"));
    assert!(
        result.refusals[0].contains("3 targets"),
        "{:?}",
        result.refusals
    );
    assert!(result.borders.values().all(|border| border.style.is_none()));
}

#[test]
fn scripted_callbacks_count_the_global_lexical_failure_once_across_country_scopes() {
    use dashboard_text::triggers::{ContentFacts, World};
    viewer_scene_base::test_game::enter();
    let content = content(MapModes::default());
    let map = map();
    let mut f = frame();
    f.tables[TableId::ResearchModifiers.index()] =
        TableView::Complete(TableRows::ResearchModifiers(
            [100, 200, 300]
                .into_iter()
                .map(|country| replay_data::rows::ResearchModifierRow {
                    country,
                    index: u32::MAX,
                    kind: replay_data::rows::ResearchModifierRow::SHARING_ROSTER,
                    raw: -1,
                    count: 0,
                    ..Default::default()
                })
                .collect(),
        ))
        .into();
    f.tables[TableId::LexicalTokens.index()] = TableView::Unavailable {
        reason: "synthetic dictionary acquisition failed".into(),
    }
    .into();
    let names = content_reader_script::trigger_keys::TriggerNames::default();
    let dlcs = BTreeSet::new();
    let aliases = Arc::default();
    let world = World::read(
        &f,
        with_empty_character_templates(ContentFacts::content_only(&dlcs, &names, &aliases)),
    );
    let inputs = Inputs {
        content: &content,
        map: &map,
        frame: &f,
        selected: Some(0),
        world: Some(&world),
        colour: None,
    };
    let layer = scripted_map_modes::Layer {
        kind: LayerType::Country,
        color: content_reader_parser::parser::parse(
            br#"
            OR = { is_in_tech_sharing_group = mod_group always = yes }
            FROM = { OR = { is_in_tech_sharing_group = mod_group always = yes } }
            set_temp_variable = { red = 1 } set_temp_variable = { alpha = 1 }
        "#,
        )
        .unwrap()
        .items,
        ..Default::default()
    };
    for _ in 0..2 {
        let refused =
            colour_script::borders("test", "top", &layer, &inputs, None, true, &mut Vec::new());
        assert_eq!(refused.refusals.len(), 1, "{:?}", refused.refusals);
        let why = &refused.refusals[0];
        assert!(
            why.contains("lexical tokens") && why.contains("3 targets"),
            "{why}"
        );
        assert!(
            !why.contains("research modifiers") && !why.contains("of country"),
            "{why}"
        );
        assert!(
            refused
                .borders
                .values()
                .all(|border| border.style.is_none())
        );
    }
}

#[test]
fn scripted_callbacks_refuse_uncertain_inputs_once_per_target() {
    use dashboard_text::triggers::{ContentFacts, World};
    viewer_scene_base::test_game::enter();
    let content = content(MapModes::default());
    let map = map();
    let mut f = frame_with_empty_country_variables();
    for table in [TableId::CountryStabilityInputs, TableId::ResearchModifiers] {
        f.tables[table.index()] = TableView::Unavailable {
            reason: "synthetic uncertified inputs".into(),
        }
        .into();
    }
    if let TableView::Complete(TableRows::CountryCounterInputs(rows)) =
        &mut *f.tables[TableId::CountryCounterInputs.index()]
    {
        for row in rows {
            row.owned_states = Some(vec![64, 65]).into();
        }
    }
    if let TableView::Complete(TableRows::States(rows)) = &mut *f.tables[TableId::States.index()] {
        rows.extend([64, 65].into_iter().map(|id| StateRow {
            id,
            owner: 100,
            ..Default::default()
        }));
    }
    let names = content_reader_script::trigger_keys::TriggerNames::default();
    let dlcs = BTreeSet::new();
    let aliases = Arc::default();
    let world = World::read(
        &f,
        with_empty_character_templates(ContentFacts::content_only(&dlcs, &names, &aliases)),
    );
    let inputs = Inputs {
        content: &content,
        map: &map,
        frame: &f,
        selected: Some(0),
        world: Some(&world),
        colour: None,
    };
    let render = |body: &str, previous: Option<&colour_script::LayerState>| {
        let layer = scripted_map_modes::Layer {
            kind: LayerType::Country,
            color: content_reader_parser::parser::parse(body.as_bytes())
                .unwrap()
                .items,
            ..Default::default()
        };
        colour_script::borders(
            "test",
            "top",
            &layer,
            &inputs,
            previous,
            true,
            &mut Vec::new(),
        )
    };
    let previous = render(
        "set_temp_variable = { alpha = 1 } set_temp_variable = { red = 1 }",
        None,
    );
    assert!(previous.refusals.is_empty());
    assert_eq!(previous.borders.len(), 3);
    assert!(
        previous
            .borders
            .values()
            .all(|border| border.style.is_some())
    );
    for (case, body, missing) in [
        (
            "known overwrite is conservatively refused",
            r#"
            if = { limit = { is_in_tech_sharing_group = mod_group }
                set_temp_variable = { scratch = 1 }
                add_to_temp_variable = { scratch = 1 }
                set_temp_variable = { red = 0 }
            }
            set_temp_variable = { red = 1 } set_temp_variable = { green = 0 }
            set_temp_variable = { blue = 0 } set_temp_variable = { alpha = 1 }
            set_temp_variable = { highlighted = 0 }
        "#,
            vec!["research modifiers"],
        ),
        (
            "optional division destination insertion without store",
            r#"
            set_temp_variable = { alpha = 1 }
            if = { limit = { has_stability > 0.5 } divide_temp_variable = { var = red?1 value = 0 } }
        "#,
            vec!["country stability inputs"],
        ),
        (
            "optional modulo destination insertion without store",
            r#"
            set_temp_variable = { alpha = 1 }
            if = { limit = { has_stability > 0.5 } modulo_temp_variable = { var = red?1 value = 0 } }
        "#,
            vec!["country stability inputs"],
        ),
        (
            "later iteration cannot erase an uncertain earlier stop",
            r#"
            set_temp_variable = { alpha = 1 }
            any_owned_state = {
                if = { limit = { state = 64 } ROOT = { has_stability > 0.5 } }
                if = { limit = { state = 65 } set_temp_variable = { red = 1 } }
            }
        "#,
            vec!["country stability inputs"],
        ),
        (
            "one input affects two outputs and several reads",
            r#"
            set_temp_variable = { alpha = 1 }
            if = { limit = { has_stability > 0.5 }
                set_temp_variable = { red = 1 } set_temp_variable = { blue = 1 }
            }
            if = { limit = { has_stability > 0.7 } set_temp_variable = { green = 1 } }
            set_temp_variable = { red = stability }
        "#,
            vec!["country stability inputs"],
        ),
        (
            "two independent inputs",
            r#"
            set_temp_variable = { alpha = 1 }
            if = { limit = { has_stability > 0.5 } set_temp_variable = { red = 1 } }
            if = { limit = { is_in_tech_sharing_group = mod_group } set_temp_variable = { blue = 1 } }
        "#,
            vec!["country stability inputs", "research modifiers"],
        ),
    ] {
        let refused = render(&format!("FROM = {{ {body} }}"), Some(&previous));
        assert_eq!(
            refused.refusals.len(),
            missing.len(),
            "{case}: {:?}",
            refused.refusals
        );
        for input in missing {
            let lines = refused
                .refusals
                .iter()
                .filter(|why| why.contains(input))
                .collect::<Vec<_>>();
            assert_eq!(lines.len(), 1, "{case}: {:?}", refused.refusals);
            assert!(
                lines[0].contains("3 targets"),
                "{case}: {:?}",
                refused.refusals
            );
        }
        assert!(
            refused
                .borders
                .values()
                .all(|border| border.style.is_none()),
            "{case}"
        );
    }
    let one_target = render(
        r#"
        if = { limit = { FROM = { tag = SOV } } has_stability > 0.5 }
        set_temp_variable = { alpha = 1 } set_temp_variable = { red = 1 }
    "#,
        None,
    );
    assert_eq!(one_target.refusals.len(), 1, "{:?}", one_target.refusals);
    assert!(one_target.refusals[0].contains("1 targets"));
    assert!(one_target.refusals[0].contains("country stability inputs"));
    assert_eq!(
        one_target
            .borders
            .values()
            .filter(|border| border.style.is_some())
            .count(),
        2
    );
}

#[test]
fn scripted_target_filters_refuse_uncertainty_before_colour_capture_on_cold_and_warm_reads() {
    use dashboard_text::triggers::{ContentFacts, World};
    viewer_scene_base::test_game::enter();
    let content = content(MapModes::default());
    let map = map();
    let mut f = frame();
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *f.tables[TableId::Countries.index()]
    {
        for row in rows {
            row.canonical_slot += 1;
        }
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *f.tables[TableId::CountryLinks.index()]
    {
        for row in rows {
            row.canonical_slot += 1;
        }
    }
    if let TableView::Complete(TableRows::States(rows)) = &mut *f.tables[TableId::States.index()] {
        for row in rows {
            row.controller = 100;
            row.single_controller = Some(1).into();
        }
    }
    let names = content_reader_script::trigger_keys::TriggerNames::default();
    let dlcs = BTreeSet::new();
    let aliases = Arc::default();
    for scenario in ["missing templates", "missing stability", "true", "false"] {
        let facts = ContentFacts::content_only(&dlcs, &names, &aliases);
        let world = World::read(
            &f,
            if scenario == "missing templates" {
                facts
            } else {
                with_empty_character_templates(facts)
            },
        );
        let inputs = Inputs {
            content: &content,
            map: &map,
            frame: &f,
            selected: Some(1),
            world: Some(&world),
            colour: None,
        };
        for kind in [
            LayerType::Country,
            LayerType::State,
            LayerType::StateController,
        ] {
            let baseline = scripted_map_modes::Layer {
                kind,
                color: content_reader_parser::parser::parse(
                    b"set_temp_variable = { green = 1 } set_temp_variable = { alpha = 1 }",
                )
                .unwrap()
                .items,
                ..Default::default()
            };
            let previous = colour_script::borders(
                "test",
                "top",
                &baseline,
                &inputs,
                None,
                true,
                &mut Vec::new(),
            );
            assert!(previous.refusals.is_empty(), "{:?}", previous.refusals);
            assert_eq!(previous.borders.len(), 3);
            assert!(
                previous
                    .borders
                    .values()
                    .all(|border| border.style.is_some())
            );
            for filter in ["target_root_trigger", "target_trigger"] {
                let candidate = if kind == LayerType::Country {
                    "country = GER"
                } else {
                    "state = 1"
                };
                let condition = match scenario {
                    "missing stability" => "OR = { has_stability > 0.5 always = yes }",
                    "false" => "ROOT = { always = no }",
                    _ => "ROOT = { always = yes } ROOT = { always = yes }",
                };
                let layer = scripted_map_modes::Layer {
                    targets: Some(
                        content_reader_parser::parser::parse(
                            format!("{candidate} {filter} = {{ {condition} }}").as_bytes(),
                        )
                        .unwrap()
                        .items,
                    ),
                    ..baseline.clone()
                };
                for _ in 0..2 {
                    let mut warnings = Vec::new();
                    let selected = colour_script::borders(
                        "test",
                        "top",
                        &layer,
                        &inputs,
                        Some(&previous),
                        true,
                        &mut warnings,
                    );
                    match scenario {
                        "missing templates" | "missing stability" => {
                            assert!(selected.refused, "{kind:?}/{filter}/{scenario}");
                            assert!(selected.borders.is_empty());
                            assert_eq!(warnings, selected.refusals);
                            assert_eq!(warnings.len(), 1, "{warnings:?}");
                            let input = if scenario == "missing templates" {
                                "character templates"
                            } else {
                                "country stability inputs"
                            };
                            assert!(warnings[0].contains(input), "{warnings:?}");
                            assert!(warnings[0].contains("1 layer"), "{warnings:?}");
                        }
                        "true" => {
                            assert!(!selected.refused);
                            assert!(warnings.is_empty(), "{warnings:?}");
                            assert_eq!(selected.borders.len(), 1, "{kind:?}/{filter}");
                            let border = selected.borders.values().next().unwrap();
                            assert_eq!(border.provinces, [1, 2]);
                            assert_eq!(border.style.unwrap().color, pack([0.0, 1.0, 0.0, 1.0]));
                        }
                        "false" => {
                            assert!(!selected.refused);
                            assert!(warnings.is_empty(), "{warnings:?}");
                            assert!(selected.borders.is_empty());
                        }
                        _ => unreachable!(),
                    }
                }
            }
        }
    }
}

#[test]
fn scripted_empty_targets_keep_country_state_and_controller_layers_unrestricted() {
    viewer_scene_base::test_game::enter();
    use dashboard_text::triggers::{ContentFacts, World};
    let content = content(MapModes::default());
    let map = map();
    let mut f = frame();
    if let TableView::Complete(TableRows::States(rows)) = &mut *f.tables[TableId::States.index()] {
        for row in rows {
            row.controller = match row.id {
                1 => 100,
                2 => 200,
                _ => 300,
            };
            row.single_controller = Some(1).into();
        }
    }
    let names = content_reader_script::trigger_keys::TriggerNames::default();
    let dlcs = BTreeSet::new();
    let aliases = Arc::default();
    let world = World::read(
        &f,
        with_empty_character_templates(ContentFacts::content_only(&dlcs, &names, &aliases)),
    );
    let inputs = Inputs {
        content: &content,
        map: &map,
        frame: &f,
        selected: Some(0),
        world: Some(&world),
        colour: None,
    };
    for kind in [
        LayerType::Country,
        LayerType::State,
        LayerType::StateController,
    ] {
        let mut layer = scripted_map_modes::Layer {
            kind,
            color: content_reader_parser::parser::parse(b"set_temp_variable = { alpha = 1 }")
                .unwrap()
                .items,
            ..Default::default()
        };
        let unrestricted =
            colour_script::borders("test", "top", &layer, &inputs, None, true, &mut Vec::new());
        assert!(
            unrestricted.refusals.is_empty(),
            "{:?}",
            unrestricted.refusals
        );
        assert_eq!(unrestricted.borders.len(), 3, "{kind:?}");
        layer.targets = Some(Vec::new());
        let empty =
            colour_script::borders("test", "top", &layer, &inputs, None, true, &mut Vec::new());
        assert_eq!(empty, unrestricted, "{kind:?}");
    }
}

#[test]
fn scripted_split_states_use_the_cached_single_branch_and_positive_province_groups() {
    viewer_scene_base::test_game::enter();
    use colour_script::{TargetId, borders};
    use dashboard_text::triggers::{ContentFacts, World};
    let mut modes = MapModes::default();
    scripted_map_modes::parse_into(&"test.txt".into(), br#"scripted_map_modes = {
        split = { top = { type = state_controller targets = { targets = { 1 } }
            color = { set_temp_variable = { alpha = 1 } FROM = { FROM = { tag = SOV } } set_temp_variable = { red = 1 } }
        } }
    }"#, &mut modes).unwrap();
    let content = content(modes);
    let map = map();
    let (names, dlcs, aliases) = (
        content_reader_script::trigger_keys::TriggerNames::default(),
        BTreeSet::new(),
        Arc::default(),
    );
    let mut f = frame();
    for (single, province_controller) in [
        (true, 200),
        (false, 200),
        (true, u32::MAX),
        (false, u32::MAX),
    ] {
        if let TableView::Complete(TableRows::Provinces(rows)) =
            &mut *f.tables[TableId::Provinces.index()]
        {
            rows[1].controller = province_controller;
        }
        if let TableView::Complete(TableRows::States(rows)) =
            &mut *f.tables[TableId::States.index()]
        {
            for row in rows {
                row.controller = 100;
                row.single_controller = (row.id == 1).then_some(u32::from(single)).into();
            }
        }
        let world = World::read(
            &f,
            with_empty_character_templates(ContentFacts::content_only(&dlcs, &names, &aliases)),
        );
        let inputs = Inputs {
            content: &content,
            map: &map,
            frame: &f,
            selected: Some(0),
            world: Some(&world),
            colour: None,
        };
        let mut warnings = Vec::new();
        let kept = borders(
            "split",
            "top",
            &content.scripted.get("split").unwrap().top,
            &inputs,
            None,
            true,
            &mut warnings,
        );
        assert!(warnings.is_empty(), "{warnings:?}");
        let sov = TargetId::Portion {
            state: 1,
            controller: 1,
        };
        if single {
            assert_eq!(kept.borders.len(), 1);
            assert_eq!(
                kept.borders[&TargetId::Portion {
                    state: 1,
                    controller: 0
                }]
                    .provinces,
                [1, 2]
            );
            assert!(kept.borders.values().all(|border| border.style.is_none()));
        } else if province_controller == 200 {
            assert_eq!(kept.borders.len(), 2);
            assert_eq!(kept.borders[&sov].provinces, [2]);
            assert!(kept.borders[&sov].style.is_some());
        } else {
            assert_eq!(kept.borders.len(), 1);
            assert_eq!(kept.borders.values().next().unwrap().provinces, [1]);
        }
    }
}

#[test]
fn scripted_forced_refresh_matches_the_recorded_declaration_between_daily_updates() {
    use replay_data::{
        rows::MapEvent, rows::MapEventFamily, rows::MapEventFamilyHealth, rows::MapEventPayload,
        rows::MapEventsRow,
    };
    let mut modes = MapModes::default();
    scripted_map_modes::parse_into(
        &"test.txt".into(),
        b"scripted_map_modes = { first = {} second = { update_daily = no } }",
        &mut modes,
    )
    .unwrap();
    let content = content(modes);
    let map = map();
    let mut f = frame();
    *f.tables[TableId::MapEvents.index()] =
        TableView::Complete(TableRows::MapEvents(vec![MapEventsRow {
            from_hour: f.hour,
            to_hour: f.hour,
            first_sequence: 1,
            next_sequence: 2,
            health: [MapEventFamilyHealth {
                complete: true,
                enabled: true,
                installed: true,
                ..Default::default()
            }; MapEventFamily::COUNT],
            events: vec![MapEvent {
                sequence: 1,
                hour: f.hour,
                payload: MapEventPayload::ScriptedMapRefresh(1),
            }],
            ..Default::default()
        }]));
    let refresh = |name: &str, frame: &DecodedFrame| {
        colour_script::forced_refresh(
            name,
            &Inputs {
                content: &content,
                map: &map,
                frame,
                selected: Some(0),
                world: None,
                colour: None,
            },
        )
    };
    assert_eq!(refresh("second", &f), Ok(true));
    assert_eq!(refresh("first", &f), Ok(false));
    if let TableView::Complete(TableRows::MapEvents(rows)) =
        &mut *f.tables[TableId::MapEvents.index()]
    {
        rows[0].to_hour += 1;
    }
    assert!(refresh("second", &f).unwrap_err().contains("not this hour"));
    if let TableView::Complete(TableRows::MapEvents(rows)) =
        &mut *f.tables[TableId::MapEvents.index()]
    {
        rows[0].to_hour -= 1;
        rows[0].events[0].payload = MapEventPayload::ScriptedMapRefresh(2);
    }
    assert!(
        refresh("second", &f)
            .unwrap_err()
            .contains("outside the recorded content")
    );
    if let TableView::Complete(TableRows::MapEvents(rows)) =
        &mut *f.tables[TableId::MapEvents.index()]
    {
        rows[0].health[MapEventFamily::ScriptedMapRefresh.index()].complete = false;
    }
    assert!(refresh("second", &f).unwrap_err().contains("incomplete"));
}

#[test]
fn scripted_forced_refresh_refuses_inherited_temps_without_guessing_zero() {
    use dashboard_text::triggers::{ContentFacts, World};
    use replay_data::{
        rows::MapEvent, rows::MapEventFamily, rows::MapEventFamilyHealth, rows::MapEventPayload,
        rows::MapEventsRow,
    };
    viewer_scene_base::test_game::enter();
    let mut modes = MapModes::default();
    scripted_map_modes::parse_into(&"test.txt".into(), br#"scripted_map_modes = {
        partial = { update_daily = no top = { type = country color = {
            set_temp_variable = { alpha = 1 }
            FROM = { if = { limit = { tag = SOV } set_temp_variable = { blue = 1 } }
                else = { set_temp_variable = { green = 1 } } }
        } } }
        complete = { top = { type = country color = {
            set_temp_variable = { red = 0.25 } set_temp_variable = { green = 0.5 }
            set_temp_variable = { blue = 0.75 } set_temp_variable = { alpha = 1 }
            set_temp_variable = { highlighted = 0 }
        } } }
        filter = { top = { type = country
            targets = { target_root_trigger = { OR = { check_variable = { var = inherited value = 0 compare = equals } always = yes } } targets = { SOV } }
            color = { set_temp_variable = { red = 0 } set_temp_variable = { green = 1 }
                set_temp_variable = { blue = 0 } set_temp_variable = { alpha = 1 }
                set_temp_variable = { highlighted = 0 } }
        } }
        transparent = { top = { type = country color = { set_temp_variable = { alpha = 0 } } } }
    }"#, &mut modes).unwrap();
    let content = content(modes);
    let (mut frame, map) = (frame(), map());
    let names = content_reader_script::trigger_keys::TriggerNames {
        scripted: Default::default(),
        counts: Some(Default::default()),
    };
    let (dlcs, aliases) = (BTreeSet::new(), Arc::default());
    let run = |name: &str, frame: &DecodedFrame, previous: Option<&ColourState>| {
        let world = World::read(
            frame,
            with_empty_character_templates(ContentFacts::content_only(&dlcs, &names, &aliases)),
        );
        checked_fill(
            &MapMode::Scripted(name.into()),
            &Inputs {
                content: &content,
                map: &map,
                frame,
                selected: Some(0),
                world: Some(&world),
                colour: previous,
            },
        )
        .unwrap()
    };
    let standalone = run("partial", &frame, None);
    assert!(standalone.warnings.is_empty(), "{:?}", standalone.warnings);
    assert_eq!(
        standalone
            .draw
            .unwrap()
            .layer
            .unwrap()
            .styles
            .values()
            .map(|s| s.color)
            .collect::<Vec<_>>(),
        [
            pack([0.0, 1.0, 0.0, 1.0]),
            pack([0.0, 0.0, 1.0, 1.0]),
            pack([0.0, 1.0, 0.0, 1.0])
        ]
    );
    let prior = standalone.colour.unwrap();
    for (ordinal, name) in ["partial", "complete", "filter", "transparent"]
        .into_iter()
        .enumerate()
    {
        *frame.tables[TableId::MapEvents.index()] =
            TableView::Complete(TableRows::MapEvents(vec![MapEventsRow {
                from_hour: frame.hour,
                to_hour: frame.hour,
                first_sequence: 1,
                next_sequence: 2,
                health: [MapEventFamilyHealth {
                    complete: true,
                    enabled: true,
                    installed: true,
                    ..Default::default()
                }; MapEventFamily::COUNT],
                events: vec![MapEvent {
                    sequence: 1,
                    hour: frame.hour,
                    payload: MapEventPayload::ScriptedMapRefresh(ordinal as u32),
                }],
                ..Default::default()
            }]));
        for previous in [None, Some(prior.as_ref())] {
            let actual = run(name, &frame, previous);
            if name == "transparent" {
                assert!(actual.warnings.is_empty(), "{:?}", actual.warnings);
                assert!(actual.draw.unwrap().layer.unwrap().styles.is_empty());
            } else if name == "complete" {
                assert!(actual.warnings.is_empty(), "{:?}", actual.warnings);
                assert_eq!(
                    actual
                        .draw
                        .unwrap()
                        .layer
                        .unwrap()
                        .styles
                        .values()
                        .map(|s| s.color)
                        .collect::<Vec<_>>(),
                    [pack([0.25, 0.5, 0.75, 1.0]); 3]
                );
            } else {
                assert_eq!(actual.warnings.len(), 1, "{:?}", actual.warnings);
                assert!(
                    actual.warnings[0].contains("caller temporary variables"),
                    "{:?}",
                    actual.warnings
                );
                assert!(
                    actual.warnings[0].contains(if name == "partial" {
                        "3 targets"
                    } else {
                        "1 layer"
                    }),
                    "{:?}",
                    actual.warnings
                );
                assert!(
                    actual.colour.unwrap().layers[1]
                        .borders
                        .values()
                        .all(|b| b.style.is_none())
                );
            }
        }
    }
}

#[test]
fn scripted_scheduled_refresh_preserves_unknown_event_context() {
    use dashboard_text::triggers::{ContentFacts, World};
    use replay_data::{rows::MapEventFamily, rows::MapEventFamilyHealth, rows::MapEventsRow};
    viewer_scene_base::test_game::enter();
    let mut modes = MapModes::default();
    scripted_map_modes::parse_into(
        &"test.txt".into(),
        br#"scripted_map_modes = {
        partial = { update_daily = yes top = { type = country color = {
            set_temp_variable = { alpha = 1 } set_temp_variable = { green = 1 }
        } } }
        complete = { update_daily = yes top = { type = country color = {
            set_temp_variable = { red = 0.25 } set_temp_variable = { green = 0.5 }
            set_temp_variable = { blue = 0.75 } set_temp_variable = { alpha = 1 }
            set_temp_variable = { highlighted = 0 }
        } } }
    }"#,
        &mut modes,
    )
    .unwrap();
    let content = content(modes);
    let (frame, map) = (frame(), map());
    let names = content_reader_script::trigger_keys::TriggerNames {
        scripted: Default::default(),
        counts: Some(Default::default()),
    };
    let (dlcs, aliases) = (BTreeSet::new(), Arc::default());
    let run = |name: &str, frame: &DecodedFrame, previous: Option<&ColourState>| {
        let world = World::read(
            frame,
            with_empty_character_templates(ContentFacts::content_only(&dlcs, &names, &aliases)),
        );
        checked_fill(
            &MapMode::Scripted(name.into()),
            &Inputs {
                content: &content,
                map: &map,
                frame,
                selected: Some(0),
                world: Some(&world),
                colour: previous,
            },
        )
        .unwrap()
    };
    for name in ["partial", "complete"] {
        let update_daily = content.scripted.get(name).unwrap().update_daily;
        assert!(update_daily, "{name} must repaint daily");
        let prior = run(name, &frame, None).colour.unwrap();
        for (hour, previous, scheduled) in [
            (43, None, true),
            (48, Some(prior.as_ref()), true),
            (43, Some(prior.as_ref()), false),
        ] {
            assert_eq!(
                colour_script::refreshes(previous, update_daily, hour),
                scheduled,
                "{name} at {hour} must exercise the intended refresh path"
            );
            let mut current = frame.clone();
            current.hour = hour;
            let clean = MapEventsRow {
                from_hour: hour,
                to_hour: hour,
                first_sequence: 1,
                next_sequence: 1,
                health: [MapEventFamilyHealth {
                    complete: true,
                    enabled: true,
                    installed: true,
                    ..Default::default()
                }; MapEventFamily::COUNT],
                ..Default::default()
            };
            let mut incomplete = clean.clone();
            incomplete.health[MapEventFamily::ScriptedMapRefresh.index()].complete = false;
            let mut malformed = clean.clone();
            malformed.from_hour += 1;
            *current.tables[TableId::MapEvents.index()] =
                TableView::Complete(TableRows::MapEvents(vec![clean.clone()]));
            let control = run(name, &current, previous);
            assert!(control.warnings.is_empty(), "{:?}", control.warnings);
            let expected = if name == "complete" {
                [pack([0.25, 0.5, 0.75, 1.0]); 3]
            } else {
                [pack([0.0, 1.0, 0.0, 1.0]); 3]
            };
            let colors = |actual: &ModeFill| {
                actual
                    .draw
                    .as_ref()
                    .unwrap()
                    .layer
                    .as_ref()
                    .unwrap()
                    .styles
                    .values()
                    .map(|style| style.color)
                    .collect::<Vec<_>>()
            };
            assert_eq!(colors(&control), expected, "{name} at {hour}");
            for (why, evidence) in [
                (
                    "events unavailable",
                    TableView::Unavailable {
                        reason: "missing refresh evidence".into(),
                    },
                ),
                (
                    "events unavailable",
                    TableView::Truncated {
                        rows: TableRows::MapEvents(vec![clean]),
                        missing: 1,
                    },
                ),
                (
                    "interval is absent",
                    TableView::Complete(TableRows::MapEvents(Vec::new())),
                ),
                (
                    "interval is incomplete",
                    TableView::Complete(TableRows::MapEvents(vec![incomplete])),
                ),
                (
                    "interval is malformed",
                    TableView::Complete(TableRows::MapEvents(vec![malformed])),
                ),
            ] {
                *current.tables[TableId::MapEvents.index()] = evidence;
                let actual = run(name, &current, previous);
                let kept = &actual.colour.as_ref().unwrap().layers[1];
                if !scheduled {
                    assert_eq!(actual.warnings.len(), 1, "{:?}", actual.warnings);
                    assert!(actual.warnings[0].contains(why), "{:?}", actual.warnings);
                    assert!(kept.refused && kept.borders.is_empty());
                } else if name == "complete" {
                    assert!(actual.warnings.is_empty(), "{:?}", actual.warnings);
                    assert_eq!(colors(&actual), expected, "{name} at {hour}: {why}");
                } else {
                    assert_eq!(actual.warnings.len(), 1, "{:?}", actual.warnings);
                    assert!(
                        actual.warnings[0].contains("caller temporary variables"),
                        "{:?}",
                        actual.warnings
                    );
                    assert!(
                        actual.warnings[0].contains("3 targets"),
                        "{:?}",
                        actual.warnings
                    );
                    assert_eq!(kept.borders.len(), 3);
                    assert!(kept.borders.values().all(|border| border.style.is_none()));
                    assert!(!kept.refused);
                }
            }
        }
    }
}

#[test]
fn colour_borders_keep_their_colour_until_their_provinces_change() {
    viewer_scene_base::test_game::enter();
    use colour_script::{Border, LayerState, TargetId, borders, refreshes};
    use dashboard_text::triggers::{ContentFacts, World};
    let mut modes = MapModes::default();
    scripted_map_modes::parse_into(
        &"test.txt".into(),
        br#"scripted_map_modes = { held = { top = { type = country color = { always = no } } } }"#,
        &mut modes,
    )
    .unwrap();
    let layer = modes.get("held").unwrap().top.clone();
    let content = content(modes);
    let (frame, map) = (frame(), map());
    let names = content_reader_script::trigger_keys::TriggerNames {
        scripted: Default::default(),
        counts: Some(Default::default()),
    };
    let (dlcs, aliases) = (BTreeSet::new(), Arc::default());
    let world = World::read(
        &frame,
        with_empty_character_templates(ContentFacts::content_only(&dlcs, &names, &aliases)),
    );
    let inputs = Inputs {
        content: &content,
        map: &map,
        frame: &frame,
        selected: Some(0),
        world: Some(&world),
        colour: None,
    };
    let red = Some(Style {
        color: pack([1.0, 0.0, 0.0, 1.0]),
        pulse: false,
        fill: 1.0,
    });
    let previous = LayerState {
        borders: BTreeMap::from([
            (
                TargetId::Country(1),
                Border {
                    provinces: vec![3],
                    style: red,
                },
            ),
            (
                TargetId::Country(0),
                Border {
                    provinces: vec![1],
                    style: red,
                },
            ),
            (
                TargetId::Country(7),
                Border {
                    provinces: vec![4],
                    style: red,
                },
            ),
        ]),
        refusals: vec!["kept refusal".to_owned()],
        refused: false,
    };
    let mut warnings = Vec::new();
    let kept = borders(
        "held",
        "top",
        &layer,
        &inputs,
        Some(&previous),
        true,
        &mut warnings,
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    let kept = kept.borders;
    assert_eq!(kept[&TargetId::Country(1)].style, red);
    assert_eq!(kept[&TargetId::Country(0)].style, None);
    assert_eq!(kept[&TargetId::Country(0)].provinces, [1, 2]);
    assert_eq!(kept[&TargetId::Country(2)].style, None);
    assert!(!kept.contains_key(&TargetId::Country(7)));
    let held = borders(
        "held",
        "top",
        &layer,
        &inputs,
        Some(&previous),
        false,
        &mut warnings,
    );
    assert_eq!(held, previous);
    assert_eq!(warnings, ["kept refusal"]);
    let state = ColourState {
        mode: "held".into(),
        layers: Default::default(),
    };
    assert!(refreshes(None, false, 43));
    assert!(refreshes(Some(&state), true, 48));
    assert!(!refreshes(Some(&state), true, 43));
    assert!(!refreshes(Some(&state), false, 48));
}

const ROADS: u32 = 77;

fn infrastructure_frame(values: &[Option<(i32, i32)>]) -> DecodedFrame {
    let mut frame = frame();
    frame.tables[TableId::States.index()] = TableView::Complete(TableRows::States(
        values
            .iter()
            .enumerate()
            .map(|(index, value)| StateRow {
                id: index as u32 + 1,
                infrastructure: Recorded::Value(value.map(|(level, maximum)| {
                    replay_data::rows::StateInfrastructure {
                        building_type: ROADS,
                        level,
                        maximum,
                    }
                })),
                ..Default::default()
            })
            .collect(),
    ))
    .into();
    frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: ROADS,
            literal: b"Roads".to_vec(),
        }]))
        .into();
    frame
}

fn infrastructure_fill(infrastructure: infrastructure::Content, frame: &DecodedFrame) -> ModeFill {
    let content = Content {
        infrastructure,
        ..content(MapModes::default())
    };
    checked_fill(
        &MapMode::Stock(Stock::Infrastructure),
        &Inputs {
            content: &content,
            map: &map(),
            frame,
            selected: Some(0),
            world: None,
            colour: None,
        },
    )
    .unwrap()
}

#[test]
fn infrastructure_uses_hourly_limits_without_daily_building_or_research_tables() {
    let roads = infrastructure::for_tests("roads", true);
    let mut frame = infrastructure_frame(&[Some((2, 5)), Some((7, 5)), Some((0, 5))]);
    for table in [
        TableId::StateBuildings,
        TableId::ResearchTechnologies,
        TableId::TechnologyDefinitions,
    ] {
        frame.tables[table.index()] = TableView::Unavailable {
            reason: "independent table unavailable".into(),
        }
        .into();
    }
    let fill = infrastructure_fill(roads.clone(), &frame);
    assert!(fill.warnings.is_empty(), "{:?}", fill.warnings);
    let layer = fill.draw.unwrap().layer.unwrap();
    assert_eq!(layer.styles[&2].color, pack([0.64, 0.435, 0.04, 0.3]));
    assert_eq!(layer.styles[&3].color, pack([0.1, 0.9, 0.1, 0.6]));
    assert_eq!(layer.styles[&4].color, pack([1.0, 0.125, 0.0, 0.1]));
    assert_eq!(layer.gradient, Some(LEVEL_GRADIENT));
    assert_eq!(layer.thickness, Some(5.0));
    assert_eq!(roads.hover_level(&frame, 1), Ok(2));
    assert_eq!(roads.hover_level(&frame, 3), Ok(7));
    assert_eq!(roads.hover_level(&frame, 4), Ok(0));
    assert_eq!(roads.hover_level(&frame, 5), Ok(0));
}

#[test]
fn missing_infrastructure_limits_withhold_only_their_states() {
    let roads = infrastructure::for_tests("roads", true);
    let frame = infrastructure_frame(&[Some((1, 2)), None, Some((1, 0))]);
    let fill = infrastructure_fill(roads.clone(), &frame);
    let layer = fill.draw.unwrap().layer.unwrap();
    assert!(layer.styles.contains_key(&2));
    assert!(!layer.styles.contains_key(&3));
    assert_eq!(layer.region_of[3], 0);
    assert_eq!(layer.styles[&4].color, pack(infrastructure::NO_MAXIMUM));
    assert_eq!(fill.warnings.len(), 1);
    assert!(roads.hover_level(&frame, 3).is_err());
    let layer = infrastructure_fill(infrastructure::for_tests("roads", false), &frame)
        .draw
        .unwrap()
        .layer
        .unwrap();
    for key in 2..=4 {
        assert_eq!(layer.styles[&key].color, pack(infrastructure::NO_MAXIMUM));
    }
}

#[test]
fn infrastructure_identity_must_join_the_recorded_dictionary() {
    let roads = infrastructure::for_tests("other", true);
    let frame = infrastructure_frame(&[Some((4, 5)), Some((0, i32::MAX)), Some((0, 0))]);
    let fill = infrastructure_fill(roads.clone(), &frame);
    assert!(fill.draw.unwrap().layer.unwrap().styles.is_empty());
    assert!(roads.hover_level(&frame, 1).is_err());
    let missing = infrastructure_fill(infrastructure::Content::default(), &frame);
    assert_eq!(
        missing.draw.unwrap().channels,
        [Source::Empty, Source::Country]
    );
    assert!(missing.warnings[0].contains("infrastructure = yes"));
}

#[test]
fn scripted_invasion_failures_withhold_colours_on_cold_and_warm_caches() {
    use dashboard_text::triggers::{ContentFacts, World};
    viewer_scene_base::test_game::enter();
    let content = content(MapModes::default());
    let map = map();
    for (known_state, unresolved, missing_province) in [
        (None, true, false),
        (Some(2), true, false),
        (Some(1), true, false),
        (Some(1), false, false),
        (Some(1), false, true),
    ] {
        let mut f = frame();
        f.tables[TableId::LandTheatres.index()] =
            TableView::Complete(TableRows::LandTheatres(vec![
                replay_data::rows::LandTheatreRow {
                    id: 1,
                    owner: 999,
                    ..Default::default()
                },
            ]))
            .into();
        f.tables[TableId::OrdersGroups.index()] =
            TableView::Complete(TableRows::OrdersGroups(vec![])).into();
        if let Some(state) = known_state {
            use replay_data::rows::*;
            macro_rules! set {
                ($table:ident, $rows:expr) => {
                    f.tables[TableId::$table.index()] =
                        TableView::Complete(TableRows::from_rows::<tables::$table>($rows)).into();
                };
            }
            set!(
                LandTheatres,
                vec![
                    LandTheatreRow {
                        id: 1,
                        owner: 999,
                        ..Default::default()
                    },
                    LandTheatreRow {
                        id: 2,
                        owner: 200,
                        ..Default::default()
                    },
                ]
            );
            set!(
                OrdersGroups,
                vec![OrdersGroupRow {
                    handle: 10,
                    theatre: 2,
                    theatre_ordinal: Some(0),
                    ..Default::default()
                }]
            );
            set!(
                OrdersGroupPlans,
                vec![OrdersGroupPlanRow {
                    group: 10,
                    primary_orders: vec![1],
                    ..Default::default()
                }]
            );
            set!(
                ArmyOrders,
                vec![ArmyOrderRow {
                    owner_group: 10,
                    instance: 1,
                    order_type: 3,
                    path: vec![if state == 1 { 1 } else { 3 }],
                    ..Default::default()
                }]
            );
            set!(
                ArmyOrderPlans,
                vec![ArmyOrderPlanRow {
                    owner_group: 10,
                    instance: 1,
                    ..Default::default()
                }]
            );
            set!(
                InvasionPreparations,
                vec![InvasionPreparationRow {
                    owner_group: 10,
                    instance: 1,
                    ..Default::default()
                }]
            );
        }
        if !unresolved {
            let TableView::Complete(TableRows::LandTheatres(rows)) =
                &mut *f.tables[TableId::LandTheatres.index()]
            else {
                panic!("theatres");
            };
            rows.retain(|row| row.owner != 999);
        }
        if missing_province {
            let TableView::Complete(TableRows::Provinces(rows)) =
                &mut *f.tables[TableId::Provinces.index()]
            else {
                panic!("complete provinces");
            };
            rows.retain(|row| row.id != 1);
        }
        let names = content_reader_script::trigger_keys::TriggerNames::default();
        let dlcs = BTreeSet::new();
        let aliases = Arc::default();
        let mut facts =
            with_empty_character_templates(ContentFacts::content_only(&dlcs, &names, &aliases));
        facts.invasion_prepare_days = Ok(1);
        let world = World::read(&f, facts);
        let inputs = Inputs {
            content: &content,
            map: &map,
            frame: &f,
            selected: Some(0),
            world: Some(&world),
            colour: None,
        };
        let render = |body: &str, previous: Option<&colour_script::LayerState>| {
            let layer = scripted_map_modes::Layer {
                kind: LayerType::Country,
                color: content_reader_parser::parser::parse(body.as_bytes())
                    .unwrap()
                    .items,
                ..Default::default()
            };
            colour_script::borders(
                "test",
                "top",
                &layer,
                &inputs,
                previous,
                true,
                &mut Vec::new(),
            )
        };
        let known = render(
            "set_temp_variable = { red = 1 } set_temp_variable = { alpha = 1 }",
            None,
        );
        assert!(known.refusals.is_empty());
        assert!(known.borders.values().all(|border| border.style.is_some()));
        for _ in 0..2 {
            let refused = render(
                "SOV = { OR = { AND = { has_naval_invasion_against_state = 1 set_temp_variable = { red = 1 } } always = yes } } set_temp_variable = { alpha = 1 }",
                Some(&known),
            );
            if !unresolved && !missing_province {
                assert!(refused.refusals.is_empty(), "{:?}", refused.refusals);
                for (key, border) in &refused.borders {
                    assert_eq!(border.style, known.borders[key].style);
                }
                continue;
            }
            assert_eq!(refused.refusals.len(), 1, "{:?}", refused.refusals);
            assert!(
                refused.refusals[0].contains(if missing_province {
                    "provinces"
                } else {
                    "country links"
                }) && refused.refusals[0].contains("3 targets"),
                "{:?}",
                refused.refusals
            );
            assert!(
                refused
                    .borders
                    .values()
                    .all(|border| border.style.is_none())
            );
        }
    }
}

#[test]
fn a_map_recording_offers_only_the_modes_whose_tables_it_records() {
    use replay_data::scope::CaptureMode;
    for stock in Stock::ALL {
        assert_eq!(
            stock.unavailable_in(CaptureMode::Full).as_deref(),
            stock.unavailable(),
            "{stock:?}"
        );
    }
    let offered: BTreeSet<_> = Stock::ALL
        .into_iter()
        .filter(|stock| stock.unavailable_in(CaptureMode::Map).is_none())
        .collect();
    let expected = [
        Stock::Default,
        Stock::States,
        Stock::Manpower,
        Stock::Terrain,
        Stock::Players,
        Stock::Ideology,
        Stock::Infrastructure,
        Stock::Factions,
        Stock::StrategicAir,
        Stock::StrategicNavy,
    ];
    assert_eq!(offered, expected.into_iter().collect());
    let refusal = Stock::Diplomacy.unavailable_in(CaptureMode::Map).unwrap();
    assert!(
        refusal.starts_with("Not in this recording: a map recording leaves out"),
        "{refusal}"
    );
    let scripted = MapMode::Scripted("synthetic".into());
    assert!(scripted.unavailable_in(CaptureMode::Full).is_none());
    assert!(scripted.unavailable_in(CaptureMode::Map).is_some());
}

#[test]
fn every_mode_a_map_recording_offers_fills_from_recorded_tables() {
    use viewer_foundation::test_support::{
        assert_reads_recorded,
        complete_frame,
        map_scoped,
        put_sample_campaign,
    };
    let mut frame = complete_frame(11, |table| {
        replay_data::rows::sample_extended_table(table, 0)
    });
    put_sample_campaign(&mut frame);
    let frame = map_scoped(frame);
    let content = content(MapModes::default());
    let map = map();
    for stock in Stock::ALL
        .into_iter()
        .filter(|stock| stock.unavailable_in(CaptureMode::Map).is_none())
    {
        let filled = assert_reads_recorded(&frame, &[], || {
            checked_fill(
                &MapMode::Stock(stock),
                &Inputs {
                    content: &content,
                    map: &map,
                    frame: &frame,
                    selected: Some(1),
                    world: None,
                    colour: None,
                },
            )
        });
        if let Err(error) = filled {
            assert!(
                !error.to_string().contains("not recorded"),
                "{stock:?}: {error}"
            );
        }
    }
}

fn checked_fill(mode: &MapMode, inputs: &Inputs) -> io::Result<ModeFill> {
    let actual = super::fill(mode, inputs);
    let expected = match mode {
        MapMode::Stock(stock) => super::stock_oracle::stock_fill(*stock, inputs),
        MapMode::Scripted(name) => super::scripted_oracle::fill(name, inputs),
    };
    match (&actual, &expected) {
        (Ok(actual), Ok(expected)) => assert_eq!(
            product_codec::encode(actual).unwrap(),
            product_codec::encode(expected).unwrap(),
            "{}",
            mode.token()
        ),
        (Err(actual), Err(expected)) => assert_eq!(
            (actual.kind(), actual.to_string()),
            (expected.kind(), expected.to_string())
        ),
        _ => panic!(
            "{}: candidate {actual:?}; frozen reference {expected:?}",
            mode.token()
        ),
    }
    actual
}

#[test]
fn every_stock_mode_and_perspective_keeps_its_independent_reference() {
    let content = content(MapModes::default());
    let map = map();
    let frame = frame();
    for stock in Stock::ALL {
        for selected in [None, Some(0), Some(1), Some(2), Some(1501)] {
            let inputs = Inputs {
                content: &content,
                map: &map,
                frame: &frame,
                selected,
                world: None,
                colour: None,
            };
            let _ = checked_fill(&MapMode::Stock(stock), &inputs);
        }
    }
}

#[test]
fn persistent_membership_rebinds_immutable_map_banks_and_style_membership() {
    let content = content(MapModes::default());
    let frame = frame();
    let mut map = Arc::new(map());
    content.bind_membership_map(&map);
    let inputs = Inputs {
        content: &content,
        map: &map,
        frame: &frame,
        selected: Some(0),
        world: None,
        colour: None,
    };
    let first = membership::plane(
        &inputs,
        0,
        &map_display_data::value_index::ValueIndex::try_from_iter([(1, style([1.0; 4]))]).unwrap(),
    )
    .unwrap();
    let same = membership::plane(
        &inputs,
        0,
        &map_display_data::value_index::ValueIndex::try_from_iter([(1, style([0.0; 4]))]).unwrap(),
    )
    .unwrap();
    assert!(Arc::ptr_eq(&first.0, &same.0));
    assert_eq!(first[1], 1);
    assert_eq!(first[3], 0);
    let replacement = Arc::make_mut(&mut map);
    replacement.definitions.get_mut(&1).unwrap().kind = ProvinceKind::Sea;
    content.bind_membership_map(&map);
    let inputs = Inputs {
        content: &content,
        map: &map,
        frame: &frame,
        selected: Some(0),
        world: None,
        colour: None,
    };
    let second = membership::plane(
        &inputs,
        0,
        &map_display_data::value_index::ValueIndex::try_from_iter([(1, style([1.0; 4]))]).unwrap(),
    )
    .unwrap();
    assert_eq!(second[1], 0);
    assert_eq!(second[2], 1);
    assert_eq!(first[1], 1);
}

#[test]
fn state_membership_does_not_validate_unread_controller_links() {
    let content = content(MapModes::default());
    let map = Arc::new(map());
    content.bind_membership_map(&map);
    let mut frame = frame();
    frame.tables[TableId::CountryLinks.index()] = replay_frame::frame::TableView::Unavailable {
        reason: "unused country links".into(),
    }
    .into();
    let inputs = Inputs {
        content: &content,
        map: &map,
        frame: &frame,
        selected: Some(0),
        world: None,
        colour: None,
    };
    let _ = checked_fill(&MapMode::Stock(Stock::States), &inputs);
    let _ = checked_fill(&MapMode::Stock(Stock::Manpower), &inputs);
    let styles =
        map_display_data::value_index::ValueIndex::try_from_iter([(8, style([1.0; 4]))]).unwrap();
    let state = membership::plane(&inputs, 1, &styles).unwrap();
    assert_eq!(
        state.len(),
        region_table_len(
            &map,
            core::<tables::Provinces>(&frame)
                .unwrap()
                .iter()
                .map(|row| row.id)
        )
    );
    assert!(membership::plane(&inputs, 0, &styles).is_err());
}

#[test]
fn country_membership_keeps_distinct_slot_root_on_quiet_reads() {
    let content = content(MapModes::default());
    let map = Arc::new(map());
    content.bind_membership_map(&map);
    let frame = frame();
    let inputs = Inputs {
        content: &content,
        map: &map,
        frame: &frame,
        selected: Some(0),
        world: None,
        colour: None,
    };
    let (plane, slots) = membership::countries(&inputs).unwrap();
    let (again, same_slots) = membership::countries(&inputs).unwrap();
    assert!(slots.shares(&same_slots));
    assert!(Arc::ptr_eq(&plane.0, &again.0));
    let (reference, expected_slots) =
        controller_regions(&map, &frame, &Countries::read(&frame).unwrap()).unwrap();
    assert_eq!(plane, reference);
    assert_eq!(
        (&slots).into_iter().copied().collect::<BTreeSet<_>>(),
        expected_slots
    );
}

#[test]
fn sparse_stock_folds_keep_quiet_roots_and_match_the_oracle_after_remaps() {
    let content = content(MapModes::default());
    let map = Arc::new(map());
    content.bind_membership_map(&map);
    let mut frame = frame();
    for step in 0..8 {
        if step == 2 {
            let TableView::Complete(TableRows::CountryAppearance(rows)) =
                &mut *frame.tables[TableId::CountryAppearance.index()]
            else {
                unreachable!()
            };
            rows[1].red = (-0.0f32).to_bits();
            rows[1].green = f32::from_bits(0x7fc01234).to_bits();
        }
        if step == 3 {
            let TableView::Complete(TableRows::CountryLinks(rows)) =
                &mut *frame.tables[TableId::CountryLinks.index()]
            else {
                unreachable!()
            };
            rows[1].canonical_slot = 2;
        }
        if step == 4 {
            let TableView::Complete(TableRows::Provinces(rows)) =
                &mut *frame.tables[TableId::Provinces.index()]
            else {
                unreachable!()
            };
            rows[0].controller = 300;
        }
        if step == 5 {
            let TableView::Complete(TableRows::LexicalTokens(rows)) =
                &mut *frame.tables[TableId::LexicalTokens.index()]
            else {
                unreachable!()
            };
            if let Some(row) = rows.first_mut() {
                row.literal = b"rural".to_vec();
            }
        }
        if step == 6 {
            let TableView::Complete(TableRows::States(rows)) =
                &mut *frame.tables[TableId::States.index()]
            else {
                unreachable!()
            };
            rows[0].population = Recorded::NotRecorded;
        }
        for selected in [Some(0), Some(1), Some(1501), Some(2)] {
            for stock in [
                Stock::Factions,
                Stock::Players,
                Stock::Ideology,
                Stock::States,
                Stock::Manpower,
            ] {
                let inputs = Inputs {
                    content: &content,
                    map: &map,
                    frame: &frame,
                    selected,
                    world: None,
                    colour: None,
                };
                let result = checked_fill(&MapMode::Stock(stock), &inputs);
                if let Ok(first) = result {
                    let second = checked_fill(&MapMode::Stock(stock), &inputs).unwrap();
                    if let (Some(first), Some(second)) = (first.draw, second.draw) {
                        for (a, b) in [(first.country, second.country), (first.layer, second.layer)]
                        {
                            if let (Some(a), Some(b)) = (a, b) {
                                assert!(a.styles.ptr_eq(&b.styles));
                                assert!(Arc::ptr_eq(&a.region_of.0, &b.region_of.0));
                            }
                        }
                    }
                }
            }
        }
        frame.hour += 1;
    }
}

#[test]
fn selected_state_folds_match_independent_network_and_occupation_outputs_after_deltas() {
    use replay_data::rows::StateResistanceRow;
    use replay_frame::frame::{FrameDecoder, FrameEncoder};
    let content = content(MapModes::default());
    let map = Arc::new(map());
    content.bind_membership_map(&map);
    let mut input = frame();
    input.tables[TableId::Campaign.index()] = viewer_scene_base::test_support::test_campaign();
    let TableView::Complete(TableRows::States(states)) =
        &mut *input.tables[TableId::States.index()]
    else {
        unreachable!()
    };
    for (state, controller) in states.iter_mut().zip([100, 200, 300]) {
        state.controller = controller;
    }
    let network = |owner, target, state_id, strength, quiet| IntelNetworkRow {
        owner_country: owner,
        target_country: target,
        state_id,
        local_strength_raw: strength,
        local_quiet: quiet,
        ..Default::default()
    };
    input.tables[TableId::IntelNetworks.index()] =
        TableView::Complete(TableRows::IntelNetworks(vec![
            network(100, 200, 2, 7_250_000, 0),
            network(100, 300, 3, 3_000_000, 1),
            network(200, 100, 1, 10_000_000, 0),
        ]))
        .into();
    input.tables[TableId::StateResistance.index()] =
        TableView::Complete(TableRows::StateResistance(
            (1..=3)
                .map(|state| StateResistanceRow {
                    state,
                    country: 100,
                    level: i64::from(state) * 500_000,
                    compliance: i64::from(state) * 800_000,
                    ..Default::default()
                })
                .collect(),
        ))
        .into();
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new().with_change_facts();
    for step in 0..10 {
        input.hour += 1;
        if step == 2 || step == 3 {
            let TableView::Complete(TableRows::CountryLinks(rows)) =
                &mut *input.tables[TableId::CountryLinks.index()]
            else {
                unreachable!()
            };
            rows[1].canonical_slot = if step == 2 { 0 } else { 1 };
        }
        if step == 4 {
            let TableView::Complete(TableRows::States(rows)) =
                &mut *input.tables[TableId::States.index()]
            else {
                unreachable!()
            };
            rows[1].controller = 100;
        }
        if step == 5 {
            let TableView::Complete(TableRows::IntelNetworks(rows)) =
                &mut *input.tables[TableId::IntelNetworks.index()]
            else {
                unreachable!()
            };
            if let Some(row) = rows.first_mut() {
                row.local_strength_raw += 250_000;
                row.local_quiet ^= 1;
            }
        }
        if step == 6 {
            let TableView::Complete(TableRows::StateResistance(rows)) =
                &mut *input.tables[TableId::StateResistance.index()]
            else {
                unreachable!()
            };
            rows.remove(0);
        }
        if step == 7 {
            let TableView::Complete(TableRows::IntelNetworks(rows)) =
                &mut *input.tables[TableId::IntelNetworks.index()]
            else {
                unreachable!()
            };
            rows.push(IntelNetworkRow {
                owner_country: 100,
                target_country: 200,
                state_id: -1,
                local_strength_raw: 100_000,
                ..Default::default()
            });
            rows.sort_by_key(replay_data::rows::RowCodec::key);
        }
        let encoded = encoder.encode(input.clone(), step == 0).unwrap();
        let frame = decoder.advance(&encoded).unwrap();
        let inputs = Inputs {
            content: &content,
            map: &map,
            frame,
            selected: Some(0),
            world: None,
            colour: None,
        };
        for stock in [Stock::Operatives, Stock::Resistance, Stock::Compliance] {
            let first = checked_fill(&MapMode::Stock(stock), &inputs).unwrap();
            let again = checked_fill(&MapMode::Stock(stock), &inputs).unwrap();
            let (Some(first), Some(again)) = (
                first.draw.and_then(|draw| draw.layer),
                again.draw.and_then(|draw| draw.layer),
            ) else {
                continue;
            };
            assert!(first.styles.ptr_eq(&again.styles));
            assert!(Arc::ptr_eq(&first.region_of.0, &again.region_of.0));
        }
        if step == 9 {
            for selected in [None, Some(1), Some(1501), Some(0)] {
                let inputs = Inputs { selected, ..inputs };
                for stock in [Stock::Operatives, Stock::Resistance, Stock::Compliance] {
                    let _ = checked_fill(&MapMode::Stock(stock), &inputs);
                }
            }
        }
    }
}
