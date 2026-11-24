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
            token: Ok("SYN_synthetic_works_organisation".to_owned()),
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
    let frame = empty_frame();
    let scripted = unread_scripted_text();
    let failures = RefCell::new(Vec::new());
    let loc = Loc::new(&localisation, &frame, &scripted, &prepared.data, &failures);
    configure_detail_main(root, &loc, &mut instance, &mut tooltips, &prepared);

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

fn empty_frame() -> DecodedFrame {
    DecodedFrame {
        branch: 1,
        hour: 0,
        metrics: replay_frame::frame::Metrics::default(),
        tables: std::array::from_fn(|_| {
            replay_frame::frame::TableView::Unavailable {
                reason: "not in this test".into(),
            }
            .into()
        }),
    }
}

fn unread_scripted_text() -> Result<Arc<ScriptedContent>, String> {
    Err("not read in this test".to_owned())
}

fn spelled_tokens() -> Vec<LexicalTokenRow> {
    [(10, "synthetic_policy"), (20, "synthetic_first"), (30, "synthetic_third")]
        .into_iter()
        .map(|(token, literal)| LexicalTokenRow {
            token,
            literal: literal.as_bytes().to_vec(),
        })
        .collect()
}

#[test]
fn unspelled_tokens_keep_their_identity_and_queue_position() {
    let rows = spelled_tokens();
    let lexical: Result<&[LexicalTokenRow], String> = Ok(&rows);
    let mut row = IndustrialOrganisationDetailsRow {
        unlocked: vec![20, 99],
        queued: vec![20, 77, 30],
        allowed_policies: vec![10, 88],
        ..Default::default()
    };
    row.state.policy = 55;
    let details = extract_details(&row, &lexical);
    assert!(matches!(details.policy, Some(Spelled::Unspelled { token: 55, .. })));
    assert_eq!(details.queued.len(), 3);
    assert_eq!(details.queued[0], Spelled::Text("synthetic_first".to_owned()));
    assert!(matches!(details.queued[1], Spelled::Unspelled { token: 77, .. }));
    assert_eq!(details.queued[2], Spelled::Text("synthetic_third".to_owned()));
    assert!(matches!(details.allowed_policies[1], Spelled::Unspelled { token: 88, .. }));

    row.state.policy = UNSELECTED_POLICY;
    assert_eq!(extract_details(&row, &lexical).policy, None);

    let unread: Result<&[LexicalTokenRow], String> = Err("truncated".to_owned());
    let details = extract_details(&row, &unread);
    assert!(
        matches!(&details.queued[0], Spelled::Unspelled { token: 20, reason } if reason.contains("truncated"))
    );
}

#[test]
fn a_trait_is_locked_only_when_every_unlocked_and_queued_token_is_named() {
    let rows = spelled_tokens();
    let lexical: Result<&[LexicalTokenRow], String> = Ok(&rows);
    let details = |unlocked: Vec<u32>, queued: Vec<u32>| {
        let row = IndustrialOrganisationDetailsRow {
            unlocked,
            queued,
            ..Default::default()
        };
        RecordedDetails::Available(Box::new(extract_details(&row, &lexical)))
    };
    assert_eq!(trait_status(&details(vec![20], vec![30]), "synthetic_first"), TraitStatus::Unlocked);
    assert_eq!(trait_status(&details(vec![20], vec![30]), "synthetic_third"), TraitStatus::Queued(1));
    assert_eq!(trait_status(&details(vec![20], vec![30]), "synthetic_other"), TraitStatus::Locked);
    assert!(matches!(
        trait_status(&details(vec![20, 99], vec![]), "synthetic_other"),
        TraitStatus::Unknown(reason) if reason.contains("99")
    ));
    assert!(matches!(
        trait_status(&details(vec![], vec![77, 30]), "synthetic_other"),
        TraitStatus::Unknown(reason) if reason.contains("77")
    ));
    assert_eq!(trait_status(&details(vec![20, 99], vec![]), "synthetic_first"), TraitStatus::Unlocked);
    assert!(matches!(
        trait_status(&RecordedDetails::Unavailable("not read".to_owned()), "synthetic_first"),
        TraitStatus::Unknown(reason) if reason.contains("not read")
    ));
}

#[test]
fn history_lists_follow_each_entrys_recorded_design_date() {
    let entry = |variant_id, units, design| HistoryEntry {
        variant_id,
        name: Ok(format!("design {variant_id}")),
        archetype: String::new(),
        units,
        design,
    };
    let entries = vec![
        entry(1, 0, DesignDate::Designed(100)),
        entry(2, 40, DesignDate::Absent),
        entry(3, 5, DesignDate::Designed(200)),
    ];
    let [designed, produced] = history_lists(&entries);
    assert_eq!(designed, HistoryList::Entries(vec![&entries[0], &entries[2]]));
    assert_eq!(produced, HistoryList::Entries(vec![&entries[1]]));

    let older = vec![
        entry(1, 0, DesignDate::NotRecorded),
        entry(2, 40, DesignDate::NotRecorded),
    ];
    let [designed, produced] = history_lists(&older);
    assert!(matches!(designed, HistoryList::Missing(reason) if reason.contains("design dates")));
    assert!(matches!(produced, HistoryList::Missing(_)));
}

#[test]
fn history_reads_only_its_organisation_and_names_a_missing_design() {
    let rows: Vec<IndustrialOrganisationHistoryRow> = [(1, 10), (2, 20), (2, 21), (3, 30)]
        .into_iter()
        .map(|(organisation, variant)| IndustrialOrganisationHistoryRow {
            organisation,
            variant,
            units: 1,
            design_hour: Recorded::Value(None),
        })
        .collect();
    let variants: Result<&[VariantDefinitionRow], String> = Ok(&[]);
    let lexical: Result<&[LexicalTokenRow], String> = Ok(&[]);
    let history = extract_history(&rows, 2, &variants, &lexical);
    assert_eq!(
        history.iter().map(|entry| entry.variant_id).collect::<Vec<_>>(),
        vec![20, 21]
    );
    assert!(matches!(&history[0].name, Err(reason) if reason.contains("VariantDefinitions")));
}

#[test]
fn text_the_shared_resolver_cannot_expand_shows_its_key_and_says_why() {
    let localisation: Localisation = [(
        "SYNTHETIC_SCOPED".to_owned(),
        "Owner [From.GetTag]".to_owned(),
    )]
    .into_iter()
    .collect();
    let frame = empty_frame();
    let scripted = unread_scripted_text();
    let failures = RefCell::new(Vec::new());
    let data = unread_organisation("synthetic_works_organisation");
    let loc = Loc::new(&localisation, &frame, &scripted, &data, &failures);
    assert_eq!(loc.get("SYNTHETIC_SCOPED"), "SYNTHETIC_SCOPED");
    assert_eq!(loc.get("SYNTHETIC_ABSENT"), "SYNTHETIC_ABSENT");
    let failures = failures.borrow();
    assert!(
        failures
            .iter()
            .any(|line| line.contains("SYNTHETIC_SCOPED") && line.contains("not read in this test")),
        "{failures:?}"
    );
    assert!(failures.iter().any(|line| line.contains("SYNTHETIC_ABSENT")), "{failures:?}");
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
        token: Ok(token.to_owned()),
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
        (&empty_frame(), &unread_scripted_text()),
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
