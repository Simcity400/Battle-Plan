use super::*;
use dashboard_core::test_fixtures::game;

pub(super) fn geography_catalogue() -> (crate::catalogue::Catalogue, crate::catalogue::tests::Folder)
{
    let (_, folder) = crate::catalogue::tests::folder_catalogue(&[
        (
            "common/defines/fixture.lua",
            "NDefines = { NGame = { MAX_SCRIPTED_LOC_RECURSION = 23 }, NBuildings = { MAX_BUILDING_LEVELS = 13 }, NMilitary = { RIVER_SMALL_START_INDEX = 0, RIVER_SMALL_STOP_INDEX = 7, RIVER_LARGE_STOP_INDEX = 12 } }",
        ),
        (
            "common/ideologies/fixture.txt",
            "ideologies = { mod_group = { types = { mod_type = {} } } }",
        ),
        (
            "common/terrain/fixture.txt",
            "categories = { mod_land = { movement_cost = 1 } mod_sea = { movement_cost = 1 } } terrain = { fixture = { type = mod_land color = { 0 } } }",
        ),
        (
            "common/scripted_localisation/fixture.txt",
            "defined_text = { name = Always text = { trigger = { always = yes } localization_key = TITLE } }
             defined_text = { name = Political text = { trigger = { has_government = mod_group } localization_key = POLITICAL } }
             defined_text = { name = Neighbors text = { trigger = { any_neighbor_state = { always = yes } } localization_key = NEIGHBORS } }",
        ),
        (
            "localisation/fixture_l_english.yml",
            "\u{feff}l_english:\n TITLE:0 \"Localized title\"\n POLITICAL:0 \"Localized political title\"\n NEIGHBORS:0 \"Localized neighbors\"\n mod_land:0 \"terrain_alias\"\n terrain_alias:0 \"Localized terrain\"\n",
        ),
        (
            "map/default.map",
            "definitions = definition.csv provinces = logs/provinces.bmp terrain = logs/terrain.bmp rivers = logs/rivers.bmp heightmap = heightmap.bmp tree_definition = trees.bmp continent = continents.txt adjacencies = adjacencies.csv adjacency_rules = adjacency_rules.txt",
        ),
        (
            "map/definition.csv",
            "0;0;0;0;land;false;mod_land;1\n1;1;0;0;land;false;mod_land;1\n2;2;0;0;land;false;mod_land;1\n3;3;0;0;sea;false;mod_sea;1\n",
        ),
        ("map/continents.txt", "continents = { mod_continent }"),
        ("map/adjacencies.csv", "header\n"),
        ("map/adjacency_rules.txt", ""),
        ("history/states/first.txt", "state = { id = 64 name = FIRST provinces = { 1 } }"),
        ("history/states/second.txt", "state = { id = 65 name = SECOND provinces = { 2 } }"),
    ]);
    std::fs::create_dir_all(folder.0.join("map/logs")).unwrap();
    let bitmap = |bits: u16, pixels: &[u8]| {
        let mut bytes = vec![0; 54];
        bytes[..2].copy_from_slice(b"BM");
        for (at, value) in [(10, 54_u32), (14, 40), (18, 4), (22, 1)] {
            bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes[26..28].copy_from_slice(&1_u16.to_le_bytes());
        bytes[28..30].copy_from_slice(&bits.to_le_bytes());
        bytes.extend_from_slice(pixels);
        bytes
    };
    for (name, bytes) in [
        (
            "provinces.bmp",
            bitmap(24, &[0, 0, 1, 0, 0, 2, 0, 0, 3, 0, 0, 3]),
        ),
        ("terrain.bmp", bitmap(8, &[0; 4])),
        ("rivers.bmp", bitmap(8, &[255; 4])),
    ] {
        std::fs::write(folder.0.join("map/logs").join(name), bytes).unwrap();
    }
    let layers = content_reader_layers::ResolvedLayers::from_folders(&[(&folder.0, &[])]).unwrap();
    let _reads = layers.report_uncovered_reads();
    (crate::catalogue::tests::layers_catalogue(layers), folder)
}

pub(super) fn geography_frame() -> DecodedFrame {
    use replay_data::{
        rows::PoliticalPartyRow, rows::ProvinceRow, rows::StateRow, rows::TableId, rows::TableRows,
    };
    use replay_frame::frame::TableView;
    let mut frame = crate::scripted_text::tests::complete_frame();
    *frame.tables[TableId::States.index()] = TableView::Complete(TableRows::States(
        [64, 65]
            .map(|id| StateRow {
                id,
                owner: 100,
                controller: 100,
                ..Default::default()
            })
            .into(),
    ));
    *frame.tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(
        [(1, 64), (2, 65)]
            .map(|(id, state)| ProvinceRow {
                id,
                state,
                owner: 100,
                controller: 100,
                ..Default::default()
            })
            .into(),
    ));
    *frame.tables[TableId::PoliticalParties.index()] =
        TableView::Complete(TableRows::PoliticalParties(vec![PoliticalPartyRow {
            country: 100,
            id: 3,
            token: b"mod_group".to_vec(),
            is_ruling: 1,
            ..Default::default()
        }]));
    frame
}

