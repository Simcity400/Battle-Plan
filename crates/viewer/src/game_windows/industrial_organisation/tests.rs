use super::*;
use content_reader_layers::{Layer, LayerKind, ResolvedLayers};
use std::collections::BTreeMap;

fn synthetic_layers(entries: &[(&str, &[u8])]) -> ResolvedLayers {
    let files: BTreeMap<String, Vec<u8>> = entries
        .iter()
        .map(|(path, bytes)| (path.to_ascii_lowercase(), bytes.to_vec()))
        .collect();
    ResolvedLayers::from_layers(vec![Layer {
        name: "synthetic".to_owned(),
        kind: LayerKind::Mod,
        root: std::path::PathBuf::from("synthetic"),
        files: std::sync::Arc::new(files),
    }])
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

#[test]
fn missing_details_sets_na_and_not_recorded_tooltips() {
    let gui = br#"
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
            owner_tag: Some("USA".to_owned()),
            token: "USA_chrysler_organization".to_owned(),
            custom_name: "Chrysler".to_owned(),
            recorded_icon: String::new(),
            details: RecordedDetails::Unavailable("table omitted in older format".to_owned()),
            policy_costs: BTreeMap::new(),
            history: None,
            history_reason: Some("history omitted".to_owned()),
            funds_defines: Err("no defines".to_owned()),
            warnings: Vec::new(),
        },
        sub_view: SubView::Traits,
        catalog: None,
        org_db: None,
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
        Some("Trait points: n/a")
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
    let layers = synthetic_layers(&[
        ("interface/empty.gui", b"guiTypes = {}"),
        ("localisation/english/empty_l_english.yml", b"l_english:\n"),
    ]);
    let service = Service::default();
    let cancel = AtomicBool::new(false);
    let key = WindowCacheKey {
        source_id: "src".to_owned(),
        revision: "rev".to_owned(),
        game_dir: std::path::PathBuf::from("synthetic"),
        user_dir: std::path::PathBuf::from("synthetic"),
        visual_revision: String::new(),
        language: "english".to_owned(),
        force_pow2_textures: false,
        window: DETAIL_WINDOW_NAME.to_owned(),
    };
    let data = OrganisationViewData {
        organisation_id: 1,
        request_hour: 0,
        read_hour: 0,
        owner_tag: None,
        token: "mod_org".to_owned(),
        custom_name: "Mod Org".to_owned(),
        recorded_icon: String::new(),
        details: RecordedDetails::Unavailable("not recorded".to_owned()),
        policy_costs: BTreeMap::new(),
        history: None,
        history_reason: None,
        funds_defines: Err("no defines".to_owned()),
        warnings: Vec::new(),
    };
    let err = render_data(
        &service,
        &layers,
        key,
        data,
        SubView::Traits,
        1.0,
        None,
        &cancel,
    )
    .expect_err("missing template must refuse");
    assert!(matches!(err, GameWindowError::Refused(msg) if msg.contains(DETAIL_WINDOW_NAME)));
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
    let err = SubView::from_request("industrialOrganisation", Some("unknown_tab"))
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
