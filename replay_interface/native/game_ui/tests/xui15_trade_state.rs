use game_ui::assets::AssetResolver;
use game_ui::compose::ComposeOptions;
use game_ui::gfx::SpriteRegistry;
use game_ui::gui::GuiElement;
use game_ui::trade_state::{
    admit_trade_screen, apply_trade_to_compose_options, build_trade_compose_scene,
    render_trade_screen, AdmittedTradeScreen, TradeData, TradeDataIssue,
};
use replay_core::{
    Leaf, MarketStockpileRow, ProductionLicenseAgreementRow, ProductionLicenseVariantRow,
    ResourceTradeDealRow, TradeContractRow, TradeResourceRow, TradeView, ViewStatus,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const ORDER: [&str; 7] = [
    "oil",
    "aluminium",
    "rubber",
    "tungsten",
    "steel",
    "chromium",
    "coal",
];

fn map(oil: f64, steel: f64) -> Leaf<BTreeMap<String, f64>> {
    Leaf::Measured(
        ORDER
            .iter()
            .map(|resource| {
                let value = if *resource == "oil" {
                    oil
                } else if *resource == "steel" {
                    steel
                } else {
                    0.0
                };
                ((*resource).to_string(), value)
            })
            .collect(),
    )
}

fn resource_rows() -> Vec<TradeResourceRow> {
    ORDER
        .iter()
        .enumerate()
        .map(|(visible_order, resource)| {
            let oil = *resource == "oil";
            let steel = *resource == "steel";
            TradeResourceRow {
                resource: Leaf::Measured((*resource).to_string()),
                visible_order: Leaf::Measured(visible_order as i64),
                balance: Leaf::Measured(if oil || steel { 5.0 } else { 0.0 }),
                produced: Leaf::Measured(if oil {
                    10.0
                } else if steel {
                    20.0
                } else {
                    0.0
                }),
                imported: Leaf::Measured(if oil {
                    4.0
                } else if steel {
                    2.0
                } else {
                    0.0
                }),
                exported: Leaf::Measured(if oil {
                    1.0
                } else if steel {
                    4.0
                } else {
                    0.0
                }),
                special_project: Leaf::Measured(0.0),
                production_line: Leaf::Measured(if oil {
                    8.0
                } else if steel {
                    13.0
                } else {
                    0.0
                }),
                needed: Leaf::Measured(if oil {
                    -12.0
                } else if steel {
                    5.0
                } else {
                    0.0
                }),
                needed_current: if oil {
                    Leaf::Measured(-12.0)
                } else {
                    Leaf::Absent
                },
                needed_max: if oil {
                    Leaf::Measured(-20.0)
                } else {
                    Leaf::Absent
                },
                needed_type: Leaf::Measured(if oil { "current_max" } else { "surplus" }.into()),
            }
        })
        .collect()
}

fn deal(
    id: i64,
    partner: &str,
    visible_order: i64,
    assigned: i64,
    required: i64,
) -> ResourceTradeDealRow {
    ResourceTradeDealRow {
        deal_id: Leaf::Measured(id),
        counterpart: Leaf::Measured(partner.into()),
        direction: Leaf::Measured("import".into()),
        resource: Leaf::Measured(if visible_order == 1 { "oil" } else { "steel" }.into()),
        requested: Leaf::Measured(8.0),
        delivered: Leaf::Measured(6.0),
        available_export: Leaf::Measured(12.0),
        civilian_factories: Leaf::Measured(1),
        factory_cost_per_resource: Leaf::Measured(0.125),
        convoys_assigned: Leaf::Measured(assigned),
        convoys_required: Leaf::Measured(required),
        route_kind: Leaf::Measured(if required == 0 { "land" } else { "sea" }.into()),
        delivery_efficiency: Leaf::Measured(0.75),
        no_trade_influence: if visible_order == 0 {
            Leaf::Measured(false)
        } else {
            Leaf::Absent
        },
        no_resources: if visible_order == 0 {
            Leaf::Measured(true)
        } else {
            Leaf::Absent
        },
        visible_order: Leaf::Measured(visible_order),
    }
}

fn equipment_contract() -> TradeContractRow {
    TradeContractRow {
        market_trade_projection: Leaf::Measured(true),
        contract_id: Leaf::Measured("21:81".into()),
        counterpart: Leaf::Measured("SWE".into()),
        direction: Leaf::Measured("outgoing".into()),
        convoys_assigned: Leaf::Measured(2),
        visible_order: Leaf::Measured(0),
        equipment_purchase_relation_complete: Leaf::Measured(true),
        relation_agreement_id: Leaf::Measured("equipment_purchase:7:1:60000000".into()),
        seller: Leaf::Measured("GER".into()),
        seller_tag_db_id: Leaf::Measured(1),
        buyer: Leaf::Measured("SWE".into()),
        buyer_tag_db_id: Leaf::Measured(7),
        relation_first: Leaf::Measured("SWE".into()),
        relation_first_tag_db_id: Leaf::Measured(7),
        relation_second: Leaf::Measured("GER".into()),
        relation_second_tag_db_id: Leaf::Measured(1),
        relation_start_date_raw: Leaf::Measured(60_000_000),
        relation_start_date: Leaf::Measured("1937.11.3.0".into()),
        relation_end_date_raw: Leaf::Unobtained,
        relation_end_date: Leaf::Unobtained,
        relation_source: Leaf::Measured(
            "D43_MTLL_plus_D53_CEquipmentPurchaseRelation".into(),
        ),
    }
}

fn view() -> TradeView {
    let equipment_contract = equipment_contract();
    TradeView {
        resource_pools_complete: Leaf::Measured(true),
        produced: map(10.0, 20.0),
        imported: map(4.0, 2.0),
        used: map(8.0, 13.0),
        exported: map(1.0, 4.0),
        to_export: map(2.0, 5.0),
        base_export: map(3.0, 6.0),
        available_derived: map(9.0, 16.0),
        resource_rows: Leaf::Measured(resource_rows()),
        resource_rows_complete: Leaf::Measured(true),
        resource_deals: Leaf::Measured(vec![
            deal(41, "SWE", 0, 2, 3),
            deal(42, "SOV", 1, 0, 0),
            deal(43, "SWE", 2, 1, 2),
        ]),
        resource_deals_complete: Leaf::Measured(true),
        factories_total: Leaf::Measured(25),
        factories_used: Leaf::Measured(11),
        factories_available: Leaf::Measured(14),
        contracts: Leaf::Measured(vec![equipment_contract.clone()]),
        equipment_purchase_contracts: Leaf::Measured(vec![equipment_contract]),
        market_stockpile: Leaf::Measured(vec![
            MarketStockpileRow {
                equipment_variant_id: Leaf::Measured(90),
                equipment_type: Leaf::Measured(70),
                amount_raw: Leaf::Measured(76_700_000),
                amount: Leaf::Measured(767.0),
                ordinal: Leaf::Measured(0),
            },
            MarketStockpileRow {
                equipment_variant_id: Leaf::Measured(91),
                equipment_type: Leaf::Measured(70),
                amount_raw: Leaf::Measured(100_000),
                amount: Leaf::Measured(1.0),
                ordinal: Leaf::Measured(1),
            },
        ]),
        market_stockpile_complete: Leaf::Measured(true),
        production_license_agreements: Leaf::Measured(vec![ProductionLicenseAgreementRow {
            agreement_id: Leaf::Measured("licence:7:1:60000000".into()),
            visible_order: Leaf::Measured(0),
            owner: Leaf::Measured("GER".into()),
            owner_tag_db_id: Leaf::Measured(1),
            partner: Leaf::Measured("SWE".into()),
            partner_tag_db_id: Leaf::Measured(7),
            relation_first: Leaf::Measured("SWE".into()),
            relation_first_tag_db_id: Leaf::Measured(7),
            relation_second: Leaf::Measured("GER".into()),
            relation_second_tag_db_id: Leaf::Measured(1),
            start_date_raw: Leaf::Measured(60_000_000),
            start_date: Leaf::Measured("1937.11.3.0".into()),
            end_date_raw: Leaf::Unobtained,
            end_date: Leaf::Unobtained,
            variants: Leaf::Measured(vec![ProductionLicenseVariantRow {
                variant_id: Leaf::Measured(5081),
                visible_order: Leaf::Measured(0),
                creator_tag_db_id: Leaf::Measured(7),
                origin_tag_db_id: Leaf::Measured(0),
                source_tag_db_id: Leaf::Measured(7),
            }]),
            source: Leaf::Measured(
                "PLIC_v2_plus_D53_CLicenceProductionAccessRelation".into(),
            ),
        }]),
        production_license_agreements_complete: Leaf::Measured(true),
    }
}

fn presentation() -> game_ui::trade_state::TradePresentation {
    let AdmittedTradeScreen::Visible(presentation) =
        admit_trade_screen(ViewStatus::Present, "GER", Some(&view()))
    else {
        panic!("complete view must be visible");
    };
    presentation
}

fn deterministic_trade_gui_root() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "xui15-license-compose-{}-{nonce}",
        std::process::id()
    ));
    let interface = root.join("interface");
    fs::create_dir_all(&interface).expect("create deterministic XUI15 fixture root");
    fs::write(
        interface.join("countrytradeview.gui"),
        r#"
guiTypes = {
    containerWindowType = {
        name = "countrytradeview"
        show_position = { x = -6 y = 78 }
        size = { width = 689 height = 1000 }
    }
    containerWindowType = {
        name = "resources_info_entry"
        size = { width = 79 height = 156 }
    }
    containerWindowType = {
        name = "country_trade_entry"
        position = { x = 0 y = 0 }
        size = { width = 648 height = 45 }
        buttonType = {
            name = "country_flag"
            position = { x = 6 y = 8 }
            quadTextureSprite = "GFX_flag_small2"
        }
        iconType = {
            name = "country_flag_frame"
            position = { x = 3 y = 6 }
            spriteType = "GFX_small_flag_frame_thin"
        }
        instantTextboxType = {
            name = "country"
            position = { x = 52 y = 6 }
            font = "hoi_16mbs"
            text = "name"
            maxWidth = 130
            maxHeight = 28
            format = center
        }
        instantTextboxType = {
            name = "exported"
            position = { x = 365 y = 12 }
            font = "hoi_18mbs"
            text = ""
            maxWidth = 120
            maxHeight = 18
            format = center
        }
        instantTextboxType = {
            name = "available_export"
            position = { x = 198 y = 12 }
            font = "hoi_18mbs"
            text = ""
            maxWidth = 120
            maxHeight = 18
            format = center
        }
        instantTextboxType = {
            name = "convoys"
            position = { x = 520 y = 12 }
            font = "hoi_18mbs"
            text = ""
            maxWidth = 120
            maxHeight = 18
            format = center
        }
    }
}
"#,
    )
    .expect("write deterministic XUI15 GUI fixture");
    root
}