#[test]
fn scripted_political_and_always_titles_leave_geography_unread() {
    game::enter();
    let (cat, folder) = geography_catalogue();
    let path = folder.0.join("map/logs/provinces.bmp");
    let bitmap = std::fs::read(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    let content = cat.scripted_content().unwrap();
    let frame = geography_frame();
    let text = ScriptedText::new(&frame, &cat.localisation, content, 1);
    assert_eq!(
        text.expand("[Always] [Political]").as_deref(),
        Ok("Localized title Localized political title")
    );
    let inputs = &content.environment().unwrap().prepared;
    assert!(inputs.neighbors.get().is_none());
    assert!(inputs.terrain.get().is_none());
    inputs.assert_title_inputs_unread();
    assert!(
        cat.reader_warning_lines()
            .iter()
            .all(|line| !line.contains("map/logs/"))
    );

    std::fs::write(&path, bitmap).unwrap();
    assert_eq!(
        text.expand("[ROOT.64.Neighbors]").as_deref(),
        Ok("Localized neighbors")
    );
    assert!(inputs.terrain.get().is_none());
}

#[test]
fn reached_scripted_geography_preserves_values_diagnostics_and_cache_reuse() {
    game::enter();
    let (cat, folder) = geography_catalogue();
    let content = cat.scripted_content().unwrap();
    let frame = geography_frame();
    let text = ScriptedText::new(&frame, &cat.localisation, content, 1);
    let ((warnings, cached), _) = content_reader_layers::content::diagnostics::capture(|| {
        assert_eq!(
            text.expand("[ROOT.64.Neighbors]").as_deref(),
            Ok("Localized neighbors")
        );
        let inputs = &content.environment().unwrap().prepared;
        assert_eq!(
            crate::triggers::PreparedContent::geography(content)
                .neighbors()
                .unwrap()[&64],
            [65]
        );
        assert!(inputs.terrain.get().is_none());
        let warnings = cat.reader_warning_lines();
        assert_eq!(
            warnings
                .iter()
                .filter(|line| line.starts_with("Content: "))
                .count(),
            1
        );
        assert!(warnings.iter().any(|line| line.contains("map/logs/provinces.bmp")
            && line.contains("map/logs/terrain.bmp")));
        assert!(
            warnings
                .iter()
                .all(|line| !line.contains("map/logs/rivers.bmp"))
        );

        let world = content.recorded_world(&frame).unwrap();
        assert_eq!(world.coastal(1), Ok(true));
        assert_eq!(world.coastal(3), Ok(false));
        assert_eq!(world.coastal(999), Ok(false));
        assert_eq!(text.province_terrain(1), Ok(1));
        let warnings = cat.reader_warning_lines();
        assert_eq!(
            warnings
                .iter()
                .filter(|line| line.starts_with("Content: "))
                .count(),
            1
        );
        assert!(
            warnings
                .iter()
                .any(|line| line.contains("map/logs/rivers.bmp"))
        );
        let cached = crate::triggers::PreparedContent::geography(content)
            .terrain()
            .unwrap();
        (warnings, cached)
    });

    for name in ["provinces.bmp", "terrain.bmp", "rivers.bmp"] {
        std::fs::remove_file(folder.0.join("map/logs").join(name)).unwrap();
    }
    let (reused, _) = content_reader_layers::content::diagnostics::capture(|| {
        let next = ScriptedText::new(&frame, &cat.localisation, content, 1);
        assert_eq!(
            next.expand("[ROOT.64.Neighbors]").as_deref(),
            Ok("Localized neighbors")
        );
        assert_eq!(next.province_terrain(1), Ok(1));
        assert_eq!(content.recorded_world(&frame).unwrap().coastal(1), Ok(true));
        assert!(std::ptr::eq(
            cached,
            crate::triggers::PreparedContent::geography(content)
                .terrain()
                .unwrap()
        ));
        cat.reader_warning_lines()
    });
    assert_eq!(reused, warnings);
}

#[test]
fn reached_geography_caches_missing_and_malformed_map_refusals_without_refusing_titles() {
    game::enter();
    for malformed in [false, true] {
        let (cat, folder) = geography_catalogue();
        let path = folder.0.join("map/logs/provinces.bmp");
        let bitmap = std::fs::read(&path).unwrap();
        if malformed {
            std::fs::write(&path, b"synthetic invalid bitmap").unwrap();
        } else {
            std::fs::remove_file(&path).unwrap();
        }
        let content = cat.scripted_content().unwrap();
        let frame = geography_frame();
        let text = ScriptedText::new(&frame, &cat.localisation, content, 1);
        assert_eq!(
            text.expand("[Always] [Political]").as_deref(),
            Ok("Localized title Localized political title")
        );
        let refusal = text.expand("[ROOT.64.Neighbors]").unwrap_err();
        assert!(
            refusal.contains("scripted localisation Neighbors")
                && refusal.contains("any_neighbor_state"),
            "{refusal}"
        );
        let inputs = &content.environment().unwrap().prepared;
        assert!(inputs.neighbors.get().unwrap().is_err());
        assert!(inputs.terrain.get().is_none());
        let terrain_refusal = text.province_terrain(1).unwrap_err();
        assert!(terrain_refusal.contains("Terrain of province 1"));
        assert!(inputs.terrain.get().unwrap().is_none());
        let coastal_refusal = content
            .recorded_world(&frame)
            .unwrap()
            .coastal(1)
            .unwrap_err();
        assert!(coastal_refusal.contains("content movement map is unavailable"));

        std::fs::write(&path, bitmap).unwrap();
        let next = ScriptedText::new(&frame, &cat.localisation, content, 1);
        assert_eq!(next.expand("[ROOT.64.Neighbors]").unwrap_err(), refusal);
        assert_eq!(next.province_terrain(1).unwrap_err(), terrain_refusal);
        assert_eq!(
            next.expand("[Always] [Political]").as_deref(),
            Ok("Localized title Localized political title")
        );
    }
}

#[test]
fn description_append_preserves_active_cursor_graph_and_guard_state() {
    let mut graph = EventScopes::new(vec![
        EventScope {
            root: Link::Scope(1),
            from: Link::Scope(2),
            ..EventScope::of(EventObject::Country(7))
        },
        EventScope::of(EventObject::State(11)),
        EventScope::of(EventObject::Country(9)),
    ]);
    graph.variables.target_state = Some(11);
    graph
        .tooltip_targets
        .insert(1, vec![(b"parent".to_vec(), EventObject::State(11))]);
    let mut expanded = ExpansionScopes::new(&graph);
    expanded.on_large_stack = true;
    expanded.fresh_depth.set(9);
    expanded.active.borrow_mut().push(ActiveEntry {
        name: "synthetic".into(),
        at: 0,
        scopes_len: 3,
        bare: false,
        variables: graph.variables,
    });
    {
        let appended = expanded.append_description(&graph).unwrap();
        assert_eq!(appended.offset, 3);
        let active = expanded.active.borrow();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].at, 0);
        assert_eq!(active[0].scopes_len, 3);
        assert!(active[0].matches("synthetic", &expanded, appended.offset, false));
        assert_eq!(expanded.scope(3).root, Link::Scope(4));
        assert_eq!(expanded.scope(3).from, Link::Scope(5));
        assert_eq!(expanded.fresh_depth.get(), 9);
        assert!(expanded.on_large_stack);
        assert_eq!(
            expanded.targets.borrow().tooltip_targets[&4],
            graph.tooltip_targets[&1]
        );
    }
    assert_eq!(*expanded.scopes.borrow(), graph.scopes);
    assert_eq!(expanded.prev_distance.borrow().len(), 3);
    assert_eq!(
        expanded.targets.borrow().tooltip_targets,
        graph.tooltip_targets
    );
    assert_eq!(expanded.variables.get(), graph.variables);
    assert_eq!(expanded.active.borrow()[0].at, 0);
    let mut child = graph.clone();
    child.scopes[0].prev = Link::Scope(usize::MAX);
    assert_eq!(
        expanded.append_description(&child).err(),
        Some(format!(
            "scripted text event scope link {} overflows when rebased",
            usize::MAX
        ))
    );
    child.scopes[0].prev = graph.scopes[0].prev;
    let target = vec![(b"child".to_vec(), EventObject::State(12))];
    let first_missing_scope = child.scopes.len();
    child.tooltip_targets.insert(first_missing_scope, target.clone());
    assert_eq!(
        expanded.append_description(&child).err(),
        Some(format!(
            "scripted text tooltip target event scope {first_missing_scope} is not given"
        ))
    );
    child.tooltip_targets.remove(&first_missing_scope);
    child.saved_targets.insert(usize::MAX, target);
    assert_eq!(
        expanded.append_description(&child).err(),
        Some(format!(
            "scripted text saved target event scope {} is not given",
            usize::MAX
        ))
    );
    assert_eq!(*expanded.scopes.borrow(), graph.scopes);
    assert_eq!(*expanded.targets.borrow(), graph);
    graph.scopes[0].prev = Link::Scope(99);
    {
        let _appended = expanded.append_description(&graph).unwrap();
        assert!(!expanded.valid_links.get());
    }
    assert!(expanded.valid_links.get());
}

