use super::*;
use installed_content_core::scripted_map_modes;
use installed_content_core::{map::CountryColors, map::ProvinceDefinition};
use replay_data::{
    rows::BuildingRow, rows::CapitulationRow, rows::CountryAppearanceRow,
    rows::CountryCounterInputRow, rows::CountryLinkRow, rows::CountryRow, rows::FactionMemberRow,
    rows::FactionRow, rows::IntelNetworkRow, rows::LexicalTokenRow, rows::PlayerRow,
    rows::ProvinceRow, rows::Recorded, rows::ResearchTechnologyRow, rows::StateRow,
    rows::SubjectRow, rows::TableId, rows::TableRows, rows::TechnologyDefinitionRow,
};
use replay_frame::{frame::Metrics, frame::TableView};

/// Synthetic scripted modes have a complete, empty character catalogue.
/// Keyword classification must not inherit an unrelated unavailable input.
fn with_empty_character_templates(
    mut facts: dashboard_text::triggers::ContentFacts<'_>,
) -> dashboard_text::triggers::ContentFacts<'_> {
    static ROLES: std::sync::OnceLock<dashboard_text::triggers::LeaderRoles> =
        std::sync::OnceLock::new();
    facts.leader_roles =
        Ok(ROLES.get_or_init(|| dashboard_text::triggers::LeaderRoles::new(Vec::new())));
    facts
}

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
        defines: defines(),
        ideologies: Arc::new(installed_content_core::ideologies::Ideologies::default()),
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