fn descendant<'a>(element: &'a GuiElement, name: &str) -> Option<&'a GuiElement> {
    if element.name == name {
        return Some(element);
    }
    element
        .children
        .iter()
        .find_map(|child| descendant(child, name))
}

#[test]
fn exact_admission_preserves_resource_and_deal_order_with_complete_totals() {
    let presentation = presentation();
    let TradeData::Available(resources) = &presentation.resource_cards else {
        panic!("resource cards unavailable");
    };
    assert_eq!(
        resources
            .iter()
            .map(|row| row.resource.as_str())
            .collect::<Vec<_>>(),
        ORDER
    );
    assert_eq!(resources[0].needed_current, Some(-12.0));
    assert_eq!(resources[0].needed_max, Some(-20.0));

    let TradeData::Available(deals) = &presentation.deals else {
        panic!("deal cards unavailable");
    };
    assert_eq!(
        deals.iter().map(|row| row.deal_id).collect::<Vec<_>>(),
        vec![41, 42, 43]
    );
    assert!(deals[0].no_convoys);
    assert_eq!(deals[0].no_resources, TradeData::Available(true));
    assert_eq!(
        deals[1].no_resources,
        TradeData::Unavailable(TradeDataIssue::NotRecorded)
    );

    let TradeData::Available(totals) = &presentation.deal_totals else {
        panic!("deal totals unavailable");
    };
    assert_eq!(totals.requested, 24.0);
    assert_eq!(totals.delivered, 18.0);
    assert_eq!(totals.civilian_factories, 3);
    assert_eq!(totals.convoys_assigned, 3);
    assert_eq!(totals.convoys_required, 5);
    assert_eq!(totals.partners, vec!["SWE", "SOV"]);
    let TradeData::Available(stockpile) = &presentation.market_stockpile else {
        panic!("market stockpile unavailable");
    };
    assert_eq!(
        stockpile
            .iter()
            .map(|row| row.equipment_variant_id)
            .collect::<Vec<_>>(),
        vec![90, 91]
    );
    assert_eq!(stockpile[0].amount_raw, 76_700_000);
    let TradeData::Available(contracts) = &presentation.equipment_purchase_contracts else {
        panic!("equipment purchase relation context unavailable");
    };
    assert_eq!(contracts[0].counterpart, "SWE");
    assert_eq!(contracts[0].agreement_id, "equipment_purchase:7:1:60000000");
    let TradeData::Available(licenses) = &presentation.production_license_agreements else {
        panic!("licence agreements unavailable");
    };
    assert_eq!(licenses[0].partner, "SWE");
    assert_eq!(licenses[0].variants[0].variant_id, 5081);
}