#[test]
fn scripted_text_cached_state_diagnostics_are_filtered_and_reported_once() {
    game::enter();
    use content_reader_states::{states::Diagnostic, states::MisreadCause};
    let (cat, _folder) = crate::scripted_text::tests::catalogue("", "");
    let warnings = vec![
        Diagnostic {
            kind: content_reader_parser::DiagnosticKind::Warning,
            message: "native template skip".into(),
            cause: None,
        },
        Diagnostic {
            kind: content_reader_parser::DiagnosticKind::Misread,
            message: "proven state recovery".into(),
            cause: Some(MisreadCause::InvalidImpassable),
        },
        Diagnostic {
            kind: content_reader_parser::DiagnosticKind::Misread,
            message: "unproven state recovery".into(),
            cause: None,
        },
        Diagnostic {
            kind: content_reader_parser::DiagnosticKind::Unsupported,
            message: "unsupported state content".into(),
            cause: None,
        },
    ];
    assert!(
        cat.state_content
            .set(Ok(StateContent {
                defines: content_reader_defines::defines::read(&cat.content.layers).unwrap(),
                states: Default::default(),
                names: StateNames::new(),
                warnings: warnings.clone(),
            }))
            .is_ok()
    );
    let expected = ["unproven state recovery", "unsupported state content"];
    assert_eq!(cat.reader_warning_lines(), expected);
    assert_eq!(cat.refusal_lines(), expected);
    let content = cat.scripted_content().unwrap();
    assert!(std::ptr::eq(
        cat.cached_states().unwrap(),
        content.cached_states().unwrap(),
    ));
    assert_eq!(cat.refusal_lines(), expected);
    let mut inputs = EnvironmentInputs::synthetic();
    inputs.warnings = live_state_warnings(&warnings).map(str::to_owned).collect();
    assert!(
        content
            .environment()
            .unwrap()
            .inputs
            .set(Ok(inputs))
            .is_ok()
    );
    assert_eq!(cat.reader_warning_lines(), expected);
    assert_eq!(cat.refusal_lines(), expected);
    cat.refuse("unrelated text failure", "synthetic text");
    assert_eq!(cat.reader_warning_lines(), expected);
    assert_eq!(cat.refusal_lines().len(), expected.len() + 1);
}

