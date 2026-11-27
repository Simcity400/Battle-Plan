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
            sampled_tables: Vec::new(),
            funds_defines: Err("no defines".to_owned()),
            warnings: Vec::new(),
        },
        sub_view: SubView::Traits,
        catalog: Err("not read in this test".to_owned()),
        org_db: Err("not read in this test".to_owned()),
        emblem: Err(("organisation emblem".to_owned(), "not recorded".to_owned())),
        tree_background: None,
        history_icons: HashMap::new(),
        read_hour: 100,
        cadence_warning: None,
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
    assert_eq!(trait_status(&details(vec![99], vec![30]), "synthetic_third"), TraitStatus::Queued(1));
    assert!(matches!(
        trait_status(&RecordedDetails::Unavailable("not read".to_owned()), "synthetic_first"),
        TraitStatus::Unknown(reason) if reason.contains("not read")
    ));
}

#[test]
fn history_lists_follow_each_entrys_recorded_design_date() {
    let row = |organisation, variant, units, design_hour| IndustrialOrganisationHistoryRow {
        organisation,
        variant,
        units,
        design_hour,
    };
    let rows = vec![
        row(1, 1, 0, Recorded::Value(Some(100))),
        row(1, 2, 40, Recorded::Value(None)),
        row(1, 3, 5, Recorded::Value(Some(200))),
    ];
    let index = HistoryIndex::build(&rows, 1);
    assert_eq!(index.designed, HistoryIndexList::Indices(vec![0, 2]));
    assert_eq!(index.produced, HistoryIndexList::Indices(vec![1]));

    let older = vec![
        row(1, 1, 0, Recorded::NotRecorded),
        row(1, 2, 40, Recorded::NotRecorded),
    ];
    let older_index = HistoryIndex::build(&older, 1);
    assert!(
        matches!(older_index.designed, HistoryIndexList::Missing(reason) if reason.contains("design dates"))
    );
    assert!(matches!(older_index.produced, HistoryIndexList::Missing(_)));
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
        sampled_tables: Vec::new(),
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
                synthetic_conv_organisation = {
                    trait = {
                        token = synthetic_conv_trait
                        name = synthetic_conv_trait_name
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
        chosen_org_emblem(&data, Some(&tree), |_| true),
        Ok("GFX_synthetic_missing_emblem".to_owned())
    );
    let mut no_recorded = data.clone();
    no_recorded.recorded_icon.clear();
    assert_eq!(
        chosen_org_emblem(&no_recorded, Some(&tree), |_| false),
        Ok("GFX_synthetic_fallback_emblem".to_owned())
    );
    no_recorded.owner_tag = Some("SYN".to_owned());
    assert_eq!(
        chosen_org_emblem(&no_recorded, None, |name| name == "GFX_SYN_synthetic_works_organisation"),
        Ok("GFX_SYN_synthetic_works_organisation".to_owned())
    );
    assert_eq!(
        chosen_org_emblem(&no_recorded, None, |name| name == "GFX_synthetic_works_organisation"),
        Ok("GFX_synthetic_works_organisation".to_owned())
    );
    assert!(chosen_org_emblem(&no_recorded, None, |_| false).is_err());
    let conv_tree = organisation_tree(&catalog, &Ok("synthetic_conv_organisation".to_owned())).unwrap();
    let omitted_icon_trait = &conv_tree.traits[0];
    assert_eq!(
        chosen_trait_icon_name(
            omitted_icon_trait,
            "synthetic_works_organisation",
            Some("SYN"),
            |name| name == "GFX_SYN_synthetic_works_organisation_synthetic_conv_trait",
        ),
        Ok("GFX_SYN_synthetic_works_organisation_synthetic_conv_trait".to_owned())
    );
    assert_eq!(
        chosen_trait_icon_name(
            omitted_icon_trait,
            "synthetic_works_organisation",
            Some("SYN"),
            |name| name == "GFX_synthetic_works_organisation_synthetic_conv_trait",
        ),
        Ok("GFX_synthetic_works_organisation_synthetic_conv_trait".to_owned())
    );
    assert!(
        chosen_trait_icon_name(omitted_icon_trait, "synthetic_works_organisation", Some("SYN"), |_| false)
            .is_err()
    );

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

    let exhausted_variant = VariantDefinitionRow {
        id: 102,
        display_name: b"Synthetic Missing\0".to_vec(),
        icon_active: 1,
        icon_kind: 0,
        icon: b"GFX_synthetic_missing_equip".to_vec(),
        ..Default::default()
    };
    let classified_missing = classify_exhausted_variant_icon(&exhausted_variant, "synthetic_tank");
    assert!(
        matches!(
            &classified_missing,
            ResolvedHistoryIcon::Missing { asset, .. } if asset == "GFX_synthetic_missing_equip"
        ),
        "exhausted recorded equipment icon must classify as Missing naming its asset: {classified_missing:?}"
    );

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
            (102, classified_missing),
        ]),
        read_hour: 0,
        cadence_warning: None,
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
    assert!(
        policy_plan.text.iter().any(|t| t.label == "synthetic_policy" && t.value.as_deref() == Some("Attached")),
        "missing policy cost must not erase known Attached state from text: {:?}",
        policy_plan.text
    );
    assert!(
        policy_plan.text.iter().any(|t| t.label == "synthetic_policy cost" && t.missing.is_some()),
        "missing policy cost must still appear alongside Attached state: {:?}",
        policy_plan.text
    );

    let mut unspelled_data = prepared.data.clone();
    if let RecordedDetails::Available(ref mut details) = unspelled_data.details {
        let unspelled = Spelled::Unspelled {
            token: 55,
            reason: "LexicalTokens does not spell it at this hour".to_owned(),
        };
        details.policy = Some(unspelled.clone());
        details.allowed_policies = vec![unspelled];
    }
    let unspelled_prepared = PreparedMio {
        data: unspelled_data,
        sub_view: SubView::Policy,
        catalog: prepared.catalog.clone(),
        org_db: prepared.org_db.clone(),
        emblem: prepared.emblem.clone(),
        tree_background: prepared.tree_background.clone(),
        history_icons: HashMap::new(),
        read_hour: prepared.read_hour,
        cadence_warning: prepared.cadence_warning.clone(),
        scroll: BTreeMap::new(),
    };
    let mut unspelled_policy_plan = ComposedPlan::default();
    compose_policy_list(
        &policy_layout,
        policy_root,
        &guis,
        &resources,
        &loc,
        &unspelled_prepared,
        &mut unspelled_policy_plan,
    )
    .unwrap();
    assert!(
        unspelled_policy_plan.text.iter().any(|t| t.label == "Policy" && t.value.as_deref() == Some("Attached")),
        "unspelled attached policy must still emit Attached in text: {:?}",
        unspelled_policy_plan.text
    );
    assert!(
        unspelled_policy_plan.text.iter().any(|t| t.label == "Policy" && t.missing.as_deref().is_some_and(|m| m.contains("55"))),
        "unspelled attached policy must still emit its missing-name reason: {:?}",
        unspelled_policy_plan.text
    );

    let item_tpl = guis.template(b"industrial_organisation_history_item").unwrap();
    let hist_rows = [
        IndustrialOrganisationHistoryRow {
            organisation: 1,
            variant: 101,
            units: 5,
            design_hour: Recorded::Value(None),
        },
        IndustrialOrganisationHistoryRow {
            organisation: 1,
            variant: 102,
            units: 9,
            design_hour: Recorded::Value(None),
        },
    ];
    let hist_index = HistoryIndex::build(&hist_rows, 1);
    let variants_rows = [
        VariantDefinitionRow {
            id: 101,
            display_name: b"Synthetic Ready\0".to_vec(),
            ..Default::default()
        },
        exhausted_variant,
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
            list: &hist_index.produced,
            all_rows: &hist_rows,
            variants: &variants,
            lexical: &lexical,
            icons: &prepared.history_icons,
            scroll: &prepared.scroll,
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

    let measure = |other_orgs: u64, own_rows: u64, scroll_y: f32| -> (HistoryResolveCounts, bool) {
        let target_org = other_orgs / 2 + 1;
        let mut world_rows = Vec::with_capacity((other_orgs * 20 + own_rows) as usize);
        let mut variant_rows = Vec::with_capacity(own_rows as usize);
        for org in 1..=other_orgs {
            if org == target_org {
                for i in 0..own_rows {
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
        let key = HistoryIndexKey {
            source_id: format!("src-{other_orgs}-{own_rows}"),
            revision: "rev1".to_owned(),
            branch_id: 1,
            hour: 100,
            organisation_id: target_org,
            rows_ptr: world_rows.as_ptr() as usize,
            rows_len: world_rows.len(),
        };
        let (index, first_hit) = cached_history_index(key.clone(), &world_rows, target_org);
        assert!(!first_hit);
        let (index_again, second_hit) = cached_history_index(key, &world_rows, target_org);
        assert!(second_hit);
        assert!(Arc::ptr_eq(&index, &index_again));
        let HistoryIndexList::Indices(ref produced_indices) = index.produced else {
            panic!("expected produced indices");
        };
        assert_eq!(produced_indices.len(), own_rows as usize);
        let variants: Result<&[VariantDefinitionRow], String> = Ok(&variant_rows);
        let lexical: Result<&[LexicalTokenRow], String> = Ok(&[]);
        let icons = HashMap::new();
        let scroll = BTreeMap::from([(list_el.path.clone(), scroll_y)]);
        let mut plan = ComposedPlan::scrolled(scroll.clone());
        let counts = compose_history_list(
            &main_layout,
            root,
            item_tpl,
            &resources,
            &loc,
            HistoryListInput {
                path: PRODUCTION_LIST_PATH,
                prefix: "production",
                kind: HistoryKind::Produced,
                list: &index.produced,
                all_rows: &world_rows,
                variants: &variants,
                lexical: &lexical,
                icons: &icons,
                scroll: &scroll,
            },
            &mut plan,
        )
        .unwrap();
        (counts, second_hit)
    };

    let (small_mid, _) = measure(10, 30, 500.0);
    let (large_mid, _) = measure(1500, 2_000, 500.0);
    let (huge_mid, _) = measure(1500, 20_000, 500_000.0);
    assert_eq!(small_mid, large_mid);
    assert_eq!(large_mid, huge_mid);
    assert!(
        huge_mid.placed_slots <= 8,
        "list placement must stay O(visible) even for 20,000 rows in the selected org, got {huge_mid:?}"
    );
    assert!(
        huge_mid.variant_lookups <= 4 && huge_mid.variant_lookups >= 3,
        "only visible rows should resolve variants/names/layouts, got {huge_mid:?}"
    );

    let plain_gui_bytes = br#"guiTypes = {
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
                    listBoxType = { name = "prod_line_history_list" position = { x = 0 y = 0 } size = { x = 260 y = 150 } scrollbartype = "list_slider" }
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
    let plain_guis = super::super::load_native_guis_from_bytes("plain.gui", plain_gui_bytes).unwrap();
    let plain_doc = native::read_at(plain_gui_bytes, DEFAULT_VIEWPORT.map(|v| v as i32));
    let mut plain_resources = gui_fixture::fixture_with(plain_doc);
    plain_resources.add_sprite("GFX_btn", [20, 20], 1);
    let plain_root = plain_guis.template(DETAIL_WINDOW_NAME.as_bytes()).unwrap();
    let plain_item_tpl = plain_guis.template(b"industrial_organisation_history_item").unwrap();
    let plain_main =
        layout_window(plain_root, &plain_resources, DEFAULT_VIEWPORT.map(|v| v as i32), |_| {}).unwrap();
    let plain_list_el = find_direct(plain_root, PRODUCTION_LIST_PATH).unwrap();
    let plain_rows: Vec<_> = (0..20_000u64)
        .map(|i| IndustrialOrganisationHistoryRow {
            organisation: 1,
            variant: 10_000 + i,
            units: (i + 1) as i32,
            design_hour: Recorded::Value(None),
        })
        .collect();
    let plain_variants: Vec<_> = (0..20_000u64)
        .map(|i| VariantDefinitionRow {
            id: 10_000 + i,
            display_name: format!("Design {i}\0").into_bytes(),
            ..Default::default()
        })
        .collect();
    let plain_index = HistoryIndex::build(&plain_rows, 1);
    let plain_variants_res: Result<&[VariantDefinitionRow], String> = Ok(&plain_variants);
    let plain_lexical: Result<&[LexicalTokenRow], String> = Ok(&[]);
    let plain_icons = HashMap::new();
    let plain_scroll = BTreeMap::from([(plain_list_el.path.clone(), 10_000.0)]);
    let mut plain_plan = ComposedPlan::scrolled(plain_scroll.clone());
    let plain_counts = compose_history_list(
        &plain_main,
        plain_root,
        plain_item_tpl,
        &plain_resources,
        &loc,
        HistoryListInput {
            path: PRODUCTION_LIST_PATH,
            prefix: "production",
            kind: HistoryKind::Produced,
            list: &plain_index.produced,
            all_rows: &plain_rows,
            variants: &plain_variants_res,
            lexical: &plain_lexical,
            icons: &plain_icons,
            scroll: &plain_scroll,
        },
        &mut plain_plan,
    )
    .unwrap();
    assert!(
        plain_counts.placed_slots <= 4 && plain_counts.variant_lookups == 3,
        "ordinary listBoxType must stay O(visible) for 20,000 rows, got {plain_counts:?}"
    );
    let plain_bar = plain_plan
        .extra_regions
        .iter()
        .find_map(|r| r.scroll.as_ref().filter(|s| s.list == plain_list_el.path))
        .expect("ordinary listBoxType scrollbar");
    assert_eq!(plain_bar.range, 19_997.0);
    assert!(
        plain_plan.text.iter().any(|t| t.label == "Design 10000"),
        "ordinary listBoxType scroll must place the scrolled row: {:?}",
        plain_plan.text
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
    let hist_rows = [
        IndustrialOrganisationHistoryRow {
            organisation: 1,
            variant: 1,
            units: 42,
            design_hour: Recorded::Value(None),
        },
        IndustrialOrganisationHistoryRow {
            organisation: 1,
            variant: 99,
            units: 7,
            design_hour: Recorded::Value(None),
        },
        IndustrialOrganisationHistoryRow {
            organisation: 1,
            variant: 200,
            units: 999,
            design_hour: Recorded::Value(None),
        },
    ];
    let hist_index = HistoryIndex::build(&hist_rows, 1);
    let icons = HashMap::from([
        (1, ResolvedHistoryIcon::NoneExpected),
        (99, ResolvedHistoryIcon::NoneExpected),
        (200, ResolvedHistoryIcon::NoneExpected),
    ]);
    let scroll = BTreeMap::new();
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
            list: &hist_index.produced,
            all_rows: &hist_rows,
            variants: &variants,
            lexical: &lexical,
            icons: &icons,
            scroll: &scroll,
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
                tree_header_text = {
                    text = synthetic_origin_header
                    x = 0
                }
                tree_header_text = {
                    text = synthetic_far_header
                    x = 4
                }
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
                name = "tree_header"
                position = { x = 0 y = 0 }
                size = { width = 260 height = 24 }
                clipping = yes
                background = { name = "Background" spriteType = "GFX_btn" }
            }
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
            name = "industrial_organisation_flavortext_window"
            size = { width = 80 height = 24 }
            instantTextboxType = { name = "flavortext" position = { x = 0 y = 0 } font = "Main_14" text = "" maxWidth = 80 maxHeight = 20 }
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
            read_hour: 0,
            cadence_warning: None,
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
        top.text.iter().any(|t| t.label == "Tree header" && t.value.as_deref() == Some("synthetic_origin_header")),
        "visible tree column header must appear in text alternative: {:?}",
        top.text
    );
    assert!(
        !top.text.iter().any(|t| t.label == "Tree header" && t.value.as_deref() == Some("synthetic_far_header")),
        "offscreen tree column header must not appear before scrolling: {:?}",
        top.text
    );
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
        scrolled.text.iter().any(|t| t.label == "Tree header" && t.value.as_deref() == Some("synthetic_far_header")),
        "scrolling horizontally must expose the far tree column header in text alternative: {:?}",
        scrolled.text
    );
    assert!(
        !scrolled.text.iter().any(|t| t.label == "synthetic_origin"),
        "scrolling to the far corner must scroll the origin trait out of view"
    );

    let offset_gui_bytes = br#"guiTypes = {
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
                name = "tree_header"
                position = { x = 0 y = 0 }
                size = { width = 260 height = 24 }
                clipping = yes
                background = { name = "Background" spriteType = "GFX_btn" }
            }
            containerWindowType = {
                name = "scrollbar_window"
                position = { x = 10 y = 10 }
                size = { width = 260 height = 220 }
                background = { name = "Background" spriteType = "GFX_btn" }
                containerWindowType = { name = "traits_window" position = { x = 0 y = 0 } clipping = no }
            }
        }
        containerWindowType = {
            name = "industrial_organisation_flavortext_window"
            size = { width = 300 height = 24 }
            instantTextboxType = { name = "flavortext" position = { x = 270 y = 0 } font = "Main_14" text = "" maxWidth = 80 maxHeight = 20 }
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
    let offset_guis = super::super::load_native_guis_from_bytes("offset.gui", offset_gui_bytes).unwrap();
    let offset_doc = native::read_at(offset_gui_bytes, DEFAULT_VIEWPORT.map(|v| v as i32));
    let mut offset_resources = gui_fixture::fixture_with(offset_doc);
    offset_resources.add_sprite("GFX_btn", [12, 12], 1);
    offset_resources.add_sprite("GFX_industrial_org_trait_button", [58, 58], 3);
    let offset_root = offset_guis.template(DETAIL_WINDOW_NAME.as_bytes()).unwrap();
    let offset_main =
        layout_window(offset_root, &offset_resources, DEFAULT_VIEWPORT.map(|v| v as i32), |_| {}).unwrap();
    let offset_prepared = PreparedMio {
        data: data.clone(),
        sub_view: SubView::Traits,
        catalog: catalog.clone(),
        org_db: org_db.clone(),
        emblem: Ok("GFX_btn".to_owned()),
        tree_background: None,
        history_icons: HashMap::new(),
        read_hour: 0,
        cadence_warning: None,
        scroll: BTreeMap::new(),
    };
    let mut offset_plan = ComposedPlan::default();
    compose_trait_tree(
        &offset_main,
        offset_root,
        &offset_guis,
        &offset_resources,
        &loc,
        &offset_prepared,
        &mut offset_plan,
    )
    .unwrap();
    assert!(
        !offset_plan.text.iter().any(|t| t.label == "Tree header"),
        "header whose placed flavortext is clipped past tree_header must not be announced: {:?}",
        offset_plan.text
    );
}