#[test]
fn incomplete_or_misordered_sections_are_explicitly_unavailable() {
    let mut incomplete = view();
    incomplete.resource_rows_complete = Leaf::Measured(false);
    incomplete.resource_deals = Leaf::Unobtained;
    let AdmittedTradeScreen::Visible(presentation) =
        admit_trade_screen(ViewStatus::Present, "GER", Some(&incomplete))
    else {
        panic!("country remains visible with unavailable sections");
    };
    assert_eq!(
        presentation.resource_cards,
        TradeData::Unavailable(TradeDataIssue::IncompleteWitness)
    );
    assert_eq!(
        presentation.deals,
        TradeData::Unavailable(TradeDataIssue::CapturedButUnobtained)
    );

    let mut wrong_order = view();
    let Leaf::Measured(rows) = &mut wrong_order.resource_rows else {
        unreachable!()
    };
    rows.swap(1, 4);
    assert!(matches!(
        admit_trade_screen(ViewStatus::Present, "GER", Some(&wrong_order)),
        AdmittedTradeScreen::Visible(game_ui::trade_state::TradePresentation {
            resource_cards: TradeData::Unavailable(TradeDataIssue::IncompleteRow),
            ..
        })
    ));
    assert_eq!(
        admit_trade_screen(ViewStatus::DomainAbsent, "GER", None),
        AdmittedTradeScreen::Unavailable(
            game_ui::trade_state::TradeScreenUnavailable::DomainAbsent
        )
    );
}

