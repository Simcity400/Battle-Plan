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
        emblem: Err(("organisation emblem".to_owned(), "not recorded".to_owned())),
        tree_background: None,
        history_icons: HashMap::new(),
        history_art_label: None,
        scroll: BTreeMap::new(),
    };

    let mut tooltips = BTreeMap::new();
    let frame = empty_frame();
    let scripted = unread_scripted_text();
    let failures = RefCell::new(Vec::new());
    let loc = Loc::new(&localisation, &frame, &scripted, &prepared.data, &failures);
    configure_detail_main(root, &DummyResources, &loc, &mut instance, &mut tooltips, &prepared);

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

#[test]
fn bonuses_for_different_equipment_targets_are_never_summed_together() {
    content_reader_script::test_game::enter();
    let layers = synthetic_layers(&[(
        "common/military_industrial_organization/organizations/00_synthetic.txt",
        br#"
            synthetic_works_organisation = {
                trait = {
                    token = synthetic_everything
                    position = { x = 0 y = 0 }
                    equipment_bonus = { soft_attack = 0.1 }
                }
                trait = {
                    token = synthetic_armour_only
                    position = { x = 1 y = 0 }
                    limit_to_equipment_type = { synthetic_armour }
                    equipment_bonus = { soft_attack = 0.1 }
                }
                trait = {
                    token = synthetic_also_everything
                    position = { x = 2 y = 0 }
                    equipment_bonus = { soft_attack = 0.05 }
                }
            }
        "#,
    )]);
    let catalog = mio_traits::read(&layers).unwrap();
    let tree = catalog.tree("synthetic_works_organisation").unwrap().unwrap();
    let row = IndustrialOrganisationDetailsRow::default();
    let lexical: Result<&[LexicalTokenRow], String> = Ok(&[]);
    let mut details = extract_details(&row, &lexical);
    details.unlocked = ["synthetic_everything", "synthetic_armour_only", "synthetic_also_everything"]
        .map(|token| Spelled::Text(token.to_owned()))
        .to_vec();
    let soft_attack = |value| Bonus { name: "soft_attack".to_owned(), value };
    let policy = Policy {
        token: b"synthetic_policy".to_vec(),
        name: Vec::new(),
        icon: Vec::new(),
        visible: Vec::new(),
        available: Vec::new(),
        groups: vec![
            (b"synthetic_group_a".to_vec(), vec![soft_attack(10_000)]),
            (b"synthetic_group_b".to_vec(), vec![soft_attack(10_000)]),
        ],
        organisation_modifiers: Default::default(),
    };
    let totals = aggregate_bonuses(&details, &tree, Some(&policy));
    assert_eq!(
        totals,
        vec![
            (BonusTarget::Organisation, vec![soft_attack(15_000)]),
            (BonusTarget::Equipment(vec![b"synthetic_armour".to_vec()]), vec![soft_attack(10_000)]),
            (BonusTarget::Group(b"synthetic_group_a".to_vec()), vec![soft_attack(10_000)]),
            (BonusTarget::Group(b"synthetic_group_b".to_vec()), vec![soft_attack(10_000)]),
        ]
    );
}