#[test]
fn scripted_text_state_display_shares_precedence_language_and_cached_failures() {
    game::enter();
    let (mut cat, _folder) =
        crate::scripted_text::tests::catalogue("", " MOD_STATE:0 \"Mod state\"\n SCRIPTED_STATE:0 \"[Root.GetName] Land\"\n");
    let content = cat.scripted_content().unwrap();
    assert!(content.state_content.get().is_none());
    let first_error = cat.cached_states().err().unwrap();
    assert!(content.state_content.get().is_some());
    assert_eq!(cat.cached_states().err().unwrap(), first_error);
    let names = BTreeMap::from([(64, Ok("MOD_STATE".into())), (65, Ok("SCRIPTED_STATE".into())), (66, Ok("MISSING_STATE".into()))]);
    let mut state = replay_data::rows::StateRow {
        id: 64,
        ..Default::default()
    };
    assert_eq!(
        state_display_name(&state, &cat, Ok(&names), StateNameUsage::Place).unwrap(),
        "Mod state"
    );
    let scripted_err = state_display_name(
        &replay_data::rows::StateRow { id: 65, ..Default::default() },
        &cat,
        Ok(&names),
        StateNameUsage::Place,
    )
    .unwrap_err();
    assert!(scripted_err.contains("SCRIPTED_STATE") && scripted_err.contains("[Root.GetName]"), "{scripted_err}");
    let missing_err = state_display_name(
        &replay_data::rows::StateRow { id: 66, ..Default::default() },
        &cat,
        Ok(&names),
        StateNameUsage::Place,
    )
    .unwrap_err();
    assert!(missing_err.contains("MISSING_STATE") && missing_err.contains("missing"), "{missing_err}");
    state.owner_name = b"Owner state".to_vec();
    assert_eq!(
        state_display_name(&state, &cat, Err("unread content"), StateNameUsage::Place).unwrap(),
        "Owner state"
    );
    state.custom_name = b"Custom state".to_vec();
    assert_eq!(
        state_display_name(&state, &cat, Ok(&names), StateNameUsage::Place).unwrap(),
        "Custom state"
    );
    cat.recorded_language = "other-language".into();
    assert!(
        state_display_name(&state, &cat, Ok(&names), StateNameUsage::Place)
            .unwrap_err()
            .contains("other-language")
    );
}

