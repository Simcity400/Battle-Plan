use super::*;
use installed_content::map::{CountryColors, ProvinceDefinition};
use installed_content::scripted_map_modes;
use replay::{
    frame::{Metrics, TableView},
    rows::{
        CapitulationRow, CountryAppearanceRow, CountryCounterInputRow, CountryLinkRow, CountryRow,
        FactionMemberRow, FactionRow, IntelNetworkRow, PlayerRow, ProvinceRow, Recorded, StateRow,
        SubjectRow, TableId, TableRows,
    },
};

/// The shipped defines' values.
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
        very_good_opinion: 50,
        very_bad_opinion: -50,
        resistance_colors: ColorList(vec![
            0.0, 0.0, 1.0, 0.0, 0.0, //
            0.0, 1.0, 1.0, 0.0, 0.0, //
            1.0, 1.0, 1.0, 0.0, 0.1, //
            30.0, 1.0, 1.0, 0.0, 0.3, //
            100.0, 1.0, 0.0, 0.0, 0.3,
        ]),
        compliance_colors: ColorList(vec![0.0, 0.3, 0.6, 0.6, 0.05, 100.0, 0.3, 0.9, 1.0, 0.5]),
        resistance_uninitialised: [1.0, 1.0, 1.0, 0.05],
        manpower_max: 20_000_000.0,
        manpower_colors: [[0.2, 0.2, 0.7, 0.18], [1.0, 0.125, 0.0, 0.6]],
        network_colors: [[0.1, 0.1, 0.5, 0.2], [0.4, 0.3, 0.9, 1.0]],
        quiet_network_colors: [[0.1, 0.5, 0.1, 0.2], [0.4, 0.9, 0.3, 1.0]],
        configurable_slots: [4, 10],
        terrain: terrain::Defines {
            alpha: [0.5, 0.8],
            channel: [0.39, 0.973],
        },
    }
}

fn content(scripted: MapModes) -> Content {
    Content {
        defines: defines(),
        ideologies: Arc::new(installed_content::ideologies::Ideologies::default()),
        scripted,
        faction_templates: Ok(Arc::new(Default::default())),
        recorded_language: "english".to_owned(),
        terrain: Err("the terrain database did not read".to_owned()),
    }
}

