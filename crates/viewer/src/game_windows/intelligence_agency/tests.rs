use super::*;
use content_reader_military::intelligence::AgencyUpgrade;
use dashboard_intelligence::products::intelligence::{Country, agencies::UpgradeItem};
use viewer_native_gui::{native_effects::Effect, native_gui::{SpriteInfo, SpriteKind, Texture}, native_text::Font};
use viewer_native_gui::test_support::gui;

const AGENCY_GUI: &[u8] = br#"
    guiTypes = {
        containerWindowType = {
            name = "synthetic_agency"
            size = { width = 320 height = 290 }
            containerWindowType = {
                name = "agency_branches"
                size = { width = 260 height = 100 }
                gridBoxType = { name = "upgrades_list" size = { width = 230 height = 80 } slotsize = { width = 70 height = 45 } format = "UPPER_LEFT" }
            }
            containerWindowType = {
                name = "agency_operations"
                size = { width = 270 height = 130 }
                containerWindowType = {
                    name = "operations_grid_container"
                    size = { width = 245 height = 100 }
                    instantTextboxType = { name = "operations_not_active" font = "Main_14" text = "Invented prerequisite" maxWidth = 210 maxHeight = 20 }
                    gridBoxType = { name = "operations_grid" size = { width = 210 height = 80 } slotsize = { width = 190 height = 55 } format = "UPPER_LEFT" }
                }
            }
            buttonType = { name = "crypto_tab_button" quadTextureSprite = "GFX_synthetic_tab" text = "SYNTHETIC_TAB_NAME" font = "Main_14" }
        }
        containerWindowType = {
            name = "upgrade_button"
            size = { width = 60 height = 35 }
            gridBoxType = { name = "researched_icons" size = { width = 50 height = 20 } slotsize = { width = 14 height = 14 } format = "UPPER_LEFT" }
        }
        containerWindowType = {
            name = "agency_upgrade_researched_icon"
            size = { width = 13 height = 13 }
            buttonType = { name = "upgrade_icon" quadTextureSprite = "GFX_synthetic_marker" }
        }
        containerWindowType = {
            name = "operation_view_entry"
            size = { width = 175 height = 44 }
            iconType = { name = "operation_bg" spriteType = "GFX_synthetic_tab" }
            containerWindowType = {
                name = "container_operation_info"
                size = { width = 160 height = 40 }
                buttonType = { name = "btn_target_flag" position = { x = 9 y = 5 } quadTextureSprite = "GFX_synthetic_shield" }
                containerWindowType = {
                    name = "container_network_strength"
                    size = { width = 42 height = 26 }
                    instantTextboxType = { name = "network_strength_text" font = "Main_14" maxWidth = 41 maxHeight = 24 }
                }
            }
            containerWindowType = {
                name = "container_operatives_required"
                size = { width = 35 height = 28 }
                instantTextboxType = { name = "operatives_required_text" font = "Main_14" maxWidth = 34 maxHeight = 27 }
            }
            instantTextboxType = { name = "operation_duration_text" font = "Main_14" text = "8" maxWidth = 52 maxHeight = 26 }
            containerWindowType = {
                name = "container_launch_operation"
                size = { width = 65 height = 24 }
                buttonType = { name = "btn_select_button" quadTextureSprite = "GFX_synthetic_tab" buttonFont = "Main_14" }
            }
            containerWindowType = {
                name = "container_tokens"
                size = { width = 76 height = 34 }
                iconType = { name = "synthetic_token" spriteType = "GFX_synthetic_tab" }
            }
        }
    }
"#;

fn agency_view() -> AgencyView {
    AgencyView {
        agency: Agency {
            country: Country { id: 8, tag: None },
            created: true,
            name: Some("Synthetic Bureau".to_owned()),
            name_missing: None,
            defense_raw: "0".to_owned(),
            defense_text: None,
            defense_value: None,
            active_count: 1,
            maximum: 3,
            upgrades: None,
        },
        branches: Ok(Vec::new()),
        operations: Ok(Vec::new()),
        upgrade_progress: "7.0".to_owned(),
        read_hour: 19,
        warnings: Vec::new(),
    }
}

