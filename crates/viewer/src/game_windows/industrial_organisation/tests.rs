use super::*;
use content_reader_layers::ResolvedLayers;
use std::collections::BTreeMap;

fn synthetic_layers(entries: &[(&str, &[u8])]) -> ResolvedLayers {
    let root = std::env::temp_dir().join(format!("battleplan-mio-test-{}", uuid::Uuid::new_v4()));
    for (path, bytes) in entries {
        let file = root.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, bytes).unwrap();
    }
    std::fs::create_dir_all(&root).unwrap();
    ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap()
}

#[test]
fn funds_progress_uses_defines_formula_and_withholds_when_defines_missing() {
    let defines = Ok(FundsDefines {
        base: 700.0,
        level_factor: 100.0,
        level_pow: 1.8,
    });
    let progress = compute_funds_progress(35_000_000, 0, 0, &defines);
    assert_eq!(progress.threshold, Some(700.0));
    assert_eq!(progress.percent, Some(50));
    assert!(progress.threshold_reason.is_none());

    let missing = compute_funds_progress(
        35_000_000,
        2,
        0,
        &Err("missing NDefines.NIndustrialOrganisation.FUNDS_FOR_SIZE_UP".to_owned()),
    );
    assert_eq!(missing.threshold, None);
    assert_eq!(missing.percent, None);
    assert!(
        missing
            .threshold_reason
            .as_deref()
            .is_some_and(|r| r.contains("Not recorded"))
    );
}

const DETAIL_WINDOW_GUI: &[u8] = br#"
        guiTypes = {
            containerWindowType = {
                name = "industrial_organisation_detail_window"
                size = { width = 1280 height = 720 }
                instantTextboxType = { name = "industrial_org_name" position = { x = 0 y = 0 } font = "hoi_36header" text = "" maxWidth = 300 maxHeight = 20 }
                containerWindowType = {
                    name = "tabs"
                    buttonType = { name = "traits_tab" position = { x = 0 y = 0 } quadTextureSprite = "GFX_tab" }
                    buttonType = { name = "history_tab" position = { x = 100 y = 0 } quadTextureSprite = "GFX_tab" }
                }
                containerWindowType = {
                    name = "top_left_window"
                    iconType = { name = "icon" position = { x = 0 y = 0 } spriteType = "GFX_idea_unknown" }
                    containerWindowType = {
                        name = "size_icon_window"
                        iconType = { name = "size_icon" position = { x = 0 y = 0 } spriteType = "GFX_size" }
                        instantTextboxType = { name = "size" position = { x = 0 y = 0 } font = "hoi_20bs" text = "0" maxWidth = 30 maxHeight = 18 }
                    }
                    instantTextboxType = { name = "points" position = { x = 0 y = 0 } font = "hoi_18mbs" text = "" maxWidth = 150 maxHeight = 20 }
                }
                containerWindowType = {
                    name = "aggregated_bonuses_window"
                    buttonType = { name = "open_queue_button" position = { x = 0 y = 0 } quadTextureSprite = "GFX_btn" }
                }
                containerWindowType = {
                    name = "open_policies_window"
                    buttonType = { name = "open_policies" position = { x = 0 y = 0 } spriteType = "GFX_btn" }
                    instantTextboxType = { name = "open_policies_text" position = { x = 0 y = 0 } font = "hoi_20b" text = "" maxWidth = 200 maxHeight = 20 }
                    instantTextboxType = { name = "click_to_open_policies_text" position = { x = 0 y = 20 } font = "hoi_20b" text = "" maxWidth = 200 maxHeight = 20 }
                    iconType = { name = "attached_policy_icon" position = { x = 0 y = 0 } spriteType = "GFX_idea_unknown" }
                    instantTextboxType = { name = "attached_policy_name" position = { x = 0 y = 0 } font = "hoi_20bs" text = "" maxWidth = 160 maxHeight = 20 }
                }
                containerWindowType = { name = "history_window" }
                buttonType = { name = "toggle_auto_designs" position = { x = 0 y = 0 } quadTextureSprite = "GFX_chk" }
                buttonType = { name = "close_button" position = { x = 0 y = 0 } quadTextureSprite = "GFX_close" }
            }
        }
    "#;