#[test]
fn policy_base_costs_come_from_the_owners_roster_never_the_holders() {
    use replay_data::rows::{
        CountryLinkRow, CountryRow, IndustrialOrganisationPoliciesRow, IndustrialOrganisationPolicy,
        TableRows,
    };
    use replay_frame::frame::TableView;
    const HOLDER: u32 = 4_999;
    const OWNER_ALIAS: u32 = 5_000;
    const OWNER: u32 = 6_000;
    let mut frame = empty_frame();
    let mut set = |id: TableId, rows: TableRows| *frame.tables[id.index()] = TableView::Complete(rows);
    set(
        TableId::Countries,
        TableRows::Countries(
            [(HOLDER, 3, "HLD"), (OWNER, 7, "OWN")]
                .into_iter()
                .map(|(id, canonical_slot, tag)| CountryRow {
                    id,
                    canonical_slot,
                    tag: tag.as_bytes().to_vec(),
                    ..Default::default()
                })
                .collect(),
        ),
    );
    set(
        TableId::CountryLinks,
        TableRows::CountryLinks(
            [(HOLDER, 3), (OWNER_ALIAS, 7), (OWNER, 7)]
                .into_iter()
                .map(|(id, canonical_slot)| CountryLinkRow { id, canonical_slot })
                .collect(),
        ),
    );
    let roster = |country, cost| IndustrialOrganisationPoliciesRow {
        country,
        policies: vec![IndustrialOrganisationPolicy {
            token: 10,
            cost,
            ..Default::default()
        }],
        ..Default::default()
    };
    set(
        TableId::IndustrialOrganisationPolicies,
        TableRows::IndustrialOrganisationPolicies(vec![roster(HOLDER, 99), roster(OWNER, 25)]),
    );
    let organisation = IndustrialOrganisationRow {
        organisation: 1,
        country: HOLDER,
        owner: OWNER_ALIAS,
        ..Default::default()
    };
    let rows = spelled_tokens();
    let costs = owner_policy_roster(&frame, &organisation, Ok(&rows)).unwrap();
    assert_eq!(costs["synthetic_policy"].cost, Some(25));

    let unowned = IndustrialOrganisationRow { owner: 1, ..organisation };
    let refused = owner_policy_roster(&frame, &unowned, Ok(&rows)).unwrap_err();
    assert!(refused.contains("roster is unavailable"), "{refused}");
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
        None,
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

#[test]
fn tab_table_projection_restricts_history_variants_and_policies_to_consuming_tabs() {
    let traits = SubView::Traits.required_tables();
    assert!(!traits.contains(&TableId::IndustrialOrganisationHistory));
    assert!(!traits.contains(&TableId::VariantDefinitions));
    assert!(!traits.contains(&TableId::IndustrialOrganisationPolicies));

    let policy = SubView::Policy.required_tables();
    assert!(policy.contains(&TableId::IndustrialOrganisationPolicies));
    assert!(!policy.contains(&TableId::IndustrialOrganisationHistory));
    assert!(!policy.contains(&TableId::VariantDefinitions));

    let history = SubView::History.required_tables();
    assert!(history.contains(&TableId::IndustrialOrganisationHistory));
    assert!(history.contains(&TableId::VariantDefinitions));
    assert!(!history.contains(&TableId::IndustrialOrganisationPolicies));
}

#[test]
fn missing_chosen_emblem_trait_policy_and_background_are_named_as_placeholders() {
    use viewer_native_gui::test_support::gui as gui_fixture;
    content_reader_script::test_game::enter();

    let mut data = unread_organisation("synthetic_works_organisation");
    data.custom_name = "Synthetic Works".to_owned();
    data.recorded_icon = "GFX_synthetic_missing_emblem".to_owned();
    let layers = synthetic_layers(&[
        (
            "common/military_industrial_organization/organizations/00_synthetic.txt",
            br#"
                synthetic_works_organisation = {
                    icon = GFX_synthetic_fallback_emblem
                    background = GFX_synthetic_missing_bg
                    trait = {
                        token = synthetic_trait_a
                        name = synthetic_trait_a_name
                        icon = GFX_synthetic_missing_trait
                        special_trait_background = yes
                        position = { x = 0 y = 0 }
                    }
                }
            "#,
        ),
        (
            "common/military_industrial_organization/policies/00_synthetic.txt",
            br#"
                synthetic_policy = {
                    icon = GFX_synthetic_missing_policy
                }
            "#,
        ),
    ]);
    let catalog = mio_traits::cached(&layers);
    let org_db = cached_organisation_database(&layers);
    let tree = organisation_tree(&catalog, &data.token).unwrap();
    assert_eq!(
        chosen_org_emblem(&data, Some(&tree)),
        Ok("GFX_synthetic_missing_emblem".to_owned())
    );
    let mut no_recorded = data.clone();
    no_recorded.recorded_icon.clear();
    assert_eq!(
        chosen_org_emblem(&no_recorded, Some(&tree)),
        Ok("GFX_synthetic_fallback_emblem".to_owned())
    );
    assert!(chosen_org_emblem(&no_recorded, None).is_err());

    let gui_bytes = br#"guiTypes = {
        scrollbarType = {
            name = "list_slider" size = { x = 10 y = 100 } minValue = 0 maxValue = 1
            slider = "slider" track = "track" leftButton = "down" rightButton = "up"
            buttonType = { name = "up" spriteType = "GFX_btn" }
            buttonType = { name = "down" spriteType = "GFX_btn" }
            guiButtonType = { name = "track" spriteType = "GFX_btn" }
            guiButtonType = { name = "slider" spriteType = "GFX_btn" }
        }
        containerWindowType = {
            name = "industrial_organisation_detail_window"
            size = { width = 1000 height = 600 }
            instantTextboxType = { name = "industrial_org_name" position = { x = 0 y = 0 } font = "Main_14" text = "" maxWidth = 200 maxHeight = 20 }
            containerWindowType = {
                name = "top_left_window"
                position = { x = 0 y = 20 }
                size = { width = 200 height = 100 }
                iconType = { name = "icon" position = { x = 10 y = 10 } spriteType = "GFX_default_icon" }
                containerWindowType = {
                    name = "size_icon_window"
                    iconType = { name = "size_icon" position = { x = 0 y = 0 } spriteType = "GFX_default_icon" }
                    instantTextboxType = { name = "size" position = { x = 0 y = 0 } font = "Main_14" text = "0" maxWidth = 30 maxHeight = 18 }
                }
                instantTextboxType = { name = "points" position = { x = 0 y = 0 } font = "Main_14" text = "" maxWidth = 100 maxHeight = 20 }
            }
            containerWindowType = {
                name = "open_policies_window"
                position = { x = 0 y = 120 }
                size = { width = 200 height = 80 }
                buttonType = { name = "open_policies" position = { x = 0 y = 0 } spriteType = "GFX_btn" }
                instantTextboxType = { name = "open_policies_text" position = { x = 0 y = 0 } font = "Main_14" text = "" maxWidth = 100 maxHeight = 20 }
                instantTextboxType = { name = "click_to_open_policies_text" position = { x = 0 y = 20 } font = "Main_14" text = "" maxWidth = 100 maxHeight = 20 }
                iconType = { name = "attached_policy_icon" position = { x = 10 y = 10 } spriteType = "GFX_default_icon" }
                instantTextboxType = { name = "attached_policy_name" position = { x = 60 y = 10 } font = "Main_14" text = "" maxWidth = 100 maxHeight = 20 }
            }
            containerWindowType = {
                name = "tree_scrollbar_window"
                position = { x = 220 y = 20 }
                size = { width = 400 height = 300 }
                containerWindowType = {
                    name = "tree_window"
                    position = { x = 0 y = 0 }
                    size = { width = 400 height = 300 }
                    gridBoxType = { name = "tree_list" position = { x = 0 y = 0 } slotsize = { width = 400 height = 300 } }
                }
            }
            containerWindowType = {
                name = "history_window"
                position = { x = 220 y = 20 }
                size = { width = 500 height = 300 }
                containerWindowType = {
                    name = "variants_window"
                    position = { x = 0 y = 0 }
                    size = { width = 240 height = 260 }
                    smoothListboxType = { name = "variant_history_list" position = { x = 0 y = 0 } size = { x = 220 y = 220 } scrollbartype = "list_slider" }
                }
                containerWindowType = {
                    name = "prod_lines_window"
                    position = { x = 250 y = 0 }
                    size = { width = 240 height = 260 }
                    smoothListboxType = { name = "prod_line_history_list" position = { x = 0 y = 0 } size = { x = 220 y = 220 } scrollbartype = "list_slider" }
                }
            }
        }
        containerWindowType = {
            name = "industrial_organisation_tree_window"
            size = { width = 400 height = 300 }
            background = { name = "Background" spriteType = "GFX_default_bg" }
            containerWindowType = {
                name = "scrollbar_window"
                position = { x = 10 y = 24 }
                size = { width = 380 height = 260 }
                background = { name = "Background" spriteType = "GFX_default_bg" }
                containerWindowType = { name = "traits_window" position = { x = 0 y = 0 } clipping = no }
            }
        }
        containerWindowType = {
            name = "industrial_organisation_trait_entry"
            size = { width = 88 height = 112 }
            containerWindowType = {
                name = "industrial_organisation_trait_icon"
                orientation = center
                buttonType = { name = "icon_frame" position = { x = -29 y = -40 } quadTextureSprite = "GFX_industrial_org_trait_button" }
                iconType = { name = "trait_icon" position = { x = -21 y = -32 } }
                containerWindowType = { name = "queue_number_window" instantTextboxType = { name = "queue_number" position = { x = 0 y = 0 } font = "Main_14" text = "" maxWidth = 20 maxHeight = 20 } }
            }
            instantTextboxType = { name = "ai_value" position = { x = 0 y = 0 } font = "Main_14" text = "" maxWidth = 20 maxHeight = 20 }
        }
        containerWindowType = {
            name = "industrial_organisation_history_item"
            size = { width = 220 height = 60 }
            iconType = { name = "equipment_icon" position = { x = 120 y = 30 } spriteType = "GFX_default_icon" centerposition = yes }
            instantTextboxType = { name = "equipment_name" position = { x = 4 y = 4 } font = "Main_14" text = "" maxWidth = 100 maxHeight = 14 }
            instantTextboxType = { name = "equipment_archetype_name" position = { x = 4 y = 20 } font = "Main_14" text = "" maxWidth = 100 maxHeight = 14 }
            instantTextboxType = { name = "flavor_fact" position = { x = 4 y = 38 } font = "Main_14" text = "" maxWidth = 100 maxHeight = 14 }
            buttonType = { name = "upgrade_variant_button" position = { x = 190 y = 10 } quadTextureSprite = "GFX_btn" }
        }
        containerWindowType = {
            name = "industrial_organisation_policy_window"
            size = { width = 400 height = 300 }
            containerWindowType = {
                name = "policy_list_window"
                position = { x = 0 y = 40 }
                size = { width = 400 height = 240 }
                smoothListboxType = { name = "policy_list" position = { x = 0 y = 0 } size = { x = 360 y = 220 } scrollbartype = "list_slider" }
            }
        }
        containerWindowType = {
            name = "policy_item"
            size = { width = 360 height = 60 }
            buttonType = { name = "background_button" spriteType = "GFX_btn" }
            iconType = { name = "icon" position = { x = 10 y = 10 } }
            instantTextboxType = { name = "name" position = { x = 60 y = 10 } font = "Main_14" text = "" maxWidth = 160 maxHeight = 20 }
            instantTextboxType = { name = "attach_cost" position = { x = 260 y = 10 } font = "Main_14" text = "" maxWidth = 60 maxHeight = 20 }
            instantTextboxType = { name = "ai_value" position = { x = 0 y = 0 } font = "Main_14" text = "" maxWidth = 20 maxHeight = 20 }
        }
    }"#;
    let guis = super::super::load_native_guis_from_bytes("test.gui", gui_bytes).unwrap();
    let doc = native::read_at(gui_bytes, DEFAULT_VIEWPORT.map(|v| v as i32));
    let mut resources = gui_fixture::fixture_with(doc);
    resources.add_sprite("GFX_btn", [20, 20], 1);
    resources.add_sprite("GFX_default_icon", [40, 40], 1);
    resources.add_sprite("GFX_default_bg", [400, 300], 1);
    resources.add_sprite("GFX_industrial_org_trait_button", [58, 58], 3);
    resources.add_sprite("GFX_synthetic_equip", [48, 32], 1);
    let localisation = Localisation::default();

    let mut details_row = IndustrialOrganisationDetailsRow::default();
    let lexical_rows = spelled_tokens();
    let lexical: Result<&[LexicalTokenRow], String> = Ok(&lexical_rows);
    details_row.state.policy = 10;
    details_row.unlocked = vec![20];
    details_row.allowed_policies = vec![10];
    let mut details = extract_details(&details_row, &lexical);
    details.unlocked = vec![Spelled::Text("synthetic_trait_a".to_owned())];
    data.details = RecordedDetails::Available(Box::new(details));

    let prepared = PreparedMio {
        data,
        sub_view: SubView::Traits,
        catalog,
        org_db,
        emblem: Ok("GFX_synthetic_missing_emblem".to_owned()),
        tree_background: Some("GFX_synthetic_missing_bg".to_owned()),
        history_icons: HashMap::from([
            (
                101,
                ResolvedHistoryIcon::Ready {
                    icon: Icon::Sprite("GFX_synthetic_equip".to_owned()),
                },
            ),
            (
                102,
                ResolvedHistoryIcon::Missing {
                    asset: "GFX_synthetic_missing_equip".to_owned(),
                    reason: "missing from stack".to_owned(),
                },
            ),
        ]),
        history_art_label: None,
        scroll: BTreeMap::new(),
    };

    let root = guis.template(DETAIL_WINDOW_NAME.as_bytes()).unwrap();
    let frame = empty_frame();
    let scripted = unread_scripted_text();
    let failures = RefCell::new(Vec::new());
    let loc = Loc::new(&localisation, &frame, &scripted, &prepared.data, &failures);
    let mut main_layout = layout_window(
        root,
        &resources,
        DEFAULT_VIEWPORT.map(|v| v as i32),
        |instance| {
            let mut tooltips = BTreeMap::new();
            configure_detail_main(root, &resources, &loc, instance, &mut tooltips, &prepared);
        },
    )
    .unwrap();
    let mut plan = ComposedPlan::default();
    compose_detail_chrome(&main_layout, root, &resources, &loc, &prepared, &mut plan);
    compose_trait_tree(&main_layout, root, &guis, &resources, &loc, &prepared, &mut plan).unwrap();
    let placed = plan.place_placeholders(&mut main_layout);

    assert!(
        plan.text.iter().any(|t| t.missing.as_deref().is_some_and(|m| m.contains("GFX_synthetic_missing_emblem"))),
        "missing chosen emblem must be named: {:?}",
        plan.text
    );
    assert!(
        plan.text.iter().any(|t| t.missing.as_deref().is_some_and(|m| m.contains("GFX_synthetic_missing_policy"))),
        "missing attached policy icon must be named: {:?}",
        plan.text
    );
    assert!(
        plan.text.iter().any(|t| t.missing.as_deref().is_some_and(|m| m.contains("GFX_synthetic_missing_bg"))),
        "missing chosen tree background must be named: {:?}",
        plan.text
    );
    assert!(
        plan.text.iter().any(|t| t.missing.as_deref().is_some_and(|m| m.contains("GFX_synthetic_missing_trait"))),
        "missing trait icon must be named: {:?}",
        plan.text
    );
    assert!(
        plan.text.iter().any(|t| t.missing.as_deref().is_some_and(|m| m.contains("GFX_country_spefific_org_trait_button"))),
        "missing special trait frame must be named instead of substituting default: {:?}",
        plan.text
    );
    assert!(placed.len() >= 4, "expected placed placeholders, got {placed:?}");
    let tree_sub = plan
        .sub_windows
        .iter()
        .find(|sub| sub.path_prefix == "mio_tree/")
        .unwrap();
    let bg_paths: BTreeSet<String> = guis
        .template(TREE_WINDOW.as_bytes())
        .unwrap()
        .container()
        .unwrap()
        .backgrounds
        .iter()
        .map(|bg| bg.path.clone())
        .collect();
    assert!(
        !tree_sub.parts.iter().any(|part| bg_paths.contains(&part.path)),
        "default tree background must be blanked when chosen background is missing"
    );

    let policy_root = guis.template(POLICY_WINDOW_NAME.as_bytes()).unwrap();
    let policy_layout =
        layout_window(policy_root, &resources, DEFAULT_VIEWPORT.map(|v| v as i32), |_| {}).unwrap();
    let mut policy_plan = ComposedPlan::default();
    compose_policy_list(
        &policy_layout,
        policy_root,
        &guis,
        &resources,
        &loc,
        &prepared,
        &mut policy_plan,
    )
    .unwrap();
    assert!(
        policy_plan.text.iter().any(|t| t.missing.as_deref().is_some_and(|m| m.contains("GFX_synthetic_missing_policy"))),
        "missing policy list icon must be named: {:?}",
        policy_plan.text
    );

    let item_tpl = guis.template(b"industrial_organisation_history_item").unwrap();
    let hist_records = HistoryRecordList::Entries(vec![
        HistoryRecord { variant_id: 101, units: 5, design: DesignDate::Absent },
        HistoryRecord { variant_id: 102, units: 9, design: DesignDate::Absent },
    ]);
    let variants_rows = [
        VariantDefinitionRow {
            id: 101,
            display_name: b"Synthetic Ready\0".to_vec(),
            ..Default::default()
        },
        VariantDefinitionRow {
            id: 102,
            display_name: b"Synthetic Missing\0".to_vec(),
            ..Default::default()
        },
    ];
    let variants: Result<&[VariantDefinitionRow], String> = Ok(&variants_rows);
    let mut hist_plan = ComposedPlan::default();
    let hist_main =
        layout_window(root, &resources, DEFAULT_VIEWPORT.map(|v| v as i32), |_| {}).unwrap();
    compose_history_list(
        &hist_main,
        root,
        item_tpl,
        &resources,
        &loc,
        HistoryListInput {
            path: PRODUCTION_LIST_PATH,
            prefix: "production",
            kind: HistoryKind::Produced,
            rows: &hist_records,
            variants: &variants,
            lexical: &lexical,
            icons: &prepared.history_icons,
        },
        &mut hist_plan,
    )
    .unwrap();
    let ready_row = hist_plan
        .sub_windows
        .iter()
        .find(|sub| sub.path_prefix == "production:0/")
        .unwrap();
    assert!(
        ready_row.parts.iter().any(|part| part.path.ends_with("equipment_icon")),
        "available history equipment icon must be drawn rather than hidden"
    );
    let missing_row = hist_plan
        .sub_windows
        .iter()
        .find(|sub| sub.path_prefix == "production:1/")
        .unwrap();
    assert!(
        !missing_row.parts.iter().any(|part| part.path.ends_with("equipment_icon")),
        "missing history equipment icon must be blanked"
    );
    assert!(
        hist_plan.text.iter().any(|t| t.missing.as_deref().is_some_and(|m| m.contains("GFX_synthetic_missing_equip"))),
        "missing history equipment icon must be named in text: {:?}",
        hist_plan.text
    );
}