fn agency_guis() -> native::Guis {
    content_reader_script::test_game::enter();
    super::super::load_native_guis_from_bytes("agency-fixture.gui", AGENCY_GUI).unwrap()
}

#[test]
fn empty_operations_replace_the_authored_prerequisite_and_unread_is_distinct() {
    let guis = agency_guis();
    let root = guis.template(b"synthetic_agency").unwrap();
    let mut resources = gui::fixture();
    resources.add_sprite("GFX_synthetic_tab", [65, 18], 1);
    let control = find_direct(root, "agency_operations/operations_grid_container/operations_not_active").unwrap();
    let mut instance = Instance::construct(root, [320, 290], &resources).unwrap();
    let mut view = agency_view();
    let mut tooltips = BTreeMap::new();

    configure_main(root, resources.localisation(), &mut instance, &mut tooltips, &view);
    assert_eq!(instance.control(root, control).unwrap().text.as_deref(), Some("No running operations recorded at this hour."));

    view.operations = Err("The synthetic operations were unread.".to_owned());
    configure_main(root, resources.localisation(), &mut instance, &mut tooltips, &view);
    assert_eq!(instance.control(root, control).unwrap().text.as_deref(), Some("n/a"));
    assert_eq!(tooltips[&control.path].body.as_deref(), Some("The synthetic operations were unread."));
}

#[test]
fn an_unavailable_marker_preserves_the_known_upgrade_level_as_text() {
    let guis = agency_guis();
    let root = guis.template(b"synthetic_agency").unwrap();
    let mut resources = gui::fixture();
    resources.add_sprite("GFX_synthetic_tab", [65, 18], 1);
    resources.add_sprite("GFX_synthetic_marker", [11, 11], 1);
    let layout = layout_window(root, &resources, [320, 290], |_| {}).unwrap();
    let sources = Sources { guis: &guis, resources: &resources, localisation: resources.localisation() };
    let mut view = agency_view();
    view.branches = Ok(vec![Branch {
        token: "survey".to_owned(),
        name: Ok("Survey".to_owned()),
        pips: vec![Pip { token: "archive".to_owned(), name: Ok("Archive".to_owned()), levels: 3, level: Ok(2) }],
    }]);

    let mut plan = ComposedPlan::default();
    compose_branches(&mut plan, &layout, root, &sources, &view).unwrap();
    let level = plan.text.iter().filter(|entry| entry.label == "Archive").collect::<Vec<_>>();
    assert_eq!(level.len(), 1);
    assert!(level[0].value.as_deref().is_some_and(|value| value.starts_with("2 / 3")));
    let art = plan.text.iter().find(|entry| entry.label == "Archive level marker").unwrap();
    assert!(art.value.as_deref().is_some_and(|reason| reason.starts_with("Not shown:") && reason.contains("no frames")));
    assert!(art.missing.is_none());
}

#[test]
fn the_disabled_tab_note_names_the_installed_control() {
    let guis = agency_guis();
    let root = guis.template(b"synthetic_agency").unwrap();
    let mut resources = gui::fixture();
    resources.add_sprite("GFX_synthetic_tab", [65, 18], 1);
    let localisation = [("SYNTHETIC_TAB_NAME".to_owned(), "Synthetic Signals".to_owned())].into_iter().collect();
    let mut instance = Instance::construct(root, [320, 290], &resources).unwrap();
    let mut tooltips = BTreeMap::new();

    configure_main(root, &localisation, &mut instance, &mut tooltips, &agency_view());
    let control = find_direct(root, "crypto_tab_button").unwrap();
    let tooltip = &tooltips[&control.path];
    assert_eq!(tooltip.title, "Synthetic Signals");
    assert_eq!(instance.control(root, control).unwrap().enabled, Some(false));
}