#[test]
fn missing_details_sets_na_and_not_recorded_tooltips() {
    content_reader_script::test_game::enter();
    let gui = DETAIL_WINDOW_GUI;
    let guis = super::super::load_native_guis_from_bytes("test.gui", gui).unwrap();
    let root = guis.template(DETAIL_WINDOW_NAME.as_bytes()).unwrap();
    let localisation = Localisation::default();
    let mut instance =
        Instance::construct(root, DEFAULT_VIEWPORT.map(|v| v as i32), &DummyResources).unwrap();
    instance.gui_show(root, root).unwrap();

    let prepared = PreparedMio {
        data: OrganisationViewData {
            organisation_id: 7,
            request_hour: 100,
            read_hour: 100,
            owner_tag: Some("SYN".to_owned()),
            token: "SYN_synthetic_works_organisation".to_owned(),
            custom_name: "Synthetic Works".to_owned(),
            recorded_icon: String::new(),
            details: RecordedDetails::Unavailable("table omitted in older format".to_owned()),
            policy_costs: Ok(BTreeMap::new()),
            history: None,
            history_reason: Some("history omitted".to_owned()),
            funds_defines: Err("no defines".to_owned()),
            warnings: Vec::new(),
        },
        sub_view: SubView::Traits,
        catalog: Err("not read in this test".to_owned()),
        org_db: Err("not read in this test".to_owned()),
        emblem_sprite: UNKNOWN_ICON_SPRITE.to_owned(),
        tree_background: None,
        attached_policy_sprite: None,
    };

    let mut tooltips = BTreeMap::new();
    configure_detail_main(root, &localisation, &mut instance, &mut tooltips, &prepared);

    let size_el = find_direct(root, "top_left_window/size_icon_window/size").unwrap();
    assert_eq!(
        instance.control(root, size_el).unwrap().text.as_deref(),
        Some("n/a")
    );
    let points_el = find_direct(root, "top_left_window/points").unwrap();
    assert_eq!(
        instance.control(root, points_el).unwrap().text.as_deref(),
        Some("INDUSTRIAL_ORG_DETAIL_TRAIT_POINTS n/a")
    );
    let policy_text_el =
        find_direct(root, "open_policies_window/click_to_open_policies_text").unwrap();
    assert_eq!(
        instance
            .control(root, policy_text_el)
            .unwrap()
            .text
            .as_deref(),
        Some("n/a")
    );
    let size_icon = find_direct(root, "top_left_window/size_icon_window/size_icon").unwrap();
    let tip = tooltips.get(&size_icon.path).expect("missing details tooltip");
    assert_eq!(tip.title, "Not recorded");
    assert!(
        tip.body
            .as_deref()
            .is_some_and(|b| b.contains("table omitted in older format"))
    );
}

#[test]
fn missing_mio_window_template_is_refused_by_name() {
    content_reader_script::test_game::enter();
    let layers = synthetic_layers(&[
        ("interface/empty.gui", b"guiTypes = {}"),
        ("localisation/english/empty_l_english.yml", b"l_english:\n"),
    ]);
    let reason = refusal(&layers, "synthetic_works_organisation", SubView::History);
    assert!(reason.contains(DETAIL_WINDOW_NAME), "{reason}");
}

fn cache_key(layers: &ResolvedLayers, window: &str) -> WindowCacheKey {
    WindowCacheKey {
        source_id: "src".to_owned(),
        revision: "rev".to_owned(),
        game_dir: std::path::PathBuf::from("synthetic"),
        user_dir: std::path::PathBuf::from("synthetic"),
        visual_revision: String::new(),
        language: "english".to_owned(),
        force_pow2_textures: false,
        art_stack: layers.art_stack_key(),
        window: window.to_owned(),
    }
}