#[test]
fn compose_binding_uses_authored_names_and_keeps_visible_order() {
    let presentation = presentation();
    let mut options = ComposeOptions::new(1920, 1080);
    apply_trade_to_compose_options(&presentation, &mut options);
    assert_eq!(options.text_overrides["factories_total"], "25");
    assert_eq!(options.text_overrides["xui15_equipment_contract_count"], "1");
    assert_eq!(
        options.text_overrides["xui15_equipment_contract_0_counterpart"],
        "SWE"
    );
    assert_eq!(options.text_overrides["xui15_license_count"], "1");
    assert_eq!(
        options.text_overrides["xui15_resource_0_needed"],
        "-12\n-20"
    );
    assert_eq!(
        options.frame_overrides["xui15_resource_4_resources_icon"],
        4
    );
    assert_eq!(options.text_overrides["xui15_deal_0_country"], "SWE");
    assert_eq!(options.text_overrides["xui15_deal_1_country"], "SOV");
    assert_eq!(options.text_overrides["xui15_deal_0_convoys"], "2/3");
    assert_eq!(options.text_overrides["xui15_license_count"], "1");
    assert_eq!(options.text_overrides["xui15_license_0_partner"], "SWE");
}

#[test]
fn production_license_options_bind_real_authored_scene_children() {
    let root = deterministic_trade_gui_root();
    let resolver = AssetResolver::new(vec![root.clone()]);
    let screen = AdmittedTradeScreen::Visible(presentation());
    let scene = build_trade_compose_scene(&resolver, &screen, 1920, 1080)
        .expect("build deterministic XUI15 licence scene");

    let license_overrides = scene
        .options
        .text_overrides
        .keys()
        .filter(|name| name.starts_with("xui15_license"))
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(license_overrides.len(), 5);
    for name in &license_overrides {
        assert!(
            descendant(&scene.root, name).is_some(),
            "licence override {name} must bind a concrete scene child"
        );
    }
    assert!(scene
        .options
        .hidden_elements
        .contains("xui15_license_country_flag"));
    assert!(scene
        .options
        .hidden_elements
        .contains("xui15_license_0_country_flag"));

    let partner = descendant(&scene.root, "xui15_license_0_partner")
        .expect("authored country text becomes licence partner");
    assert_eq!(partner.font.as_deref(), Some("hoi_16mbs"));
    assert_eq!(partner.format.as_deref(), Some("center"));
    for name in [
        "xui15_license_0_start_date",
        "xui15_license_0_end_date",
        "xui15_license_0_variant_count",
    ] {
        let field = descendant(&scene.root, name).expect("authored licence field");
        assert_eq!(field.font.as_deref(), Some("hoi_18mbs"));
        assert_eq!(field.format.as_deref(), Some("center"));
    }

    let output = render_trade_screen(
        &resolver,
        &SpriteRegistry::new(),
        &screen,
        1920,
        1080,
    )
    .expect("compose deterministic XUI15 licence scene");
    let rect = |name: &str| {
        output
            .debug_rects
            .iter()
            .find(|rect| rect.name == name)
            .unwrap_or_else(|| panic!("missing composed rect {name}"))
    };
    assert_eq!(
        (
            rect("xui15_license_count").x,
            rect("xui15_license_count").y
        ),
        (61, 681)
    );
    assert_eq!(
        (
            rect("xui15_license_0_partner").x,
            rect("xui15_license_0_partner").y
        ),
        (61, 726)
    );
    assert_eq!(
        (
            rect("xui15_license_0_start_date").x,
            rect("xui15_license_0_start_date").y
        ),
        (207, 732)
    );
    assert_eq!(
        (
            rect("xui15_license_0_end_date").x,
            rect("xui15_license_0_end_date").y
        ),
        (374, 732)
    );
    assert_eq!(
        (
            rect("xui15_license_0_variant_count").x,
            rect("xui15_license_0_variant_count").y
        ),
        (529, 732)
    );
    assert!(output
        .debug_rects
        .iter()
        .all(|rect| rect.name != "xui15_license_0_country_flag"));

    fs::remove_dir_all(root).expect("remove deterministic XUI15 fixture root");
}