#[test]
fn history_scroll_cost_stays_constant_as_world_history_grows() {
    use viewer_native_gui::test_support::gui as gui_fixture;
    content_reader_script::test_game::enter();

    let gui_bytes = br#"guiTypes = {
        scrollbarType = {
            name = "list_slider" size = { x = 10 y = 100 } minValue = 0 maxValue = 1
            slider = "slider" track = "track" leftButton = "down" rightButton = "up"
            buttonType = { name = "up" spriteType = "GFX_btn" }
            buttonType = { name = "down" spriteType = "GFX_btn" }
            guiButtonType = { name = "track" spriteType = "GFX_btn" }
            guiButtonType = { name = "slider" spriteType = "GFX_btn" }
        }
        containerWindowType = {
            name = "industrial_organisation_detail_window"
            size = { width = 500 height = 300 }
            containerWindowType = {
                name = "history_window"
                position = { x = 0 y = 0 }
                size = { width = 500 height = 300 }
                containerWindowType = {
                    name = "prod_lines_window"
                    position = { x = 0 y = 0 }
                    size = { width = 300 height = 240 }
                    smoothListboxType = { name = "prod_line_history_list" position = { x = 0 y = 0 } size = { x = 260 y = 150 } scrollbartype = "list_slider" }
                }
            }
        }
        containerWindowType = {
            name = "industrial_organisation_history_item"
            size = { width = 260 height = 50 }
            iconType = { name = "equipment_icon" position = { x = 120 y = 25 } spriteType = "GFX_btn" centerposition = yes }
            instantTextboxType = { name = "equipment_name" position = { x = 4 y = 4 } font = "Main_14" text = "" maxWidth = 100 maxHeight = 14 }
            instantTextboxType = { name = "equipment_archetype_name" position = { x = 4 y = 18 } font = "Main_14" text = "" maxWidth = 100 maxHeight = 14 }
            instantTextboxType = { name = "flavor_fact" position = { x = 4 y = 32 } font = "Main_14" text = "" maxWidth = 100 maxHeight = 14 }
            buttonType = { name = "upgrade_variant_button" position = { x = 220 y = 10 } quadTextureSprite = "GFX_btn" }
        }
    }"#;
    let guis = super::super::load_native_guis_from_bytes("test.gui", gui_bytes).unwrap();
    let doc = native::read_at(gui_bytes, DEFAULT_VIEWPORT.map(|v| v as i32));
    let mut resources = gui_fixture::fixture_with(doc);
    resources.add_sprite("GFX_btn", [20, 20], 1);
    let root = guis.template(DETAIL_WINDOW_NAME.as_bytes()).unwrap();
    let item_tpl = guis.template(b"industrial_organisation_history_item").unwrap();
    let main_layout =
        layout_window(root, &resources, DEFAULT_VIEWPORT.map(|v| v as i32), |_| {}).unwrap();
    let list_el = find_direct(root, PRODUCTION_LIST_PATH).unwrap();
    let localisation = Localisation::default();
    let frame = empty_frame();
    let scripted = unread_scripted_text();
    let failures = RefCell::new(Vec::new());
    let data = unread_organisation("synthetic_works_organisation");
    let loc = Loc::new(&localisation, &frame, &scripted, &data, &failures);

    let measure = |other_orgs: u64, scroll_y: f32| -> HistoryResolveCounts {
        let target_org = other_orgs / 2 + 1;
        let mut world_rows = Vec::with_capacity((other_orgs * 20 + 30) as usize);
        let mut variant_rows = Vec::with_capacity(60);
        for org in 1..=other_orgs {
            if org == target_org {
                for i in 0..30u64 {
                    let vid = 10_000 + i;
                    world_rows.push(IndustrialOrganisationHistoryRow {
                        organisation: target_org,
                        variant: vid,
                        units: (i + 1) as i32,
                        design_hour: Recorded::Value(None),
                    });
                    variant_rows.push(VariantDefinitionRow {
                        id: vid,
                        display_name: format!("Design {i}\0").into_bytes(),
                        ..Default::default()
                    });
                }
            } else {
                for i in 0..20u64 {
                    world_rows.push(IndustrialOrganisationHistoryRow {
                        organisation: org,
                        variant: org * 100 + i,
                        units: 1,
                        design_hour: Recorded::Value(None),
                    });
                }
            }
        }
        let records = extract_history_records(&world_rows, target_org);
        assert_eq!(records.len(), 30);
        let [_, produced] = split_history_records(&records);
        let variants: Result<&[VariantDefinitionRow], String> = Ok(&variant_rows);
        let lexical: Result<&[LexicalTokenRow], String> = Ok(&[]);
        let icons = HashMap::new();
        let mut plan = ComposedPlan::scrolled(BTreeMap::from([(list_el.path.clone(), scroll_y)]));
        compose_history_list(
            &main_layout,
            root,
            item_tpl,
            &resources,
            &loc,
            HistoryListInput {
                path: PRODUCTION_LIST_PATH,
                prefix: "production",
                kind: HistoryKind::Produced,
                rows: &produced,
                variants: &variants,
                lexical: &lexical,
                icons: &icons,
            },
            &mut plan,
        )
        .unwrap()
    };

    let small_top = measure(10, 0.0);
    let large_top = measure(1500, 0.0);
    let large_mid = measure(1500, 500.0);
    assert_eq!(small_top, large_top);
    assert_eq!(large_top, large_mid);
    assert!(
        large_mid.variant_lookups <= 4 && large_mid.variant_lookups >= 3,
        "only visible rows should resolve variants/names/layouts, got {large_mid:?}"
    );
}