fn unread_organisation(token: &str) -> OrganisationViewData {
    OrganisationViewData {
        organisation_id: 1,
        request_hour: 0,
        read_hour: 0,
        owner_tag: None,
        token: token.to_owned(),
        custom_name: String::new(),
        recorded_icon: String::new(),
        details: RecordedDetails::Unavailable("not recorded".to_owned()),
        policy_costs: Ok(BTreeMap::new()),
        history: None,
        history_reason: None,
        funds_defines: Err("no defines".to_owned()),
        warnings: Vec::new(),
    }
}

fn refusal(layers: &ResolvedLayers, token: &str, sub_view: SubView) -> String {
    let service = Service::default();
    let cancel = AtomicBool::new(false);
    let result = render_data(
        &service,
        layers,
        cache_key(layers, sub_view.window_name()),
        unread_organisation(token),
        sub_view,
        1.0,
        &BTreeMap::new(),
        None,
        &cancel,
    );
    match result {
        Err(GameWindowError::Refused(reason)) => reason,
        Err(other) => panic!("expected a refusal by name, got {other:?}"),
        Ok(_) => panic!("expected a refusal by name, got a rendered window"),
    }
}

const SYNTHETIC_ORGANISATION_SCRIPT: &[u8] = br#"
    synthetic_works_organisation = {
        trait = {
            token = synthetic_trait
            name = synthetic_trait_name
            position = { x = 0 y = 0 }
            equipment_bonus = { soft_attack = 0.1 }
        }
    }
"#;

#[test]
fn a_withheld_stat_table_refuses_the_traits_and_policies_by_name_instead_of_panicking() {
    let layers = synthetic_layers(&[(
        "common/military_industrial_organization/organizations/00_synthetic.txt",
        SYNTHETIC_ORGANISATION_SCRIPT,
    )]);
    content_reader_script::test_game::with(|game| {
        game.auxiliary(|tables| {
            tables.stat_metadata = Err(engine_executable::availability::Withheld::new(
                "equipment stat metadata",
                "not decoded from this build",
            ));
        })
    });
    for sub_view in [SubView::Traits, SubView::Policy] {
        let reason = refusal(&layers, "synthetic_works_organisation", sub_view);
        assert!(reason.contains("synthetic_works_organisation"), "{reason}");
        assert!(
            reason.contains("equipment stat metadata withheld: not decoded from this build"),
            "{reason}"
        );
    }
}

#[test]
fn an_organisation_missing_from_the_recorded_scripts_refuses_its_trait_tree_by_name() {
    content_reader_script::test_game::enter();
    let layers = synthetic_layers(&[(
        "common/military_industrial_organization/organizations/00_synthetic.txt",
        SYNTHETIC_ORGANISATION_SCRIPT,
    )]);
    let reason = refusal(&layers, "synthetic_absent_organisation", SubView::Traits);
    assert!(reason.contains("synthetic_absent_organisation"), "{reason}");
    assert!(reason.contains("is not defined in the recorded mod stack"), "{reason}");
}

#[test]
fn a_tab_whose_interface_lacks_a_required_list_refuses_by_name_instead_of_omitting_it() {
    content_reader_script::test_game::enter();
    let layers = synthetic_layers(&[
        ("interface/synthetic_mio.gui", DETAIL_WINDOW_GUI),
        ("localisation/english/empty_l_english.yml", b"l_english:\n"),
    ]);
    let reason = refusal(&layers, "synthetic_works_organisation", SubView::History);
    assert!(reason.contains("progressbar_widget"), "{reason}");
}