fn map() -> MapData {
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

/// GER (raw 100, slot 0) controls provinces 1-2 in state 1, SOV (raw 200, slot 1) province 3 in
/// state 2 and ITA (raw 300, slot 2) province 4 in state 3; GER and ITA share a faction and a
/// human plays SOV.
fn frame() -> DecodedFrame {
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
    // A tag operand resolves the country's recorded owned-state count before considering
    // original-tag substitution; controlling provinces alone does not supply that count.
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
    tables[TableId::Players.index()] = TableView::Complete(TableRows::Players(vec![PlayerRow {
        id: 1,
        country: 200,
        ..Default::default()
    }]));
    DecodedFrame {
        branch: 1,
        hour: 42,
        tables: tables.map(Into::into),
        metrics: Metrics::default(),
    }
}

fn run(mode: MapMode, content: &Content, selected: Option<u32>) -> ModeFill {
    fill(
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
fn colours_pack_by_truncating_each_component() {
    assert_eq!(pack(DIPLOMACY_SELF), [25, 168, 25, 255]);
    assert_eq!(pack([0.7, 0.7, 0.7, 0.7]), [178, 178, 178, 178]);
}

#[test]
fn colour_lists_interpolate_between_the_rows_around_the_value() {
    let list = &defines().resistance_colors;
    // The first two rows share a threshold: a zero-width interval takes its first row.
    assert_eq!(list.color(0.0), [0.0, 1.0, 0.0, 0.0]);
    assert_eq!(list.color(0.5), [1.0, 1.0, 0.0, 0.05]);
    assert_eq!(list.color(65.0), [1.0, 0.5, 0.0, 0.3]);
    // Past the last threshold the last interval clamps; below the first one it starts there.
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
    // The variant replaces the country layer in channel 1; channel 2 shows nothing.
    assert_eq!(draw.channels, [Source::Country, Source::Empty]);
    let layer = draw.country.unwrap();
    assert_eq!(layer.region_of, [0, 1, 1, 2, 3, 0]);
    assert_eq!(layer.styles[&1].color, [51, 102, 153, 255]);
    assert_eq!(layer.styles[&3].color, [51, 102, 153, 255]);
    assert_eq!(layer.styles[&2].color, [178, 178, 178, 178]);
    assert_eq!(layer.thickness, None);
}

#[test]
fn players_fade_unplayed_countries_and_pulse_the_selected_one() {
    let fill = run(
        MapMode::Stock(Stock::Players),
        &content(MapModes::default()),
        Some(0),
    );
    let layer = fill.draw.unwrap().country.unwrap();
    // SOV is played: its own colour and alpha.
    assert_eq!(layer.styles[&2].color, [127, 0, 63, 255]);
    // GER is not played but selected: faded, pulsing and widened.
    let ger = layer.styles[&1];
    assert_eq!(ger.color, [191, 127, 63, 38]);
    assert!(ger.pulse);
    assert_eq!(ger.fill, 1.5);
    assert!(!layer.styles[&3].pulse);
}

#[test]
fn states_withhold_their_category_colour_and_name_why() {
    let fill = run(
        MapMode::Stock(Stock::States),
        &content(MapModes::default()),
        None,
    );
    let draw = fill.draw.unwrap();
    // The states draw over the political countries.
    assert_eq!(draw.channels, [Source::Country, Source::Mode]);
    assert_eq!(draw.country, None);
    // The countries take their own defines into channel 1.
    assert_eq!(
        draw.channel_constants(),
        [Some([0.0, 0.973]), Some([0.4, 0.973])]
    );
    // The current category is not recorded, so no state takes its history colour.
    let layer = draw.layer.unwrap();
    assert!(layer.styles.is_empty());
    assert_eq!(layer.region_of, [0; 6]);
    assert_eq!(layer.thickness, Some(5.0));
    assert_eq!(layer.channel, [0.4, 0.973]);
    assert!(
        fill.warnings
            .iter()
            .any(|line| line.contains("category colours are refused"))
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
    // The thresholds are exclusive.
    assert_eq!(
        Relation {
            opinion: 50,
            ..good
        }
        .color(&defines),
        Ok(None)
    );
    assert_eq!(
        Relation {
            opinion: -51,
            ..good
        }
        .color(&defines),
        Ok(Some(DIPLOMACY_BAD))
    );
    // Without the war goals, a country past war and faction cannot be coloured.
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
        fill(
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
    // A quarter of the range; past its end; below its start.
    assert_eq!(layer.styles[&2].color, pack([0.4, 0.18125, 0.525, 0.285]));
    assert_eq!(layer.styles[&3].color, pack([1.0, 0.125, 0.0, 0.6]));
    assert_eq!(layer.styles[&4].color, pack([0.2, 0.2, 0.7, 0.18]));
    assert_eq!(layer.gradient, Some(LEVEL_GRADIENT));
    assert_eq!(layer.thickness, Some(5.0));
    assert_eq!(layer.channel, [0.0, 0.973]);
    // Provinces join their state's region; a province outside every state joins none.
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
    // GER (raw 100) controls state 1, SOV (200) state 2 and ITA (300) state 3.
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
            // SOV's network in GER: 72.5% in state 1, and state 2, which GER does not
            // control.
            network(200, 100, 1, 7_250_000, 0),
            network(200, 100, 2, 10_000_000, 0),
            // SOV's quiet network in ITA at 30%, and a state the recording does not hold.
            network(200, 300, 3, 3_000_000, 1),
            network(200, 300, 99, 3_000_000, 0),
            // GER's own network, which SOV's view does not show.
            network(100, 200, 2, 10_000_000, 0),
        ]))
        .into();
    let content = content(MapModes::default());
    let run = |frame: &DecodedFrame, selected| {
        fill(
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
    // 72.5% blends by 0.725 * 0.725 between the low and high colours.
    assert_eq!(
        layer.styles[&2].color,
        pack([0.2576875, 0.205125, 0.71025, 0.6205])
    );
    // 30% blends by 0.3 * 0.5 between the quiet pair.
    assert_eq!(layer.styles[&4].color, pack([0.145, 0.56, 0.13, 0.32]));
    assert_eq!(layer.styles.len(), 2);
    assert_eq!(&layer.region_of[1..6], &[2, 2, 0, 4, 0]);
    assert_eq!(layer.gradient, Some(LEVEL_GRADIENT));
    assert_eq!(layer.thickness, Some(5.0));
    assert_eq!(layer.channel, [0.0, 0.973]);
    // The unrecorded state and the icons are named.
    assert_eq!(fill.warnings.len(), 2, "{:?}", fill.warnings);
    // A network at or past 100% takes the high colour; a share below one half blends by
    // half of itself.
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
    // GER's view shows its own network in SOV.
    let german = run(&frame, Some(0)).draw.unwrap().layer.unwrap();
    assert_eq!(german.styles.keys().copied().collect::<Vec<_>>(), [3]);
    // Without a viewing country, or without the networks, the layer is refused by name.
    let refused = run(&frame, None);
    assert_eq!(
        refused.draw.unwrap().channels,
        [Source::Country, Source::Empty]
    );
    assert_eq!(refused.warnings.len(), 2);
    let mut unavailable = frame.clone();
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
        // SOV is ITA's subject.
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
        "test.txt",
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
    // The navy mode shows the political channels but names the strategic regions.
    for stock in [Stock::StrategicAir, Stock::StrategicNavy] {
        assert_eq!(far(MapMode::Stock(stock)), NameLayer::StrategicRegions);
    }
    // Every stock mode writes nothing close in: the country names fade out.
    for stock in Stock::ALL {
        assert_eq!(layers(MapMode::Stock(stock)).near, NameLayer::Nothing);
    }
    // A scripted mode names what its far_text and near_text say; undeclared, nothing.
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
fn modes_no_recording_can_draw_say_why() {
    for stock in Stock::ALL {
        let expected = matches!(stock, Stock::Supply | Stock::Infrastructure);
        assert_eq!(stock.unavailable().is_some(), expected, "{stock:?}");
    }
}

#[test]
fn each_mode_takes_the_games_channel_pair_and_names_what_it_lacks() {
    use Source::{Country, Empty, Strategic};
    for (stock, channels) in [
        (Stock::StrategicAir, [Strategic, Empty]),
        (Stock::Supply, [Empty, Empty]),
        (Stock::Terrain, [Empty, Country]),
        (Stock::Infrastructure, [Empty, Country]),
        (Stock::Resources, [Country, Empty]),
        (Stock::Raids, [Country, Empty]),
    ] {
        let fill = run(
            MapMode::Stock(stock),
            &content(MapModes::default()),
            Some(0),
        );
        assert_eq!(fill.draw.unwrap().channels, channels, "{stock:?}");
        assert_eq!(fill.warnings.len(), 1);
    }
    // The navy mode keeps the political channels.
    let navy = run(
        MapMode::Stock(Stock::StrategicNavy),
        &content(MapModes::default()),
        Some(0),
    );
    assert!(navy.draw.is_none() && navy.warnings.len() == 1);
    assert_eq!(
        run(
            MapMode::Stock(Stock::Default),
            &content(MapModes::default()),
            None
        ),
        ModeFill::default()
    );
}

#[test]
fn a_scripted_mode_shows_two_game_layers_or_one_in_both_channels() {
    let mut modes = MapModes::default();
    scripted_map_modes::parse_into(
        "test.txt",
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
        fill(
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
    // States below diplomacy: each channel has its own layer.
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
    // The same layer below and above is one layer shown in both.
    let twice = run("twice");
    assert_eq!(twice.channels, [Source::Mode, Source::Mode]);
    assert!(twice.second.is_none());
}

#[test]
fn scripted_modes_delegate_game_layers_and_refuse_colour_scripts() {
    let mut modes = MapModes::default();
    scripted_map_modes::parse_into(
        "test.txt",
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
    // The top layer is channel 2, the bottom one channel 1.
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
    // Both colour layers refused by name, and the forced refreshes recordings lack.
    assert_eq!(scripted.warnings.len(), 3, "{:?}", scripted.warnings);
    assert!(
        scripted
            .warnings
            .iter()
            .any(|warning| warning.contains("force_update_map_mode not recorded"))
    );
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
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use dashboard_text::triggers::{ContentFacts, World};
    let mut modes = MapModes::default();
    scripted_map_modes::parse_into(
        "test.txt",
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
    let names = installed_content::trigger_keys::TriggerNames {
        scripted: Default::default(),
        counts: Some(Default::default()),
    };
    let dlcs = BTreeSet::new();
    let aliases = Arc::default();
    let world = World::read(&frame, ContentFacts::content_only(&dlcs, &names, &aliases));
    let map = map();
    let run = |name: &str| {
        fill(
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
    // The bottom layer is channel 1, the top channel 2.
    assert_eq!(
        draw.channels,
        [Source::Mode, Source::Second],
        "{:?}",
        painted.warnings
    );
    // Only SOV's callback holds: its controlled province is its own region, red clamped to 1,
    // pulsing, its thickness temp the fill, at the scripted width.
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
    // A callback that leaves alpha at 0 paints nothing; without thickness the fill is flat.
    let bottom = draw.layer.unwrap();
    assert!(bottom.styles.is_empty());
    assert_eq!(bottom.gradient, Some(colour_script::FLAT_GRADIENT));
    // A trigger the evaluator cannot decide is refused by name, per target.
    let refused = run("refused");
    assert!(
        refused
            .warnings
            .iter()
            .any(|warning| warning.contains("has_stability") && warning.contains("3 targets")),
        "{:?}",
        refused.warnings
    );
}

#[test]
fn colour_borders_keep_their_colour_until_their_provinces_change() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use colour_script::{Border, LayerState, TargetId, borders, refreshes};
    use dashboard_text::triggers::{ContentFacts, World};
    let mut modes = MapModes::default();
    scripted_map_modes::parse_into(
        "test.txt",
        br#"scripted_map_modes = { held = { top = { type = country color = { always = no } } } }"#,
        &mut modes,
    )
    .unwrap();
    let layer = modes.get("held").unwrap().top.clone();
    let content = content(modes);
    let (frame, map) = (frame(), map());
    let names = installed_content::trigger_keys::TriggerNames {
        scripted: Default::default(),
        counts: Some(Default::default()),
    };
    let (dlcs, aliases) = (BTreeSet::new(), Arc::default());
    let world = World::read(&frame, ContentFacts::content_only(&dlcs, &names, &aliases));
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
    // SOV still controls province 3; GER's provinces changed since its border was kept; a
    // country that has since lost its last province (slot 7) is no target any more.
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
    // A failing callback keeps an unchanged border's colour, and a changed list clears it.
    assert_eq!(kept[&TargetId::Country(1)].style, red);
    assert_eq!(kept[&TargetId::Country(0)].style, None);
    assert_eq!(kept[&TargetId::Country(0)].provinces, [1, 2]);
    assert_eq!(kept[&TargetId::Country(2)].style, None);
    // A target gone since keeps no border to draw over its province's new controller.
    assert!(!kept.contains_key(&TargetId::Country(7)));
    // Between refreshes the kept borders show as they are, their refusals reported again.
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
    // A first hour refreshes, as does each day's first hour of a daily mode.
    let state = ColourState {
        mode: "held".into(),
        layers: Default::default(),
    };
    assert!(refreshes(None, false, 43));
    assert!(refreshes(Some(&state), true, 48));
    assert!(!refreshes(Some(&state), true, 43));
    assert!(!refreshes(Some(&state), false, 48));
}

#[test]
fn the_terrain_mode_draws_its_layer_under_the_countries() {
    let mut content = content(MapModes::default());
    let layer = ModeLayer {
        region_of: vec![0, 1, 2],
        styles: BTreeMap::from([(
            1,
            Style {
                color: [10, 20, 30, 127],
                pulse: false,
                fill: 1.0,
            },
        )]),
        thickness: Some(0.0),
        gradient: Some(terrain::TERRAIN_GRADIENT),
        channel: [0.39, 0.973],
    };
    content.terrain = Ok(terrain::Built {
        layer: layer.clone(),
        warnings: vec!["one province left out".to_owned()],
    });
    let fill = run(MapMode::Stock(Stock::Terrain), &content, Some(0));
    let draw = fill.draw.unwrap();
    assert_eq!(draw.channels, [Source::Mode, Source::Country]);
    assert_eq!(draw.layer, Some(layer));
    assert_eq!(
        draw.channel_constants(),
        [Some([0.39, 0.973]), Some(defines().country_channel)]
    );
    assert_eq!(fill.warnings, ["one province left out"]);
}