#[test]
fn unavailable_production_licenses_render_only_fail_closed_notice() {
    let root = deterministic_trade_gui_root();
    let resolver = AssetResolver::new(vec![root.clone()]);
    let mut presentation = presentation();
    presentation.production_license_agreements =
        TradeData::Unavailable(TradeDataIssue::CapturedButUnobtained);
    let screen = AdmittedTradeScreen::Visible(presentation);
    let scene = build_trade_compose_scene(&resolver, &screen, 1920, 1080)
        .expect("build unavailable XUI15 licence scene");

    assert_eq!(scene.options.text_overrides["xui15_license_count"], "--");
    assert!(descendant(&scene.root, "xui15_license_count").is_some());
    assert!(descendant(&scene.root, "xui15_licenses_unavailable").is_some());
    assert!(descendant(&scene.root, "xui15_license_0_partner").is_none());

    fs::remove_dir_all(root).expect("remove deterministic XUI15 fixture root");
}

#[test]
#[cfg_attr(
    not(gate_hoi4_install),
    ignore = "the game install (set BATTLE_PLAN_GAME_ROOT / BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA, or install at the default Steam path)"
)]
fn shipped_gui_scene_and_compositor_accept_exact_trade_cards() {
    let game = replay_test_gates::require_vanilla_game_root();
    let resolver = AssetResolver::new(vec![game]);
    let screen = AdmittedTradeScreen::Visible(presentation());
    let scene = build_trade_compose_scene(&resolver, &screen, 1920, 1080)
        .expect("build exact trade scene from shipped GUI");
    assert_eq!(
        scene
            .root
            .children
            .iter()
            .filter(|child| child.name.starts_with("xui15_resource_"))
            .count(),
        7
    );
    assert_eq!(
        scene
            .root
            .children
            .iter()
            .filter(|child| child.name.starts_with("xui15_deal_"))
            .count(),
        3
    );
    let output = render_trade_screen(&resolver, &SpriteRegistry::new(), &screen, 1920, 1080)
        .expect("render exact trade scene");
    assert_eq!((output.image.width, output.image.height), (1920, 1080));
    assert!(output.report.drawn_elements > 0);
    assert!(output
        .debug_rects
        .iter()
        .any(|rect| rect.name == "xui15_resource_0_balance"));
}