#[test]
fn mio_tree_reads_background_icon_and_header_texts_through_mod_layers() {
    let script = br#"
        base_mod_mio = {
            icon = GFX_mod_base_icon
            background = GFX_mod_base_bg
            tree_header_text = {
                text = mod_header_left
                x = 2
            }
            trait = {
                token = t_root
                name = t_root_name
                icon = GFX_t_root
                position = { x = 1 y = 0 }
            }
        }
        child_mod_mio = {
            include = base_mod_mio
            trait = {
                token = t_child
                name = t_child_name
                icon = GFX_t_child
                position = { x = 0 y = 1 }
                relative_position_id = t_root
                any_parent = { t_root }
            }
        }
    "#;
    let layers = synthetic_layers(&[(
        "common/military_industrial_organization/organizations/00_mod.txt",
        script,
    )]);
    let catalog = mio_traits::read(&layers).unwrap();
    let tree = catalog.tree("child_mod_mio").unwrap().expect("child tree");
    assert_eq!(tree.background, Some("GFX_mod_base_bg"));
    assert_eq!(tree.icon, Some("GFX_mod_base_icon"));
    assert_eq!(tree.tree_header_texts.len(), 1);
    assert_eq!(tree.tree_header_texts[0].text, "mod_header_left");
    assert_eq!(tree.tree_header_texts[0].x, 2);
    assert_eq!(
        catalog.position("child_mod_mio", "t_child").unwrap(),
        Some([1, 1])
    );
}

#[test]
fn unsupported_sub_view_is_refused_by_name() {
    let err = SubView::from_request(Some("unknown_tab"))
        .expect_err("unknown sub_view must refuse");
    assert!(matches!(err, GameWindowError::Refused(msg) if msg.contains("unknown_tab")));
}

struct DummyResources;

impl Resources for DummyResources {
    fn physical_viewport(&self) -> Option<[i32; 2]> {
        None
    }
    fn sprite(&self, _: &[u8]) -> Option<viewer_native_gui::native_gui::SpriteInfo> {
        None
    }
    fn sprite_info(&self, _: &[u8]) -> Option<std::borrow::Cow<'_, viewer_native_gui::native_gui::SpriteInfo>> {
        None
    }
    fn portrait_texture(&self, _: &[u8]) -> Option<viewer_native_gui::native_gui::Texture> {
        None
    }
    fn sprite_alpha(&self, _: &[u8]) -> Option<viewer_native_gui::native_gui::AlphaMask> {
        None
    }
    fn unloaded_sprite(&self, _: &[u8]) -> bool {
        false
    }
    fn font(&self, _: &[u8]) -> Option<&viewer_native_gui::native_text::Font> {
        None
    }
    fn localisation(&self) -> &Localisation {
        static EMPTY: std::sync::OnceLock<Localisation> = std::sync::OnceLock::new();
        EMPTY.get_or_init(Localisation::default)
    }
    fn text_colors(&self) -> &native::fonts::TextColors {
        static EMPTY: std::sync::OnceLock<native::fonts::TextColors> = std::sync::OnceLock::new();
        EMPTY.get_or_init(native::fonts::TextColors::default)
    }
    fn text_effect(&self) -> Result<viewer_native_gui::native_effects::Effect, String> {
        Err("unused".to_owned())
    }
    fn effect(&self, _: &str) -> Result<viewer_native_gui::native_effects::Effect, String> {
        Err("unused".to_owned())
    }
    fn shield_flag(&self, _: bool, _: i32) -> Result<Option<(viewer_native_gui::native_gui::Texture, [f32; 4])>, String> {
        Err("unused".to_owned())
    }
    fn text_flag(&self, _: &str) -> Result<Option<viewer_native_gui::native_gui::Texture>, String> {
        Err("unused".to_owned())
    }
    fn resolved_text_flag(&self, _: &[u8]) -> Result<Option<viewer_native_gui::native_gui::Texture>, String> {
        Err("unused".to_owned())
    }
    fn resolved_text_flag_key(&self, _: &[char]) -> Option<usize> {
        None
    }
    fn gui_type(&self, _: &[u8]) -> Option<&Element> {
        None
    }
    fn session_text_sprite(&self, _: &[u8]) -> bool {
        false
    }
    fn texture_file(&self, _: &[u8]) -> Option<viewer_native_gui::native_gui::TextureFile> {
        None
    }
    fn sprite_animations(&self, _: &[u8]) -> Option<Vec<viewer_native_gui::native_gui::AnimationRecord>> {
        None
    }
}