struct MissingShield<'a>(&'a gui::Fixture);

impl Resources for MissingShield<'_> {
    fn sprite(&self, name: &[u8]) -> Option<SpriteInfo> {
        self.0.sprite(name)
    }

    fn font(&self, name: &[u8]) -> Option<&Font> {
        self.0.font(name)
    }

    fn localisation(&self) -> &Localisation {
        self.0.localisation()
    }

    fn text_colors(&self) -> &native::fonts::TextColors {
        self.0.text_colors()
    }

    fn text_effect(&self) -> Result<Effect, String> {
        self.0.text_effect()
    }

    fn effect(&self, file: &str) -> Result<Effect, String> {
        self.0.effect(file)
    }

    fn gui_type(&self, name: &[u8]) -> Option<&Element> {
        self.0.gui_type(name)
    }

    fn shield_flag(&self, _: bool, _: i32) -> Result<Option<(Texture, [f32; 4])>, String> {
        Err("Synthetic flag data unavailable".to_owned())
    }

    fn text_flag(&self, tag: &str) -> Result<Option<Texture>, String> {
        self.0.text_flag(tag)
    }
}

#[test]
fn an_unavailable_target_flag_keeps_the_running_operation_and_its_named_placeholder() {
    let guis = agency_guis();
    let root = guis.template(b"synthetic_agency").unwrap();
    let mut fixture = gui::fixture();
    fixture.add_sprite("GFX_synthetic_tab", [65, 18], 1);
    fixture.add_sprite("GFX_synthetic_shield", [19, 14], 1);
    fixture.sprites.get_mut("GFX_synthetic_shield").unwrap().kind = SpriteKind::MaskedShield { mask: [12, 9], small: true };
    let resources = MissingShield(&fixture);
    let layout = layout_window(root, &resources, [320, 290], |_| {}).unwrap();
    let localisation = [("OPERATION_VIEW_PROGRESS".to_owned(), "Survey".to_owned())].into_iter().collect();
    let sources = Sources { guis: &guis, resources: &resources, localisation: &localisation };
    let mut view = agency_view();
    view.operations = Ok(vec![Operation {
        token: Some("survey".to_owned()),
        name: Ok("Barrow survey".to_owned()),
        sprite: Err("No operation art".to_owned()),
        target: Some("SUN".to_owned()),
        required: 2,
        duration_days: 17,
        preparing: false,
    }]);

    let mut plan = ComposedPlan::default();
    compose_operations(&mut plan, &layout, root, &sources, &view).unwrap();
    assert_eq!(plan.sub_windows.len(), 1);
    assert!(plan.text.iter().any(|entry| entry.label == "Operation" && entry.value.as_deref() == Some("Barrow survey")));
    let (_, rect, label) = plan.placeholders.iter().find(|(path, _, _)| path.ends_with("/btn_target_flag")).unwrap();
    assert_eq!(rect.map(|bounds| [bounds[2], bounds[3]]), Some([19.0, 14.0]));
    assert_eq!(label.title, "SUN");
    assert!(label.body.as_deref().is_some_and(|reason| reason.contains("flag atlas binding")));
    assert!(plan.warnings.iter().any(|warning| warning.contains("flag atlas binding") && warning.contains("SUN")));
    for (label, value) in [("Target", "SUN"), ("Target flag for SUN", UNBOUND_TARGET_FLAG)] {
        let line = plan.text.iter().find(|entry| entry.label == label).unwrap();
        assert_eq!(line.value.as_deref(), Some(value));
        assert!(line.missing.is_none());
    }
    let details = plan.sub_windows[0].tooltip_overrides.values().next().unwrap().body.as_deref().unwrap();
    assert!(details.lines().any(|line| line == "Target: SUN"));
    assert!(details.lines().any(|line| line == format!("Target flag for SUN: {UNBOUND_TARGET_FLAG}")));

    let sources = Sources { resources: &fixture, ..sources };
    let mut plan = ComposedPlan::default();
    compose_operations(&mut plan, &layout, root, &sources, &view).unwrap();
    assert!(plan.placeholders.iter().any(|(path, _, tooltip)| path.ends_with("/btn_target_flag") && tooltip.title == "SUN"));
    assert!(plan.sub_windows[0].parts.iter().all(|part| !matches!(&part.kind, native_gui::PartKind::MaskedFlag { .. })));

    let parts = &plan.sub_windows[0].parts;
    for (name, expected) in [
        ("operatives_required_text", "2"),
        ("network_strength_text", "n/a"),
        ("operation_duration_text", "17"),
        ("btn_select_button", "Survey"),
    ] {
        let text = parts.iter().filter(|part| part.path.ends_with(name)).find_map(|part| match &part.kind {
            native_gui::PartKind::Text { glyphs, .. } => Some(glyphs.iter().map(|glyph| glyph.ch).collect::<String>()),
            _ => None,
        });
        assert_eq!(text.as_deref(), Some(expected), "{name}");
    }
    assert!(parts.iter().all(|part| !part.path.contains("container_tokens")));

    let template = String::from_utf8_lossy(AGENCY_GUI);
    let missing = template.replace("operation_duration_text", "unrelated_text").replace("container_tokens", "unrelated_tokens");
    let ambiguous = template.replace("name = \"container_network_strength\"", "name = \"container_network_strength\" instantTextboxType = { name = \"operation_duration_text\" font = \"Main_14\" text = \"6\" maxWidth = 41 maxHeight = 24 }");
    for (template, detail) in [(missing, "missing"), (ambiguous, "ambiguous")] {
        let guis = super::super::load_native_guis_from_bytes("agency-withheld-field.gui", template.as_bytes()).unwrap();
        let root = guis.template(b"synthetic_agency").unwrap();
        let layout = layout_window(root, &fixture, [320, 290], |_| {}).unwrap();
        let sources = Sources { guis: &guis, ..sources };
        let mut plan = ComposedPlan::default();
        compose_operations(&mut plan, &layout, root, &sources, &view).unwrap();
        assert!(plan.text.iter().any(|entry| entry.label == "Duration (days)" && entry.value.as_deref() == Some("17")));
        let explains = |reason: &str| reason.contains("operation_duration_text") && reason.contains(detail);
        assert!(plan.text.iter().any(|entry| entry.label == "Duration display" && entry.value.as_deref().is_some_and(explains)));
        assert!(plan.warnings.iter().any(|warning| explains(warning)));
        assert!(plan.warnings.iter().all(|warning| !warning.contains("container_tokens")));
        assert!(plan.sub_windows[0].tooltip_overrides.values().any(|tooltip| tooltip.body.as_deref().is_some_and(explains)));
        assert!(plan.sub_windows[0].parts.iter().all(|part| !part.path.ends_with("operation_duration_text")));
    }
}

