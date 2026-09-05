use game_ui::assets::AssetResolver;
use game_ui::character_ui::{
    compose_observer_character_panel, inspect_character_layouts, resolve_named_ace_portrait,
    CharacterPanel, CharacterPanelLayouts, CharacterPortraitCatalog, CharacterViewerSelection,
    LeaderSkillCostCatalog, LeaderSkillKind, ObserverAceRow, ObserverCharacterUi, ObserverFleetRow,
    ObserverLeaderRow, ObserverLeaderSkill, ObserverValue, TraitNodePresentation, TraitNodeState,
};
use game_ui::compose::UiAssetCache;
use game_ui::gfx::SpriteRegistry;
use game_ui::loc::Localisation;
use std::path::PathBuf;

fn game_root() -> PathBuf {
    replay_test_gates::require_vanilla_game_root()
}

#[test]
#[cfg_attr(
    not(gate_hoi4_install),
    ignore = "the game install (set BATTLE_PLAN_GAME_ROOT / BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA, or install at the default Steam path)"
)]
fn installed_xui17_layouts_and_portrait_declarations_are_complete() {
    let root = game_root();
    let resolver = AssetResolver::new(vec![root]);
    let gates = inspect_character_layouts(&resolver).expect("parse XUI17 GUI files");
    assert_eq!(gates.len(), 10);
    assert!(gates.iter().all(|gate| gate.complete()), "{gates:#?}");

    let portraits = CharacterPortraitCatalog::load(&resolver).expect("parse character portraits");
    assert!(
        portraits.token_count() > 100,
        "installed portrait catalog unexpectedly small: {}",
        portraits.token_count()
    );

    let costs = LeaderSkillCostCatalog::load(&resolver).expect("parse effective leader skills");
    assert_eq!(costs.cost(LeaderSkillKind::CorpsCommander, 4), Some(1000));
    assert_eq!(costs.cost(LeaderSkillKind::FieldMarshal, 4), Some(1000));
    assert_eq!(costs.cost(LeaderSkillKind::Navy, 4), Some(800));

    let aces = resolver
        .resolve_optional_file("interface/aces.gfx")
        .expect("resolve aces.gfx")
        .expect("aces.gfx present");
    let mut sprites = SpriteRegistry::new();
    sprites
        .parse_content(&resolver.read_file(&aces).unwrap(), &aces.rel_path)
        .expect("parse aces.gfx");
    let named = resolve_named_ace_portrait("FIN", "Ilmari", "Juutilainen", &sprites)
        .expect("documented FIN named ace portrait");
    assert_eq!(named.0, "GFX_FIN_ace_Ilmari_Juutilainen");
    assert!(named.1.ends_with("FIN_ilmari_juutilainen_ace.dds"));

    let mut complete_registry = SpriteRegistry::new();
    for file in resolver
        .list_files_recursive("interface", Some("gfx"))
        .expect("enumerate effective interface GFX")
    {
        complete_registry
            .parse_content(&resolver.read_file(&file).unwrap(), &file.rel_path)
            .expect("parse effective interface GFX");
    }
    assert!(complete_registry
        .get("GFX_portrait_GER_erwin_rommel")
        .is_some());
    let localisation = Localisation::load_english(&resolver).expect("load English localisation");
    let model = ObserverCharacterUi {
        selected_country: "GER".into(),
        presentation_record_identity: None,
        trait_content_manifest_sha256: None,
        ace_content_manifest_sha256: None,
        leader_component_unavailable: None,
        leaders: vec![ObserverLeaderRow {
            unit_leader_id: 1,
            producer_identity: "d47:GER:unit_leaders:0:1".into(),
            character_token: "GER_erwin_rommel".into(),
            display_name: "Erwin Rommel".into(),
            leader_kind: "army".into(),
            skill: 4,
            leader_skill: ObserverValue::Measured(ObserverLeaderSkill {
                kind: LeaderSkillKind::CorpsCommander,
                experience: game_ui::character_ui::LeaderExperienceExact {
                    experience_raw: 0,
                    fixed_scale: 100_000,
                    next_level_cost: Some(1_500),
                    next_level_cost_raw: Some(150_000_000),
                    progress_raw: 0,
                },
                effect_text: String::new(),
                effect_text_language: "l_english".into(),
            }),
            attack: 4,
            defense: 4,
            planning: 3,
            logistics: 3,
            maneuvering: 0,
            coordination: 0,
            assignment: ObserverValue::Measured(None),
            portrait_gfx: ObserverValue::Measured("GFX_portrait_GER_erwin_rommel".into()),
            picked_traits: ObserverValue::Measured(Vec::new()),
            trait_tree_nodes: ObserverValue::Unavailable {
                port_spec: "G8",
                reason: "installed composition test does not open trait state".into(),
            },
        }],
        navy_fleets: ObserverValue::Unavailable {
            port_spec: "D28",
            reason: "installed Army composition test does not open Navy".into(),
        },
        ace_component_unavailable: Some("ACE1 not required by army roster".into()),
        aces: Vec::new(),
    };
    let viewer = CharacterViewerSelection {
        panel: CharacterPanel::ArmyRoster,
        ..CharacterViewerSelection::default()
    };
    let mut cache = UiAssetCache::default();
    let layouts = CharacterPanelLayouts::load(&resolver).expect("load installed XUI17 layouts");
    let composed = compose_observer_character_panel(
        &resolver,
        &complete_registry,
        &localisation,
        &mut cache,
        &layouts,
        &model,
        &"a".repeat(64),
        &viewer,
        1920,
        1080,
    )
    .expect("compose installed XUI17 army roster");
    assert_eq!(composed.authored_roots, ["armyleaderwindow"]);
    assert_eq!(composed.rendered_stable_ids, ["leader:1"]);
    assert_eq!(composed.applied_scroll_row, 0);
    assert!(composed.input_region.panel_rect[2] > 0);
    assert!(composed.input_region.scroll_rect[3] > 0);
    assert!(
        composed.report.missing_sprites.is_empty()
            && composed.report.undecodable_sprites.is_empty()
            && composed.report.missing_fonts.is_empty()
            && composed.report.missing_glyphs.is_empty()
            && composed.report.unresolved_loc_keys.is_empty()
            && composed.report.unresolved_text_inputs.is_empty(),
        "incomplete installed XUI17 composition: {:#?}",
        composed.report
    );
    assert!(composed.hit_targets.iter().any(|target| matches!(
        target.action,
        game_ui::character_ui::CharacterUiAction::OpenTraitTree(1)
    )));
    let army_xp = composed
        .tooltip_targets
        .iter()
        .find(|target| target.leader_experience.is_some())
        .expect("Army must expose D47-authenticated XP without a G8 presentation record");
    assert_eq!(army_xp.producer_identity, "d47:GER:unit_leaders:0:1");
    assert_eq!(army_xp.content_manifest_sha256, "a".repeat(64));

    let mut navy_model = model.clone();
    navy_model.leaders[0].leader_kind = "navy".into();
    if let ObserverValue::Measured(skill) = &mut navy_model.leaders[0].leader_skill {
        skill.kind = LeaderSkillKind::Navy;
    }
    navy_model.navy_fleets = ObserverValue::Measured(vec![ObserverFleetRow {
        fleet_id: 70,
        unit_leader_id: Some(1),
        task_force_count: 3,
    }]);
    let navy = compose_observer_character_panel(
        &resolver,
        &complete_registry,
        &localisation,
        &mut cache,
        &layouts,
        &navy_model,
        &"a".repeat(64),
        &CharacterViewerSelection {
            panel: CharacterPanel::NavyRoster,
            ..CharacterViewerSelection::default()
        },
        1920,
        1080,
    )
    .expect("compose installed XUI17 navy roster");
    assert_eq!(navy.authored_roots, ["navy_leader_window"]);
    assert!(navy.rendered_stable_ids.contains(&"fleet:70".into()));
    assert!(navy.rendered_stable_ids.contains(&"leader:1".into()));
    assert!(navy.report.missing_sprites.is_empty());
    assert!(navy.report.undecodable_sprites.is_empty());
    assert!(navy.report.missing_fonts.is_empty());

    let mut ace_model = model.clone();
    ace_model.ace_component_unavailable = None;
    ace_model.ace_content_manifest_sha256 = Some("a".repeat(64));
    ace_model.aces = vec![ObserverAceRow {
        ace_id: 9,
        display_name: "Ilmari Juutilainen".into(),
        wing_id: 0,
        portrait_gfx: ObserverValue::Measured(named.0.clone()),
        modifier_text: ObserverValue::Measured("+10% agility".into()),
        wing_display_name: ObserverValue::Measured(None),
        wing_equipment_sprite: ObserverValue::Measured(None),
    }];
    let aces = compose_observer_character_panel(
        &resolver,
        &complete_registry,
        &localisation,
        &mut cache,
        &layouts,
        &ace_model,
        &"a".repeat(64),
        &CharacterViewerSelection {
            panel: CharacterPanel::Aces,
            ..CharacterViewerSelection::default()
        },
        1920,
        1080,
    )
    .expect("compose installed XUI17 ace roster");
    assert_eq!(aces.authored_roots, ["acesview"]);
    assert_eq!(aces.rendered_stable_ids, ["ace:9"]);
    assert!(aces.report.missing_sprites.is_empty());
    assert!(aces.report.undecodable_sprites.is_empty());
    assert!(aces.report.missing_fonts.is_empty());

    let mut trait_model = model.clone();
    trait_model.presentation_record_identity = Some("installed-xui17-g8-receipt".into());
    trait_model.trait_content_manifest_sha256 = Some("a".repeat(64));
    trait_model.leaders[0].trait_tree_nodes =
        ObserverValue::Measured(vec![TraitNodePresentation {
            tree_root_offset: 0x120,
            trait_token: "panzer_leader".into(),
            definition_kind: Some(2),
            definition_census_ordinal: Some(3),
            gui_row: Some(0),
            gui_column: Some(0),
            state: TraitNodeState::Blocked,
            progress_raw: Some(0),
            progress_scale: Some(100_000),
            progress_max: Some(700),
            progress_max_source_file: Some("common/unit_leader/00_traits.txt".into()),
            reason: "exact engine-evaluated blocked reason".into(),
        }]);
    let traits = compose_observer_character_panel(
        &resolver,
        &complete_registry,
        &localisation,
        &mut cache,
        &layouts,
        &trait_model,
        &"a".repeat(64),
        &CharacterViewerSelection {
            panel: CharacterPanel::TraitTree { unit_leader_id: 1 },
            ..CharacterViewerSelection::default()
        },
        1920,
        1080,
    )
    .expect("compose installed XUI17 trait tree");
    assert!(traits
        .authored_roots
        .iter()
        .any(|root| root == "unitleader_trait_tree_window"));
    assert_eq!(traits.rendered_stable_ids, ["trait:1:288:panzer_leader"]);
    assert_eq!(traits.tooltip_targets.len(), 2);
    let tooltip = traits
        .tooltip_targets
        .iter()
        .find(|tooltip| !tooltip.blocked_reason.is_empty())
        .expect("installed blocked trait must expose an authenticated tooltip target");
    assert_eq!(tooltip.producer_identity, "installed-xui17-g8-receipt");
    assert_eq!(tooltip.content_manifest_sha256, "a".repeat(64));
    assert_eq!(
        tooltip.blocked_reason,
        "exact engine-evaluated blocked reason"
    );
    assert!(!tooltip.element_path.is_empty());
    assert!(tooltip.contains((tooltip.rect.x(), tooltip.rect.y())));
    let leader_xp = traits
        .tooltip_targets
        .iter()
        .find(|tooltip| tooltip.leader_experience.is_some())
        .expect("installed trait tree must expose the exact leader XP bar tooltip");
    assert_eq!(leader_xp.leader_skill, Some(4));
    assert!(leader_xp.contains((leader_xp.rect.x(), leader_xp.rect.y())));
    assert!(
        traits.report.missing_sprites.is_empty()
            && traits.report.undecodable_sprites.is_empty()
            && traits.report.missing_fonts.is_empty(),
        "incomplete installed XUI17 trait composition: {:#?}",
        traits.report
    );
}