#[test]
fn state_templates_accept_synthesized_unclaimed_bmp_provinces() {
    content_reader_script::test_game::enter();
    let stack = content_reader_layers::test_support::Stack::new("state_templates_unclaimed_bmp");
    let base = stack.base();
    stack.write(
        &base,
        "common/defines/fixture.lua",
        "NDefines = { NBuildings = { MAX_BUILDING_LEVELS = 13 } }",
    );
    stack.write(
        &base,
        "map/default.map",
        "definitions = definition.csv provinces = provinces.bmp terrain = terrain.bmp rivers = rivers.bmp heightmap = heightmap.bmp tree_definition = trees.bmp",
    );
    stack.write(
        &base,
        "map/definition.csv",
        "0;0;0;0;land;false;unknown;0\n1;1;0;0;land;false;unknown;1\n",
    );
    stack.write(
        &base,
        "history/states/unclaimed.txt",
        "state = { id = 77 name = SYNTH_STATE provinces = { 1 2 } }",
    );
    let mut bmp = vec![0_u8; 62];
    bmp[..2].copy_from_slice(b"BM");
    for (at, value) in [(2, 62_u32), (10, 54), (14, 40), (18, 2), (22, 1)] {
        bmp[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    bmp[26..28].copy_from_slice(&1_u16.to_le_bytes());
    bmp[28..30].copy_from_slice(&24_u16.to_le_bytes());
    bmp[54..60].copy_from_slice(&[0, 0, 1, 7, 8, 9]);
    std::fs::write(base.join("map/provinces.bmp"), bmp).unwrap();

    let layers = stack.layers(&[]);
    let defines = content_reader_defines::defines::read(&layers).unwrap();
    let (states, names, _) = read_state_templates(&layers, &defines).unwrap();
    assert_eq!(states.definitions()[&77].provinces, [1, 2]);
    assert_eq!(names[&77], Ok("SYNTH_STATE".into()));
}
