use game_ui::assets::AssetResolver;
use game_ui::compose::ComposeOptions;
use game_ui::gfx::SpriteRegistry;
use game_ui::trade_state::{
    admit_trade_screen, apply_trade_to_compose_options, build_trade_compose_scene,
    render_trade_screen, AdmittedTradeScreen, TradeData, TradeDataIssue,
};
use replay_core::{Leaf, ResourceTradeDealRow, TradeResourceRow, TradeView, ViewStatus};
use std::collections::BTreeMap;
use std::path::PathBuf;

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
                needed: Leaf::Measured(if oil { -12.0 } else { 0.0 }),
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

fn view() -> TradeView {
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
        contracts: Leaf::Absent,
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
}

#[test]
fn shipped_gui_scene_and_compositor_accept_exact_trade_cards() {
    let game = PathBuf::from(r"X:\SteamLibrary\steamapps\common\Hearts of Iron IV");
    if !game.join("interface/countrytradeview.gui").is_file() {
        eprintln!("SKIP: shipped countrytradeview.gui unavailable");
        return;
    }
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