// -------------------------------------------------------------------------
// The resource set is whatever the recording states, not a list compiled into
// the panel. `common/resources` is moddable content.
// -------------------------------------------------------------------------

/// Vanilla's seven, plus a resource vanilla does not have, in an order vanilla
/// does not use. Every card is built, in the recording's order.
#[test]
fn xui15_trade_panel_renders_a_modded_eight_resource_set_in_the_recordings_order() {
    // The first slot is deliberately NOT named "oil": a mod may rename the
    // engine's cached first resource, and the current/max needed form belongs
    // to that SLOT, not to that name.
    let modded = [
        "petroleum", "steel", "rubber", "tungsten", "aluminium", "chromium", "coal", "uranium",
    ];
    let mut view = view();
    view.resource_rows = Leaf::Measured(resource_rows_for(&modded));
    let pools = pool_map_for(&modded);
    view.produced = pools.clone();
    view.imported = pools.clone();
    view.used = pools.clone();
    view.exported = pools.clone();
    view.to_export = pools.clone();
    view.base_export = pools.clone();
    view.available_derived = pools;

    let AdmittedTradeScreen::Visible(screen) =
        admit_trade_screen(ViewStatus::Present, "GER", Some(&view))
    else {
        panic!("modded trade view must be visible");
    };

    let TradeData::Available(cards) = &screen.resource_cards else {
        panic!("resource cards unavailable: {:?}", screen.resource_cards);
    };
    assert_eq!(cards.len(), 8);
    for (index, resource) in modded.iter().enumerate() {
        assert_eq!(cards[index].resource, *resource);
        assert_eq!(cards[index].visible_order, index as i64);
    }

    let TradeData::Available(pool_cards) = &screen.resource_pools else {
        panic!("pool cards unavailable: {:?}", screen.resource_pools);
    };
    assert_eq!(pool_cards.len(), 8);
    for (index, resource) in modded.iter().enumerate() {
        assert_eq!(pool_cards[index].resource, *resource);
    }
}

/// A REORDERED resource table still builds every card, including oil's.
///
/// `common/resources` is moddable, so a mod may prepend to it and move oil off
/// slot 0. The recorder identifies oil from the engine's own cached oil
/// descriptor and stamps the current/max needed model on whichever slot that
/// turns out to be; the panel must key on that stamp.
///
/// Restoring the `expected_order == 0` conjunct in `measured_resource_card`
/// makes this red: the oil card fails its needed-semantics check, one card of
/// eight comes back unbuilt, and the whole panel withholds its resource cards.
#[test]
fn xui15_trade_panel_builds_the_oil_card_when_a_mod_moves_oil_off_the_first_slot() {
    const OIL_SLOT: usize = 3;
    let modded = [
        "aether", "bauxite", "cobalt", "oil", "steel", "rubber", "tungsten", "uranium",
    ];
    let mut view = view();
    view.resource_rows = Leaf::Measured(resource_rows_for_with_oil_slot(&modded, OIL_SLOT));
    let pools = pool_map_for(&modded);
    view.produced = pools.clone();
    view.imported = pools.clone();
    view.used = pools.clone();
    view.exported = pools.clone();
    view.to_export = pools.clone();
    view.base_export = pools.clone();
    view.available_derived = pools;

    let AdmittedTradeScreen::Visible(screen) =
        admit_trade_screen(ViewStatus::Present, "GER", Some(&view))
    else {
        panic!("a reordered resource table must still be visible");
    };

    let TradeData::Available(cards) = &screen.resource_cards else {
        panic!("resource cards unavailable: {:?}", screen.resource_cards);
    };
    assert_eq!(cards.len(), 8);
    for (index, resource) in modded.iter().enumerate() {
        assert_eq!(cards[index].resource, *resource);
        assert_eq!(cards[index].visible_order, index as i64);
        // The current/max pair is on the slot the recording marked, and on no
        // other -- in particular not on slot 0, which a positional panel would
        // have insisted on.
        let is_oil = index == OIL_SLOT;
        assert_eq!(
            cards[index].needed_type,
            if is_oil { "current_max" } else { "surplus" },
            "slot {index} ({resource})"
        );
        assert_eq!(
            cards[index].needed_current.is_some(),
            is_oil,
            "slot {index} ({resource}) current/max presence"
        );
    }
}