fn upgrades(branches: &[(&str, &[(&str, usize)])]) -> AgencyUpgrades {
    let mut upgrades = AgencyUpgrades::default();
    for (branch, tokens) in branches {
        upgrades.branches.push((branch.to_string(), tokens.iter().map(|(token, _)| token.to_string()).collect()));
        for (token, levels) in *tokens {
            upgrades.branch_definitions.insert(
                (branch.to_string(), token.to_string()),
                AgencyUpgrade { token: token.to_string(), name: token.to_string(), branch: branch.to_string(), sprite: String::new(), levels: *levels },
            );
        }
    }
    upgrades
}

fn item(ordinal: u32, branch: Option<&str>, token: Option<&str>, level: i32, named: bool) -> UpgradeItem {
    UpgradeItem {
        ordinal,
        branch: branch.map(str::to_owned),
        token: token.map(str::to_owned),
        level,
        name: named.then(|| token.unwrap_or_default().to_owned()),
        missing: (!named).then(|| "Acquired upgrade identity refused: unknown".to_owned()),
    }
}

fn held(items: Vec<UpgradeItem>, unavailable: Option<&str>) -> Upgrades {
    Upgrades { unavailable: unavailable.map(str::to_owned), nonnegative_level_sum: None, items }
}

fn laid(content: &AgencyUpgrades, recorded: Result<&Upgrades, &str>) -> Vec<Branch> {
    branches(content, recorded, |branch| Ok(format!("title {branch}")), |_, token| Ok(format!("name {token}")))
}