#[test]
fn text_alternative_includes_visible_history_names_units_and_unread_facts() {
    use viewer_native_gui::test_support::gui as gui_fixture;
    content_reader_script::test_game::enter();

    let gui_bytes = br#"guiTypes = {
        scrollbarType = {
            name = "list_slider" size = { x = 10 y = 100 } minValue = 0 maxValue = 1
            slider = "slider" track = "track" leftButton = "down" rightButton = "up"
            buttonType = { name = "up" spriteType = "GFX_btn" }
            buttonType = { name = "down" spriteType = "GFX_btn" }
            guiButtonType = { name = "track" spriteType = "GFX_btn" }
            guiButtonType = { name = "slider" spriteType = "GFX_btn" }
        }
        containerWindowType = {
            name = "industrial_organisation_detail_window"
            size = { width = 600 height = 300 }
            containerWindowType = {
                name = "history_window"
                position = { x = 0 y = 0 }
                size = { width = 600 height = 300 }
                containerWindowType = {
                    name = "variants_window"
                    position = { x = 0 y = 0 }
                    size = { width = 260 height = 240 }
                    smoothListboxType = { name = "variant_history_list" position = { x = 0 y = 0 } size = { x = 240 y = 120 } scrollbartype = "list_slider" }
                }
                containerWindowType = {
                    name = "prod_lines_window"
                    position = { x = 280 y = 0 }
                    size = { width = 260 height = 240 }
                    smoothListboxType = { name = "prod_line_history_list" position = { x = 0 y = 0 } size = { x = 240 y = 120 } scrollbartype = "list_slider" }
                }
            }
        }
        containerWindowType = {
            name = "industrial_organisation_history_item"
            size = { width = 240 height = 60 }
            iconType = { name = "equipment_icon" position = { x = 120 y = 30 } spriteType = "GFX_btn" centerposition = yes }
            instantTextboxType = { name = "equipment_name" position = { x = 4 y = 4 } font = "Main_14" text = "" maxWidth = 100 maxHeight = 14 }
            instantTextboxType = { name = "equipment_archetype_name" position = { x = 4 y = 20 } font = "Main_14" text = "" maxWidth = 100 maxHeight = 14 }
            instantTextboxType = { name = "flavor_fact" position = { x = 4 y = 36 } font = "Main_14" text = "" maxWidth = 100 maxHeight = 14 }
            buttonType = { name = "upgrade_variant_button" position = { x = 200 y = 10 } quadTextureSprite = "GFX_btn" }
        }
    }"#;
    let guis = super::super::load_native_guis_from_bytes("test.gui", gui_bytes).unwrap();
    let doc = native::read_at(gui_bytes, DEFAULT_VIEWPORT.map(|v| v as i32));
    let mut resources = gui_fixture::fixture_with(doc);
    resources.add_sprite("GFX_btn", [20, 20], 1);
    let root = guis.template(DETAIL_WINDOW_NAME.as_bytes()).unwrap();
    let item_tpl = guis.template(b"industrial_organisation_history_item").unwrap();
    let main_layout =
        layout_window(root, &resources, DEFAULT_VIEWPORT.map(|v| v as i32), |_| {}).unwrap();
    let localisation: Localisation = [(
        "INDUSTRIAL_ORG_DETAIL_WINDOW_HISTORY_ENTRY_FLAVOR_UNITS_PRODUCED".to_owned(),
        "Units produced: $NUMBER$".to_owned(),
    )]
    .into_iter()
    .collect();
    let frame = empty_frame();
    let scripted = unread_scripted_text();
    let failures = RefCell::new(Vec::new());
    let mut data = unread_organisation("synthetic_works_organisation");
    data.custom_name = "Synthetic Works".to_owned();
    let loc = Loc::new(&localisation, &frame, &scripted, &data, &failures);

    let variants_rows = [VariantDefinitionRow {
        id: 1,
        display_name: b"Synthetic Cruiser\0".to_vec(),
        ..Default::default()
    }];
    let variants: Result<&[VariantDefinitionRow], String> = Ok(&variants_rows);
    let lexical: Result<&[LexicalTokenRow], String> = Ok(&[]);
    let rows = HistoryRecordList::Entries(vec![
        HistoryRecord { variant_id: 1, units: 42, design: DesignDate::Absent },
        HistoryRecord { variant_id: 99, units: 7, design: DesignDate::Absent },
        HistoryRecord { variant_id: 200, units: 999, design: DesignDate::Absent },
    ]);
    let icons = HashMap::from([
        (1, ResolvedHistoryIcon::NoneExpected),
        (99, ResolvedHistoryIcon::NoneExpected),
        (200, ResolvedHistoryIcon::NoneExpected),
    ]);
    let mut plan = ComposedPlan::default();
    compose_history_list(
        &main_layout,
        root,
        item_tpl,
        &resources,
        &loc,
        HistoryListInput {
            path: PRODUCTION_LIST_PATH,
            prefix: "production",
            kind: HistoryKind::Produced,
            rows: &rows,
            variants: &variants,
            lexical: &lexical,
            icons: &icons,
        },
        &mut plan,
    )
    .unwrap();

    assert!(
        plan.text.iter().any(|t| t.label == "Synthetic Cruiser" && t.value.as_deref().is_some_and(|v| v.contains("42"))),
        "visible history name and units must appear in text: {:?}",
        plan.text
    );
    assert!(
        plan.text.iter().any(|t| t.value.is_none() && t.missing.as_deref().is_some_and(|m| m.contains("VariantDefinitions") && m.contains("7"))),
        "unread history entry must appear as missing with its reason: {:?}",
        plan.text
    );
    assert!(
        !plan.text.iter().any(|t| t.value.as_deref().is_some_and(|v| v.contains("999")) || t.missing.as_deref().is_some_and(|m| m.contains("999"))),
        "offscreen history row must not appear before scrolling: {:?}",
        plan.text
    );
}