#[test]
fn hidden_unlocked_trait_is_not_shown_and_undecided_keeps_queued_status() {
    use viewer_native_gui::test_support::gui as gui_fixture;
    content_reader_script::test_game::enter();

    let layers = synthetic_layers(&[
        (
            "common/defines/00_defines.lua",
            b"NDefines = { NGame = { MAX_SCRIPTED_LOC_RECURSION = 10 }, NBuildings = { MAX_BUILDING_LEVELS = 10 } }\n",
        ),
        (
            "map/default.map",
            b"definitions=definition.csv provinces=provinces.bmp terrain=terrain.bmp rivers=rivers.bmp heightmap=heightmap.bmp tree_definition=trees.bmp continent=continent.txt\n",
        ),
        ("map/continent.txt", b"continents = { europe }\n"),
        ("map/definition.csv", b"0;0;0;0;land;false;unknown;0\n1;1;0;0;land;false;plains;1\n"),
        (
            "common/military_industrial_organization/organizations/00_synthetic.txt",
            br#"
            synthetic_vis_organisation = {
                trait = {
                    token = synthetic_hidden_unlocked
                    name = synthetic_hidden_unlocked
                    icon = GFX_btn
                    position = { x = 0 y = 0 }
                    visible = { always = no }
                }
                trait = {
                    token = synthetic_undecided_queued
                    name = synthetic_undecided_queued
                    icon = GFX_btn
                    position = { x = 1 y = 0 }
                    visible = { FROM = { has_country_flag = synthetic_flag } }
                }
            }
        "#,
        ),
    ]);
    let gui_bytes = br#"guiTypes = {
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
    let guis = super::super::load_native_guis_from_bytes("vis.gui", gui_bytes).unwrap();
    let doc = native::read_at(gui_bytes, DEFAULT_VIEWPORT.map(|v| v as i32));
    let mut resources = gui_fixture::fixture_with(doc);
    resources.add_sprite("GFX_btn", [12, 12], 1);
    resources.add_sprite("GFX_industrial_org_trait_button", [58, 58], 3);
    let root = guis.template(DETAIL_WINDOW_NAME.as_bytes()).unwrap();
    let main_layout =
        layout_window(root, &resources, DEFAULT_VIEWPORT.map(|v| v as i32), |_| {}).unwrap();
    let localisation = Localisation::default();
    let mut frame = empty_frame();
    frame.tables[TableId::Countries.index()] =
        TableView::Complete(replay_data::rows::TableRows::Countries(vec![
            replay_data::rows::CountryRow {
                id: 1,
                canonical_slot: 1,
                tag: b"SYN".to_vec(),
                ..Default::default()
            },
        ]))
        .into();
    frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(replay_data::rows::TableRows::CountryLinks(vec![
            replay_data::rows::CountryLinkRow {
                id: 1,
                canonical_slot: 1,
                ..Default::default()
            },
        ]))
        .into();
    frame.tables[TableId::Provinces.index()] =
        TableView::Complete(replay_data::rows::TableRows::Provinces(Vec::new())).into();
    frame.tables[TableId::States.index()] =
        TableView::Complete(replay_data::rows::TableRows::States(Vec::new())).into();
    frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(replay_data::rows::TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: 1,
                literal: b"synthetic_vis_organisation".to_vec(),
            },
            LexicalTokenRow {
                token: 2,
                literal: b"synthetic_flag".to_vec(),
            },
        ]))
        .into();
    frame.tables[TableId::IndustrialOrganisations.index()] =
        TableView::Complete(replay_data::rows::TableRows::IndustrialOrganisations(vec![
            IndustrialOrganisationRow {
                organisation: 1,
                owner: 1,
                token: 1,
                ..Default::default()
            },
        ]))
        .into();
    let content = Arc::new(content_reader_layers::content::ResolvedContent {
        layers: layers.clone(),
        stats: Default::default(),
        changed: Vec::new(),
        notes: Vec::new(),
    });
    let scripted = ScriptedContent::read(content, "english".to_owned()).map(Arc::new);
    if let Err(err) = &scripted {
        panic!("synthetic scripted content must load: {err}");
    }
    let failures = RefCell::new(Vec::new());
    let catalog = mio_traits::cached(&layers);
    let org_db = cached_organisation_database(&layers);
    let mut data = unread_organisation("synthetic_vis_organisation");
    data.owner_tag = Some("SYN".to_owned());
    data.details = RecordedDetails::Available(Box::new(OrganisationDetails {
        funds_raw: 0,
        size: 1,
        points: 0,
        research_bonus_raw: 0,
        task_capacity: 1,
        research_assign_cost_raw: 0,
        production_assign_cost_raw: 0,
        design_team_change_cost: 0,
        funds_gain_factor_raw: 0,
        size_cost_factor_raw: 0,
        auto_upgrade: false,
        policy: None,
        cooldown_hour: None,
        unlocked: vec![
            Spelled::Text("synthetic_hidden_unlocked".to_owned()),
            Spelled::Unspelled {
                token: 99,
                reason: "LexicalTokens does not spell it at this hour".to_owned(),
            },
        ],
        queued: vec![Spelled::Text("synthetic_undecided_queued".to_owned())],
        allowed_policies: Vec::new(),
    }));
    let loc = Loc::new(&localisation, &frame, &scripted, &data, &failures);
    assert_eq!(
        loc.trait_condition(1, "synthetic_hidden_unlocked", OrganisationCondition::Visible),
        Ok(false)
    );
    assert!(
        loc.trait_condition(1, "synthetic_undecided_queued", OrganisationCondition::Visible)
            .is_err()
    );
    let prepared = PreparedMio {
        data,
        sub_view: SubView::Traits,
        catalog,
        org_db,
        emblem: Ok("GFX_btn".to_owned()),
        tree_background: None,
        history_icons: HashMap::new(),
        read_hour: 0,
        cadence_warning: None,
        scroll: BTreeMap::new(),
    };
    let mut plan = ComposedPlan::default();
    compose_trait_tree(&main_layout, root, &guis, &resources, &loc, &prepared, &mut plan).unwrap();
    assert!(
        !plan.text.iter().any(|t| t.label == "synthetic_hidden_unlocked"),
        "unlocked trait whose visible condition is false must stay hidden: {:?}",
        plan.text
    );
    assert!(
        plan.text.iter().any(|t| t.label == "synthetic_undecided_queued" && t.value.as_deref() == Some("Queued (#1)")),
        "queued trait with undecided visibility and an unspelled unlocked token must keep Queued (#1): {:?}",
        plan.text
    );
    assert!(
        plan.text.iter().any(|t| t.label == "synthetic_undecided_queued visibility" && t.missing.is_some()),
        "undecided visibility reason must still be emitted independently: {:?}",
        plan.text
    );
}