/// GER (raw 100, slot 0) controls provinces 1-2 in state 1, SOV (raw 200, slot 1) province 3 in
/// state 2 and ITA (raw 300, slot 2) province 4 in state 3; GER and ITA share a faction and a
/// human plays SOV.
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
    // Every state is a town.
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
    // Standalone colour callbacks need proof that no effect refresh inherited
    // a caller's temporary store in this hour.
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
fn states_take_their_category_colour_and_name_what_they_cannot_draw() {
    // State 1 is a town, state 2 a colourless rural state, state 3's category has no colour
    // the content reads; the category tokens fold to the declared names.
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
        fill(
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
    // The states draw over the political countries.
    assert_eq!(draw.channels, [Source::Country, Source::Mode]);
    assert_eq!(draw.country, None);
    // The countries take their own defines into channel 1.
    assert_eq!(
        draw.channel_constants(),
        [Some([0.0, 0.973]), Some([0.4, 0.973])]
    );
    let layer = draw.layer.unwrap();
    assert_eq!(layer.region_of, [0, 2, 2, 3, 0, 0]);
    assert_eq!(layer.styles[&2].color, [0, 200, 0, 255]);
    // A category without a colour takes the fixed blue; one whose colour is unknown is refused.
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
    // The live category decides: state 1 recategorised as rural turns blue.
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
    // The unrecorded state is named; the icons are drawn by their own layer.
    assert_eq!(fill.warnings.len(), 1, "{:?}", fill.warnings);
    assert!(
        !fill
            .warnings
            .iter()
            .any(|why| why.contains("icons are not drawn"))
    );
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
    assert_eq!(refused.warnings.len(), 1);
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
        let expected = stock == Stock::Supply;
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
    ] {
        let fill = run(
            MapMode::Stock(stock),
            &content(MapModes::default()),
            Some(0),
        );
        assert_eq!(fill.draw.unwrap().channels, channels, "{stock:?}");
        assert_eq!(fill.warnings.len(), 1);
        if stock == Stock::StrategicAir {
            assert!(fill.warnings[0].starts_with("Air-supply regional summary operands"));
            assert!(fill.warnings[0].contains("recording cost rules"));
        }
    }
    // The raid and resources modes are the countries alone, with nothing left to refuse: the
    // resources mode's state icons draw.
    for stock in [Stock::Raids, Stock::Resources] {
        let fill = run(
            MapMode::Stock(stock),
            &content(MapModes::default()),
            Some(0),
        );
        assert_eq!(fill.draw.unwrap().channels, [Country, Empty], "{stock:?}");
        assert!(fill.warnings.is_empty(), "{stock:?}");
    }
    // The navy mode keeps the political channels.
    let navy = run(
        MapMode::Stock(Stock::StrategicNavy),
        &content(MapModes::default()),
        Some(0),
    );
    assert_eq!(navy.draw.unwrap().channels, POLITICAL);
    assert!(navy.warnings.is_empty(), "{:?}", navy.warnings);
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
    // Selecting a mode rebuilds both layers without a historical refresh interval.
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
    engine_executable::executable_tables::initialize_for_tests()
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
    let names = installed_content_core::trigger_keys::TriggerNames {
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
            .any(|warning| warning.contains("CountryStabilityInputs")
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
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    fn set<T: TableSchema>(f: &mut DecodedFrame, rows: Vec<T::Row>) {
        f.tables[T::ID.index()] = TableView::Complete(TableRows::from_rows::<T>(rows)).into();
    }
    let mut modes = MapModes::default();
    scripted_map_modes::parse_into(
        "test.txt",
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
    let names = installed_content_core::trigger_keys::TriggerNames::default();
    let dlcs = BTreeSet::new();
    let aliases = Arc::default();
    let render = |f: &DecodedFrame| {
        let world = World::read(
            f,
            with_empty_character_templates(ContentFacts::content_only(&dlcs, &names, &aliases)),
        );
        fill(
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
    // A poisoned certificate can leave the table unavailable, or complete with
    // unrelated metadata but no acquired country-local roster witnesses.
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
                .filter(|why| why.contains("ResearchModifiers"))
                .collect();
            if missing_witness {
                // Three countries have three distinct missing local inputs;
                // each target is counted once against its own roster.
                assert_eq!(roster_refusals.len(), 3, "{:?}", refused.warnings);
                for country in 0..3 {
                    assert_eq!(
                        roster_refusals
                            .iter()
                            .filter(|why| why.contains(&format!("Country({country})"))
                                && why.contains("1 targets"))
                            .count(),
                        1,
                        "{:?}",
                        refused.warnings
                    );
                }
            } else {
                // Whole-table unavailability is one input used by all targets.
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
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let content = content(MapModes::default());
    let map = map();
    let mut f = frame();
    if let TableView::Complete(TableRows::CountryCounterInputs(rows)) =
        &mut *f.tables[TableId::CountryCounterInputs.index()]
    {
        for row in rows {
            row.owned_states = None.into();
        }
    }
    let names = installed_content_core::trigger_keys::TriggerNames::default();
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
        color: installed_content_core::parser::parse(
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
    assert!(result.refusals[0].contains("CountryCounterInputs.owned_states"));
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
    engine_executable::executable_tables::initialize_for_tests().unwrap();
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
    let names = installed_content_core::trigger_keys::TriggerNames::default();
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
        color: installed_content_core::parser::parse(
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
    // Reuse the same World's failed lexical cache on the second presentation.
    for _ in 0..2 {
        let refused =
            colour_script::borders("test", "top", &layer, &inputs, None, true, &mut Vec::new());
        assert_eq!(refused.refusals.len(), 1, "{:?}", refused.refusals);
        let why = &refused.refusals[0];
        assert!(
            why.contains("LexicalTokens") && why.contains("3 targets"),
            "{why}"
        );
        assert!(
            !why.contains("ResearchModifiers") && !why.contains("Country("),
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
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let content = content(MapModes::default());
    let map = map();
    let mut f = frame();
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
    let names = installed_content_core::trigger_keys::TriggerNames::default();
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
            color: installed_content_core::parser::parse(body.as_bytes())
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
            vec!["ResearchModifiers"],
        ),
        (
            "optional division destination insertion without store",
            r#"
            set_temp_variable = { alpha = 1 }
            if = { limit = { has_stability > 0.5 } divide_temp_variable = { var = red?1 value = 0 } }
        "#,
            vec!["CountryStabilityInputs"],
        ),
        (
            "optional modulo destination insertion without store",
            r#"
            set_temp_variable = { alpha = 1 }
            if = { limit = { has_stability > 0.5 } modulo_temp_variable = { var = red?1 value = 0 } }
        "#,
            vec!["CountryStabilityInputs"],
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
            vec!["CountryStabilityInputs"],
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
            vec!["CountryStabilityInputs"],
        ),
        (
            "two independent inputs",
            r#"
            set_temp_variable = { alpha = 1 }
            if = { limit = { has_stability > 0.5 } set_temp_variable = { red = 1 } }
            if = { limit = { is_in_tech_sharing_group = mod_group } set_temp_variable = { blue = 1 } }
        "#,
            vec!["CountryStabilityInputs", "ResearchModifiers"],
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
        // Refusing a refresh also withholds a previously painted border.
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
    assert!(one_target.refusals[0].contains("CountryStabilityInputs"));
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
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let content = content(MapModes::default());
    let map = map();
    let mut f = frame();
    // Keep the filter initiator/candidate separate from the reserved country.
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
    let names = installed_content_core::trigger_keys::TriggerNames::default();
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
                color: installed_content_core::parser::parse(
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
                        installed_content_core::parser::parse(
                            format!("{candidate} {filter} = {{ {condition} }}").as_bytes(),
                        )
                        .unwrap()
                        .items,
                    ),
                    ..baseline.clone()
                };
                // Reuse the parsed layer and World's lazy indexes, with painted
                // previous borders proving an uncertain selection clears them.
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
                                "CountryStabilityInputs"
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
    engine_executable::executable_tables::initialize_for_tests().unwrap();
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
    let names = installed_content_core::trigger_keys::TriggerNames::default();
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
            color: installed_content_core::parser::parse(b"set_temp_variable = { alpha = 1 }")
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
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    use colour_script::{TargetId, borders};
    use dashboard_text::triggers::{ContentFacts, World};
    let mut modes = MapModes::default();
    scripted_map_modes::parse_into("test.txt", br#"scripted_map_modes = {
        split = { top = { type = state_controller targets = { targets = { 1 } }
            color = { set_temp_variable = { alpha = 1 } FROM = { FROM = { tag = SOV } } set_temp_variable = { red = 1 } }
        } }
    }"#, &mut modes).unwrap();
    let content = content(modes);
    let map = map();
    let (names, dlcs, aliases) = (
        installed_content_core::trigger_keys::TriggerNames::default(),
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
            // State 1's own controller is GER even though one province is SOV.
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
        "test.txt",
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
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let mut modes = MapModes::default();
    scripted_map_modes::parse_into("test.txt", br#"scripted_map_modes = {
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
            targets = { target_root_trigger = { OR = { check_variable = { inherited = 0 } always = yes } } targets = { SOV } }
            color = { set_temp_variable = { red = 0 } set_temp_variable = { green = 1 }
                set_temp_variable = { blue = 0 } set_temp_variable = { alpha = 1 }
                set_temp_variable = { highlighted = 0 } }
        } }
        transparent = { top = { type = country color = { set_temp_variable = { alpha = 0 } } } }
    }"#, &mut modes).unwrap();
    let content = content(modes);
    let (mut frame, map) = (frame(), map());
    let names = installed_content_core::trigger_keys::TriggerNames {
        scripted: Default::default(),
        counts: Some(Default::default()),
    };
    let (dlcs, aliases) = (BTreeSet::new(), Arc::default());
    let run = |name: &str, frame: &DecodedFrame, previous: Option<&ColourState>| {
        let world = World::read(
            frame,
            with_empty_character_templates(ContentFacts::content_only(&dlcs, &names, &aliases)),
        );
        fill(
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
        // First presentation, retained-border refresh, and an event coinciding
        // with a scheduled refresh must all honour inherited TLS availability.
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
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    let mut modes = MapModes::default();
    scripted_map_modes::parse_into(
        "test.txt",
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
    let names = installed_content_core::trigger_keys::TriggerNames {
        scripted: Default::default(),
        counts: Some(Default::default()),
    };
    let (dlcs, aliases) = (BTreeSet::new(), Arc::default());
    let run = |name: &str, frame: &DecodedFrame, previous: Option<&ColourState>| {
        let world = World::read(
            frame,
            with_empty_character_templates(ContentFacts::content_only(&dlcs, &names, &aliases)),
        );
        fill(
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
        // Exercise the actual first/daily dispatch, then the stricter retained
        // path between scheduled refreshes with the same uncertain evidence.
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
                    // Unknown event coverage cannot certify retained borders,
                    // even for a callback whose outputs would be independent.
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
    engine_executable::executable_tables::initialize_for_tests()
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
    let names = installed_content_core::trigger_keys::TriggerNames {
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

/// Token 77 spells the infrastructure building `roads`; token 78 spells another building.
const ROADS: u32 = 77;

/// GER's state 1, SOV's state 2 and ITA's state 3, with the given building levels and
/// technology rows.
fn infrastructure_frame(
    levels: &[(u32, u32, i16)],
    research: Vec<ResearchTechnologyRow>,
    definitions: Vec<TechnologyDefinitionRow>,
) -> DecodedFrame {
    let mut frame = frame();
    frame.tables[TableId::States.index()] = TableView::Complete(TableRows::States(
        [(1, 100), (2, 200), (3, 300)]
            .into_iter()
            .map(|(id, controller)| StateRow {
                id,
                owner: controller,
                controller,
                ..Default::default()
            })
            .collect(),
    ))
    .into();
    frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: ROADS,
                literal: b"Roads".to_vec(),
            },
            LexicalTokenRow {
                token: 78,
                literal: b"arms_factory".to_vec(),
            },
        ]))
        .into();
    frame.tables[TableId::StateBuildings.index()] = TableView::Complete(TableRows::StateBuildings(
        levels
            .iter()
            .map(|&(location, building_type, level)| BuildingRow {
                location,
                building_type,
                level,
                ..Default::default()
            })
            .collect(),
    ))
    .into();
    frame.tables[TableId::ResearchTechnologies.index()] =
        TableView::Complete(TableRows::ResearchTechnologies(research)).into();
    frame.tables[TableId::TechnologyDefinitions.index()] =
        TableView::Complete(TableRows::TechnologyDefinitions(definitions)).into();
    frame
}

fn infrastructure_fill(infrastructure: infrastructure::Content, frame: &DecodedFrame) -> ModeFill {
    let content = Content {
        infrastructure,
        ..content(MapModes::default())
    };
    fill(
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
fn infrastructure_blends_each_states_level_over_its_maximum_and_clamps_it() {
    let roads = infrastructure::for_tests("roads", 5, true, None, Vec::new());
    // State 1 at two of five, state 2 past its maximum, state 3 without an entry; another
    // building's level is not the infrastructure level.
    let frame = infrastructure_frame(
        &[(1, ROADS, 2), (2, ROADS, 7), (3, 78, 4)],
        Vec::new(),
        Vec::new(),
    );
    let fill = infrastructure_fill(roads.clone(), &frame);
    assert!(fill.warnings.is_empty(), "{:?}", fill.warnings);
    let draw = fill.draw.unwrap();
    assert_eq!(draw.channels, [Source::Mode, Source::Country]);
    let layer = draw.layer.unwrap();
    assert_eq!(layer.styles[&2].color, pack([0.64, 0.435, 0.04, 0.3]));
    assert_eq!(layer.styles[&3].color, pack([0.1, 0.9, 0.1, 0.6]));
    assert_eq!(layer.styles[&4].color, pack([1.0, 0.125, 0.0, 0.1]));
    assert_eq!(layer.gradient, Some(LEVEL_GRADIENT));
    assert_eq!(layer.thickness, Some(5.0));
    assert_eq!(layer.channel, [0.0, 0.973]);
    assert_eq!(&layer.region_of[1..6], &[2, 2, 3, 4, 0]);
    // The hover reads the level of the province's state: zero outside every state or entry.
    assert_eq!(roads.hover_level(&frame, 1), Ok(2));
    assert_eq!(roads.hover_level(&frame, 3), Ok(7));
    assert_eq!(roads.hover_level(&frame, 4), Ok(0));
    assert_eq!(roads.hover_level(&frame, 5), Ok(0));
    // A type no state status can hold leaves every state without a maximum.
    let provincial = infrastructure::for_tests("roads", 5, false, None, Vec::new());
    let layer = infrastructure_fill(provincial, &frame)
        .draw
        .unwrap()
        .layer
        .unwrap();
    for key in 2..=4 {
        assert_eq!(layer.styles[&key].color, pack(infrastructure::NO_MAXIMUM));
    }
}

#[test]
fn enabling_technologies_cap_the_maximum_by_the_controllers_completed_ones() {
    let technologies = vec![
        ("rail_a", Some(3)),
        ("rail_b", Some(4)),
        ("unrelated", None),
    ];
    let roads = infrastructure::for_tests("roads", 5, true, Some(technologies), Vec::new());
    let completed = engine_research::ctechnology::CLASSIFIER_COMPLETED;
    let research = |country, database_index, classifier| ResearchTechnologyRow {
        country,
        database_index,
        classifier,
        ..Default::default()
    };
    let definition = |database_index, id: &str, max_level| TechnologyDefinitionRow {
        database_index,
        id: id.as_bytes().to_vec(),
        max_level,
        ..Default::default()
    };
    let frame = |max_level| {
        infrastructure_frame(
            &[(1, ROADS, 2), (2, ROADS, 1), (3, ROADS, 3)],
            vec![
                // GER completed the level-three technology, SOV is still researching one and
                // ITA completed both.
                research(100, 1, completed),
                research(200, 2, completed + 1),
                research(300, 1, completed),
                research(300, 2, completed),
            ],
            vec![
                definition(1, "Rail_A", 1),
                definition(2, "rail_b", max_level),
                definition(3, "unrelated", 4),
            ],
        )
    };
    let layer = infrastructure_fill(roads.clone(), &frame(1))
        .draw
        .unwrap()
        .layer
        .unwrap();
    let blend = |t: f32| {
        let [from, to] = [[1.0, 0.125, 0.0, 0.1], [0.1, 0.9, 0.1, 0.6_f32]];
        pack(std::array::from_fn(|i| (to[i] - from[i]) * t + from[i]))
    };
    // GER: two of three; SOV: no maximum; ITA: three of four.
    assert_eq!(layer.styles[&2].color, blend(2.0 / 3.0));
    assert_eq!(layer.styles[&3].color, pack(infrastructure::NO_MAXIMUM));
    assert_eq!(layer.styles[&4].color, blend(0.75));
    // A completed repeatable technology was applied on every path, so its level holds.
    let fill = infrastructure_fill(roads.clone(), &frame(3));
    assert!(fill.warnings.is_empty(), "{:?}", fill.warnings);
    assert_eq!(
        fill.draw.unwrap().layer.unwrap().styles[&4].color,
        blend(0.75)
    );
    // A partly researched one may or may not have been applied: only its controller's states
    // are left unfilled, by name and count.
    let mut partly = frame(3);
    let TableView::Complete(TableRows::ResearchTechnologies(rows)) =
        &mut *partly.tables[TableId::ResearchTechnologies.index()]
    else {
        unreachable!()
    };
    rows[1].level = 1;
    let fill = infrastructure_fill(roads, &partly);
    let layer = fill.draw.unwrap().layer.unwrap();
    assert!(!layer.styles.contains_key(&3));
    assert_eq!(layer.styles[&2].color, blend(2.0 / 3.0));
    assert_eq!(fill.warnings.len(), 1);
    assert!(
        fill.warnings[0].contains("leaves 1 states") && fill.warnings[0].contains("rail_b"),
        "{:?}",
        fill.warnings
    );
}

#[test]
fn refused_infrastructure_content_and_country_statuses_are_named_and_counted() {
    let frame = infrastructure_frame(&[(1, ROADS, 2)], Vec::new(), Vec::new());
    let refused = infrastructure::for_tests(
        "roads",
        5,
        true,
        None,
        vec!["1 state categories set its maximum".to_owned()],
    );
    let fill = infrastructure_fill(refused, &frame);
    assert_eq!(
        fill.draw.unwrap().channels,
        [Source::Empty, Source::Country]
    );
    assert_eq!(
        fill.warnings,
        ["The infrastructure map mode is not drawn: 1 state categories set its maximum"]
    );
    let missing = infrastructure_fill(infrastructure::Content::default(), &frame);
    assert_eq!(
        missing.draw.unwrap().channels,
        [Source::Empty, Source::Country]
    );
    assert!(missing.warnings[0].contains("infrastructure = yes"));
    // A state whose building status names a country is left unfilled and counted.
    let mut frame = frame;
    frame.tables[TableId::StateBuildings.index()] =
        TableView::Complete(TableRows::StateBuildings(vec![BuildingRow {
            location: 2,
            building_type: ROADS,
            country: 200,
            level: 1,
            ..Default::default()
        }]))
        .into();
    let roads = infrastructure::for_tests("roads", 5, true, None, Vec::new());
    let fill = infrastructure_fill(roads, &frame);
    let layer = fill.draw.unwrap().layer.unwrap();
    assert!(!layer.styles.contains_key(&3));
    assert_eq!(layer.region_of[3], 0);
    assert_eq!(fill.warnings.len(), 1);
    assert!(fill.warnings[0].starts_with("The infrastructure map mode leaves 1 states unfilled"));
}

#[test]
fn a_states_live_category_lowers_its_infrastructure_maximum() {
    let roads = infrastructure::for_tests("roads", 5, true, None, Vec::new())
        .with_categories(&[("Ridge", 2), ("plateau", 9)]);
    let mut frame = infrastructure_frame(
        &[(1, ROADS, 1), (2, ROADS, 1), (3, ROADS, 5)],
        Vec::new(),
        Vec::new(),
    );
    let TableView::Complete(TableRows::States(states)) =
        &mut *frame.tables[TableId::States.index()]
    else {
        unreachable!()
    };
    // State 1 is a ridge, state 2 a town no category caps, state 3 a plateau above the type's own.
    for (state, token) in states.iter_mut().zip([90, 91, 92]) {
        state.category = Recorded::Value(Some(token));
    }
    let TableView::Complete(TableRows::LexicalTokens(tokens)) =
        &mut *frame.tables[TableId::LexicalTokens.index()]
    else {
        unreachable!()
    };
    for (token, literal) in [(90, "ridge"), (91, "town"), (92, "PLATEAU")] {
        tokens.push(LexicalTokenRow {
            token,
            literal: literal.as_bytes().to_vec(),
        });
    }
    let fill = infrastructure_fill(roads.clone(), &frame);
    assert!(fill.warnings.is_empty(), "{:?}", fill.warnings);
    let layer = fill.draw.unwrap().layer.unwrap();
    let blend = |t: f32| {
        let [from, to] = [[1.0, 0.125, 0.0, 0.1], [0.1, 0.9, 0.1, 0.6_f32]];
        pack(std::array::from_fn(|i| (to[i] - from[i]) * t + from[i]))
    };
    assert_eq!(layer.styles[&2].color, blend(0.5));
    assert_eq!(layer.styles[&3].color, blend(0.2));
    assert_eq!(layer.styles[&4].color, blend(1.0));
    // A refused category read leaves its state unfilled and counted.
    let TableView::Complete(TableRows::States(states)) =
        &mut *frame.tables[TableId::States.index()]
    else {
        unreachable!()
    };
    states[0].category = Recorded::Value(None);
    let fill = infrastructure_fill(roads, &frame);
    assert!(!fill.draw.unwrap().layer.unwrap().styles.contains_key(&2));
    assert_eq!(fill.warnings.len(), 1);
    assert!(fill.warnings[0].contains("1 states unfilled whose category read was refused"));
}

#[test]
fn an_incomplete_dictionary_without_the_infrastructure_token_is_refused_not_zero() {
    let roads = infrastructure::for_tests("roads", 5, true, None, Vec::new());
    let mut frame = infrastructure_frame(&[(1, ROADS, 4)], Vec::new(), Vec::new());
    // The hour's dictionary lost rows, the infrastructure token among them.
    frame.tables[TableId::LexicalTokens.index()] = TableView::Truncated {
        rows: TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 78,
            literal: b"arms_factory".to_vec(),
        }]),
        missing: 1,
    }
    .into();
    let fill = infrastructure_fill(roads.clone(), &frame);
    assert_eq!(
        fill.draw.unwrap().channels,
        [Source::Empty, Source::Country]
    );
    assert!(
        fill.warnings[0].contains("dictionary is incomplete"),
        "{:?}",
        fill.warnings
    );
    assert!(roads.hover_level(&frame, 1).is_err());
    // A complete dictionary that does not name the type leaves every level zero.
    frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(Vec::new())).into();
    assert_eq!(roads.hover_level(&frame, 1), Ok(0));
}

#[test]
fn each_recorded_declaration_of_a_repeated_technology_gives_its_own_unlock() {
    // Two declarations of `t`: the first enables level 3, the second level 4.
    let technologies = vec![("t", Some(3)), ("t", Some(4))];
    let roads = infrastructure::for_tests("roads", 5, true, Some(technologies), Vec::new());
    let completed = engine_research::ctechnology::CLASSIFIER_COMPLETED;
    let research = |country, database_index| ResearchTechnologyRow {
        country,
        database_index,
        classifier: completed,
        ..Default::default()
    };
    let definition = |database_index, id: &str| TechnologyDefinitionRow {
        database_index,
        id: id.as_bytes().to_vec(),
        max_level: 1,
        ..Default::default()
    };
    // GER completed the first declaration (index 1), ITA the second (index 2).
    let frame = |definitions| {
        infrastructure_frame(
            &[(1, ROADS, 3), (2, ROADS, 1), (3, ROADS, 2)],
            vec![research(100, 1), research(300, 2)],
            definitions,
        )
    };
    let fill = infrastructure_fill(
        roads.clone(),
        &frame(vec![definition(1, "t"), definition(2, "T")]),
    );
    assert!(fill.warnings.is_empty(), "{:?}", fill.warnings);
    let layer = fill.draw.unwrap().layer.unwrap();
    let blend = |t: f32| {
        let [from, to] = [[1.0, 0.125, 0.0, 0.1], [0.1, 0.9, 0.1, 0.6_f32]];
        pack(std::array::from_fn(|i| (to[i] - from[i]) * t + from[i]))
    };
    // GER: three of three; SOV completed neither; ITA: two of four.
    assert_eq!(layer.styles[&2].color, blend(1.0));
    assert_eq!(layer.styles[&3].color, pack(infrastructure::NO_MAXIMUM));
    assert_eq!(layer.styles[&4].color, blend(0.5));
    // A recorded declaration whose index names no content declaration of its id hides nothing
    // while nobody researched it.
    let unjoined = vec![definition(1, "t"), definition(2, "t"), definition(3, "t")];
    let fill = infrastructure_fill(roads.clone(), &frame(unjoined.clone()));
    assert!(fill.warnings.is_empty(), "{:?}", fill.warnings);
    let layer = fill.draw.unwrap().layer.unwrap();
    assert_eq!(layer.styles[&2].color, blend(1.0));
    assert_eq!(layer.styles[&4].color, blend(0.5));
    // Once SOV completes it, only SOV's states are left unfilled, by name and count.
    let mut completed = frame(unjoined);
    let TableView::Complete(TableRows::ResearchTechnologies(rows)) =
        &mut *completed.tables[TableId::ResearchTechnologies.index()]
    else {
        unreachable!()
    };
    rows.push(research(200, 3));
    let fill = infrastructure_fill(roads, &completed);
    let layer = fill.draw.unwrap().layer.unwrap();
    assert!(!layer.styles.contains_key(&3));
    assert_eq!(layer.styles[&2].color, blend(1.0));
    assert_eq!(layer.styles[&4].color, blend(0.5));
    assert_eq!(fill.warnings.len(), 1);
    assert!(
        fill.warnings[0].contains("leaves 1 states")
            && fill.warnings[0].contains("t (its declaration"),
        "{:?}",
        fill.warnings
    );
}

#[test]
fn scripted_invasion_failures_withhold_colours_on_cold_and_warm_caches() {
    use dashboard_text::triggers::{ContentFacts, World};
    engine_executable::executable_tables::initialize_for_tests().unwrap();
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
        let names = installed_content_core::trigger_keys::TriggerNames::default();
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
                color: installed_content_core::parser::parse(body.as_bytes())
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
                    "Provinces"
                } else {
                    "CountryLinks"
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