/// The pools are keyed by name and carry no order of their own. Without rows
/// there is no stated order, and the panel must say so rather than invent one:
/// rendering the map's own key order would put cards in a sequence the game
/// never had.
#[test]
fn xui15_trade_panel_withholds_pool_cards_when_no_resource_order_is_stated() {
    let mut view = view();
    view.resource_rows = Leaf::Unobtained;

    let AdmittedTradeScreen::Visible(screen) =
        admit_trade_screen(ViewStatus::Present, "GER", Some(&view))
    else {
        panic!("trade view must be visible");
    };

    assert!(
        matches!(
            &screen.resource_pools,
            TradeData::Unavailable(TradeDataIssue::InvalidExactOrder)
        ),
        "expected InvalidExactOrder, got {:?}",
        screen.resource_pools
    );
}

/// A pool map keyed by a set the rows do not state is not this frame's pools.
#[test]
fn xui15_trade_panel_withholds_pool_cards_whose_keys_disagree_with_the_rows() {
    let mut view = view();
    let mut disagreeing = BTreeMap::new();
    for (index, resource) in ORDER.iter().enumerate() {
        let key = if index == 6 { "uranium" } else { resource };
        disagreeing.insert(key.to_string(), 0.0);
    }
    view.produced = Leaf::Measured(disagreeing);

    let AdmittedTradeScreen::Visible(screen) =
        admit_trade_screen(ViewStatus::Present, "GER", Some(&view))
    else {
        panic!("trade view must be visible");
    };

    assert!(
        matches!(
            &screen.resource_pools,
            TradeData::Unavailable(TradeDataIssue::InvalidExactOrder)
        ),
        "expected InvalidExactOrder, got {:?}",
        screen.resource_pools
    );
}

fn pool_map_for(resources: &[&str]) -> Leaf<BTreeMap<String, f64>> {
    Leaf::Measured(
        resources
            .iter()
            .enumerate()
            .map(|(index, resource)| ((*resource).to_string(), index as f64))
            .collect(),
    )
}

/// Rows for an arbitrary resource set. The current/max needed form belongs to
/// slot 0 -- the engine's cached first resource -- and every other slot carries
/// the surplus/deficit form; nothing here depends on a resource's name.
fn resource_rows_for(resources: &[&str]) -> Vec<TradeResourceRow> {
    resource_rows_for_with_oil_slot(resources, 0)
}

/// The same rows with the OIL SLOT as a parameter. Vanilla puts oil first, so
/// the caller above passes 0; a mod that prepends to `common/resources` does
/// not, and the panel has to key on the stamped needed-model rather than on the
/// position.
fn resource_rows_for_with_oil_slot(resources: &[&str], oil_slot: usize) -> Vec<TradeResourceRow> {
    resources
        .iter()
        .enumerate()
        .map(|(visible_order, resource)| {
            let first = visible_order == oil_slot;
            TradeResourceRow {
                resource: Leaf::Measured((*resource).to_string()),
                visible_order: Leaf::Measured(visible_order as i64),
                balance: Leaf::Measured(0.0),
                produced: Leaf::Measured(0.0),
                imported: Leaf::Measured(0.0),
                exported: Leaf::Measured(0.0),
                special_project: Leaf::Measured(0.0),
                production_line: Leaf::Measured(0.0),
                needed: Leaf::Measured(0.0),
                needed_current: if first {
                    Leaf::Measured(0.0)
                } else {
                    Leaf::Absent
                },
                needed_max: if first {
                    Leaf::Measured(0.0)
                } else {
                    Leaf::Absent
                },
                needed_type: Leaf::Measured(
                    if first { "current_max" } else { "surplus" }.into(),
                ),
            }
        })
        .collect()
}