#[test]
fn history_icon_reads_exclude_unread_tables_after_earlier_error() {
    content_reader_script::test_game::enter();
    let mut bmp = vec![0_u8; 58];
    bmp[..2].copy_from_slice(b"BM");
    bmp[2..6].copy_from_slice(&58_u32.to_le_bytes());
    bmp[10..14].copy_from_slice(&54_u32.to_le_bytes());
    bmp[14..18].copy_from_slice(&40_u32.to_le_bytes());
    bmp[18..22].copy_from_slice(&1_i32.to_le_bytes());
    bmp[22..26].copy_from_slice(&1_i32.to_le_bytes());
    bmp[26..28].copy_from_slice(&1_u16.to_le_bytes());
    bmp[28..30].copy_from_slice(&24_u16.to_le_bytes());
    bmp[34..38].copy_from_slice(&4_u32.to_le_bytes());
    bmp[54..58].copy_from_slice(&[0, 0, 1, 0]);
    let layers = synthetic_layers(&[
        (
            "map/default.map",
            b"definitions=definition.csv provinces=provinces.bmp terrain=terrain.bmp rivers=rivers.bmp heightmap=heightmap.bmp tree_definition=trees.bmp continent=continent.txt\n",
        ),
        ("map/continent.txt", b"continents = { europe asia }\n"),
        (
            "map/definition.csv",
            b"0;0;0;0;land;false;unknown;0\n1;1;0;0;land;false;plains;1\n",
        ),
        ("map/provinces.bmp", &bmp),
        (
            "common/defines/00_defines.lua",
            b"NDefines = { NBuildings = { MAX_BUILDING_LEVELS = 10 } }\n",
        ),
        (
            "common/country_tags/00_countries.txt",
            b"SYN = \"countries/Synthetic.txt\"\n",
        ),
    ]);
    let db = designer_database(&layers, "history equipment").unwrap();
    let registry = content_reader_gui::gui::native::resources::Registry::default();
    let ic = Arc::new(
        dashboard_base::products::equipment_icon::IconContent::native(
            Arc::new(layers),
            db,
            &Err("unread".to_owned()),
            &registry,
        )
        .unwrap(),
    );
    let mut frame = empty_frame();
    let set = |frame: &mut DecodedFrame, id: TableId, rows: replay_data::rows::TableRows, hour: i32| {
        frame.tables[id.index()] =
            replay_frame::frame::SharedView::from(TableView::Complete(rows)).with_sampled_at(Some(hour));
    };
    set(&mut frame, TableId::CountryLinks, replay_data::rows::TableRows::CountryLinks(Vec::new()), 100);
    set(&mut frame, TableId::Countries, replay_data::rows::TableRows::Countries(Vec::new()), 100);
    set(&mut frame, TableId::CountryAppearance, replay_data::rows::TableRows::CountryAppearance(Vec::new()), 100);
    set(&mut frame, TableId::ReservedCountryNaming, replay_data::rows::TableRows::ReservedCountryNaming(Vec::new()), 100);
    set(&mut frame, TableId::PoliticalParties, replay_data::rows::TableRows::PoliticalParties(Vec::new()), 100);
    set(&mut frame, TableId::States, replay_data::rows::TableRows::States(Vec::new()), 10);
    frame.tables[TableId::Politics.index()] = TableView::Unavailable {
        reason: "unavailable".to_owned(),
    }
    .into();

    let (icon_hour, icon_tables) = read_icon_hour_with_tables(&frame, &Ok(ic));
    assert!(icon_hour.is_ok(), "IconHour::read retains Politics error internally");
    assert!(
        !icon_tables.contains(&TableId::States),
        "States was never read after Politics failed and must not pollute read_hour: {icon_tables:?}"
    );
    assert!(
        icon_tables.contains(&TableId::CountryAppearance),
        "tables actually read by IconHour::read must remain accounted for: {icon_tables:?}"
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