#[test]
fn trait_tree_sizes_to_installed_container_and_scrolls_to_offscreen_traits() {
    use viewer_native_gui::test_support::gui as gui_fixture;
    content_reader_script::test_game::enter();

    let layers = synthetic_layers(&[(
        "common/military_industrial_organization/organizations/00_synthetic.txt",
        br#"
            synthetic_wide_organisation = {
                trait = {
                    token = synthetic_origin
                    name = synthetic_origin
                    icon = GFX_btn
                    position = { x = 0 y = 0 }
                }
                trait = {
                    token = synthetic_far_corner
                    name = synthetic_far_corner
                    icon = GFX_btn
                    position = { x = 4 y = 4 }
                }
            }
        "#,
    )]);
    let gui_bytes = br#"guiTypes = {
        extendedScrollbarType = {
            name = "vert_bar"
            position = { x = -12 y = 0 }
            size = { width = 12 height = 18 }
            orientation = upper_right
            origo = upper_right
            startValue = 0
            background = { name = "bg" spriteType = "GFX_btn" }
            slider = { name = "slider" spriteType = "GFX_btn" }
            track = { name = "track" spriteType = "GFX_btn" }
            decreaseButton = { name = "dec" spriteType = "GFX_btn" }
            increaseButton = { name = "inc" spriteType = "GFX_btn" }
        }
        extendedScrollbarType = {
            name = "horiz_bar"
            position = { x = 0 y = -12 }
            size = { width = 18 height = 12 }
            horizontal = yes
            orientation = lower_left
            origo = lower_left
            startValue = 0
            background = { name = "bg" spriteType = "GFX_btn" }
            slider = { name = "slider" spriteType = "GFX_btn" }
            track = { name = "track" spriteType = "GFX_btn" }
            decreaseButton = { name = "dec" spriteType = "GFX_btn" }
            increaseButton = { name = "inc" spriteType = "GFX_btn" }
        }
        containerWindowType = {
            name = "industrial_organisation_detail_window"
            size = { width = 400 height = 320 }
            containerWindowType = {
                name = "tree_scrollbar_window"
                position = { x = 20 y = 20 }
                size = { width = 320 height = 260 }
                containerWindowType = {
                    name = "tree_window"
                    position = { x = 0 y = 0 }
                    size = { width = 320 height = 260 }
                    gridBoxType = { name = "tree_list" position = { x = 0 y = 0 } slotsize = { width = 320 height = 260 } }
                }
            }
        }
        containerWindowType = {
            name = "industrial_organisation_tree_window"
            size = { width = 320 height = 260 }
            background = { name = "Background" spriteType = "GFX_btn" }
            containerWindowType = {
                name = "scrollbar_window"
                position = { x = 10 y = 10 }
                size = { width = 260 height = 220 }
                margin = { top = 0 left = 0 bottom = 12 right = 12 }
                verticalScrollbar = "vert_bar"
                horizontalScrollbar = "horiz_bar"
                background = { name = "Background" spriteType = "GFX_btn" }
                containerWindowType = { name = "traits_window" position = { x = 0 y = 0 } clipping = no }
            }
        }
        containerWindowType = {
            name = "industrial_organisation_trait_entry"
            size = { width = 88 height = 112 }
            containerWindowType = {
                name = "industrial_organisation_trait_icon"
                orientation = center
                buttonType = { name = "icon_frame" position = { x = -29 y = -40 } quadTextureSprite = "GFX_industrial_org_trait_button" }
                iconType = { name = "trait_icon" position = { x = -21 y = -32 } }
                containerWindowType = { name = "queue_number_window" instantTextboxType = { name = "queue_number" position = { x = 0 y = 0 } font = "Main_14" text = "" maxWidth = 20 maxHeight = 20 } }
            }
            instantTextboxType = { name = "ai_value" position = { x = 0 y = 0 } font = "Main_14" text = "" maxWidth = 20 maxHeight = 20 }
        }
    }"#;
    let guis = super::super::load_native_guis_from_bytes("test.gui", gui_bytes).unwrap();
    let doc = native::read_at(gui_bytes, DEFAULT_VIEWPORT.map(|v| v as i32));
    let mut resources = gui_fixture::fixture_with(doc);
    resources.add_sprite("GFX_btn", [12, 12], 1);
    resources.add_sprite("GFX_industrial_org_trait_button", [58, 58], 3);
    let root = guis.template(DETAIL_WINDOW_NAME.as_bytes()).unwrap();
    let main_layout =
        layout_window(root, &resources, DEFAULT_VIEWPORT.map(|v| v as i32), |_| {}).unwrap();
    let localisation = Localisation::default();
    let frame = empty_frame();
    let scripted = unread_scripted_text();
    let failures = RefCell::new(Vec::new());
    let catalog = mio_traits::cached(&layers);
    let org_db = cached_organisation_database(&layers);
    let data = unread_organisation("synthetic_wide_organisation");
    let loc = Loc::new(&localisation, &frame, &scripted, &data, &failures);

    let run = |scroll: BTreeMap<String, f32>| -> ComposedPlan {
        let prepared = PreparedMio {
            data: data.clone(),
            sub_view: SubView::Traits,
            catalog: catalog.clone(),
            org_db: org_db.clone(),
            emblem: Ok("GFX_btn".to_owned()),
            tree_background: None,
            history_icons: HashMap::new(),
            history_art_label: None,
            scroll,
        };
        let mut plan = ComposedPlan::default();
        compose_trait_tree(&main_layout, root, &guis, &resources, &loc, &prepared, &mut plan)
            .unwrap();
        plan
    };

    let top = run(BTreeMap::new());
    assert!(top.text.iter().any(|t| t.label == "synthetic_origin"));
    assert!(
        !top.text.iter().any(|t| t.label == "synthetic_far_corner"),
        "offscreen trait must be culled to the installed 260x220 container, not hardcoded 905x640"
    );
    let v_list = top
        .extra_regions
        .iter()
        .find(|r| r.scroll.as_ref().is_some_and(|s| s.vertical && matches!(s.control, GameWindowScrollControl::List)))
        .expect("vertical tree scroll region");
    let h_list = top
        .extra_regions
        .iter()
        .find(|r| r.scroll.as_ref().is_some_and(|s| !s.vertical && matches!(s.control, GameWindowScrollControl::List)))
        .expect("horizontal tree scroll region");
    assert_eq!(v_list.rect[2], 248.0, "width matches installed scrollbar_window content width");
    assert_eq!(v_list.rect[3], 208.0, "height matches installed scrollbar_window content height");
    assert!(v_list.scroll.as_ref().unwrap().range > 0.0);
    assert!(h_list.scroll.as_ref().unwrap().range > 0.0);

    let scrolled = run(BTreeMap::from([
        (
            v_list.scroll.as_ref().unwrap().list.clone(),
            v_list.scroll.as_ref().unwrap().range,
        ),
        (
            h_list.scroll.as_ref().unwrap().list.clone(),
            h_list.scroll.as_ref().unwrap().range,
        ),
    ]));
    assert!(
        scrolled.text.iter().any(|t| t.label == "synthetic_far_corner"),
        "scrolling to the far corner must bring the offscreen trait into view: {:?}",
        scrolled.text
    );
    assert!(
        !scrolled.text.iter().any(|t| t.label == "synthetic_origin"),
        "scrolling to the far corner must scroll the origin trait out of view"
    );
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