#[test]
fn a_pip_takes_the_frame_run_for_its_level_count_from_the_installed_marker_and_none_past_its_frames() {
    assert_eq!(pip_frame(14, 1, 0), Some(1));
    assert_eq!(pip_frame(14, 1, 1), Some(2));
    assert_eq!(pip_frame(14, 2, 0), Some(3));
    assert_eq!(pip_frame(14, 2, 2), Some(5));
    assert_eq!(pip_frame(14, 3, 1), Some(7));
    assert_eq!(pip_frame(14, 4, 4), Some(14));
    assert_eq!(pip_frame(14, 4, 9), Some(14), "a level above the count fills the marker");
    assert_eq!(pip_frame(14, 2, -1), Some(3));
    assert_eq!(pip_frame(14, 5, 1), None, "the installed marker has no run for five levels");
    assert_eq!(pip_frame(20, 5, 1), Some(16), "a mod's longer marker draws five levels");
    assert_eq!(pip_frame(14, 0, 0), None);
}

#[test]
fn branches_follow_the_content_order_and_absence_is_zero_only_when_every_acquired_upgrade_is_identified() {
    let content = upgrades(&[("branch_a", &[("one", 1), ("two", 2)]), ("branch_b", &[("three", 4)])]);
    let recorded = held(vec![item(0, Some("BRANCH_A"), Some("Two"), 2, true), item(1, Some("branch_b"), Some("three"), 3, true)], None);
    let shown = laid(&content, Ok(&recorded));
    assert_eq!(shown.iter().map(|branch| (branch.token.as_str(), branch.name.clone())).collect::<Vec<_>>(), [
        ("branch_a", Ok("title branch_a".to_owned())),
        ("branch_b", Ok("title branch_b".to_owned())),
    ]);
    assert_eq!(shown[0].pips[0], Pip { token: "one".into(), name: Ok("name one".into()), levels: 1, level: Ok(0) });
    assert_eq!(shown[0].pips[1].level, Ok(2));
    assert_eq!(shown[1].pips[0].level, Ok(3));

    let unidentified = held(vec![item(0, Some("branch_a"), Some("two"), 1, true), item(1, None, Some("x"), 1, false)], None);
    let shown = laid(&content, Ok(&unidentified));
    assert_eq!(shown[0].pips[1].level, Ok(1), "a witnessed level is kept");
    assert!(matches!(&shown[0].pips[0].level, Err(why) if why.contains("could not be identified")));

    let refused = held(vec![item(0, Some("branch_a"), Some("one"), 1, false)], Some("Acquired agency levels contain unauthenticated identities"));
    let shown = laid(&content, Ok(&refused));
    assert!(matches!(&shown[0].pips[0].level, Err(why) if why.contains("unauthenticated")), "a refused identity is not taken as this upgrade");
    assert!(shown.iter().flat_map(|branch| &branch.pips).all(|pip| pip.level.is_err()), "nor is any absence a zero then");

    let missing = laid(&content, Err("table truncated"));
    assert!(missing.iter().flat_map(|branch| &branch.pips).all(|pip| matches!(&pip.level, Err(why) if why.contains("table truncated"))));
}
