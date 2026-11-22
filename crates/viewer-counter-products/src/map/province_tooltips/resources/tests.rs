use super::*;
use replay_data::{rows::TableRows, rows::TradeRouteRow};
use replay_frame::frame::DecodedFrame;
use viewer_foundation::test_support::localisation::loaded;
use viewer_runtime::resource_origins::fixture::{self, AFG, GER, MIXED, ORPHAN};

fn texts() -> Localisation {
    loaded(concat!(
        "l_english:\n",
        " TRADEVIEW_RES_PRODUCED_DETAILS_LINE:0 \"* $NUM$ $TYPE$ at $LOC$$ORIGINCONTROLLER$\"\n",
        " TRADEVIEW_RES_PRODUCED_DETAILS_LINE_PENALTY:0 \"* $NUM$/$MAX$ $TYPE$ at $LOC$ eff $PEN$$ORIGINCONTROLLER$\"\n",
        " TRADEVIEW_RES_PRODUCED_DETAILS_LINE_LOCKED:0 \"* $NUM$/$MAX$ $TYPE$ at $LOC$ held$ORIGINCONTROLLER$\"\n",
        " TRADEVIEW_RES_PRODUCED_RIGHTHOLDER:0 \" for $COUNTRY$\"\n",
        " TRADEVIEW_RES_PRODUCED_BY_BUILDING_DETAILS:0 \"  $NUM$ from buildings\"\n",
        " TRADEVIEW_RES_PRODUCED_DETAILS_LINE_ROUTE:0 \"  route: $DEL$\"\n",
        " TRADE_ROUTE_DELIVERY_TYPE_LOCKED:0 \"held\"\n",
        " TRADE_ROUTE_DELIVERY_TYPE_SEA:0 \"$VAL$ by sea, $CON$ of $TOT$\"\n",
        " INFASTRUCTURE_DAMAGE:0 \"Damage $VALUE$\"\n",
        " PRODUCTION_MATERIALS_OIL:0 \"Fuel\"\n",
        " PRODUCTION_MATERIALS_STEEL:0 \"Metal\"\n",
        " PRODUCTION_MATERIALS_RUBBER:0 \"Gum\"\n",
    ))
}

fn hover(frame: &DecodedFrame, state: u32) -> Vec<Line> {
    let content = fixture::content();
    let hour = Hour::new(frame, &content).unwrap();
    lines(&hour, state, &texts(), &|| Ok("Ruhr".to_owned()), &|raw| {
        Ok(format!("country {raw}"))
    })
    .unwrap()
}

fn text(key: &str, args: &[(&str, Arg<'_>)]) -> Result<String, String> {
    Ok(texts().localise_with(key.as_bytes(), args).unwrap())
}

#[test]
fn each_resource_writes_its_delivery_buildings_route_damage_and_refused_explanations() {
    let lines = hover(&fixture::frame(), MIXED);
    let rights = text(
        "TRADEVIEW_RES_PRODUCED_RIGHTHOLDER",
        &[("COUNTRY", Arg::Text(&format!("country {AFG}")))],
    )
    .unwrap();
    let damage = text("INFASTRUCTURE_DAMAGE", &[("VALUE", Arg::Fixed5(-30_000))]);
    assert_eq!(
        lines,
        [
            (
                "resource produced",
                text(
                    "TRADEVIEW_RES_PRODUCED_DETAILS_LINE",
                    &[
                        ("NUM", Arg::Int32(5)),
                        ("TYPE", Arg::Text("Fuel")),
                        ("LOC", Arg::Text("Ruhr")),
                        ("ORIGINCONTROLLER", Arg::Text(&rights)),
                    ]
                )
            ),
            (
                "resource buildings",
                text(
                    "TRADEVIEW_RES_PRODUCED_BY_BUILDING_DETAILS",
                    &[("NUM", Arg::Int32(1))]
                )
            ),
            (
                "resource route",
                Err(format!(
                    "the route of country {AFG}'s resource origin in state {MIXED} is not recorded"
                ))
            ),
            ("resource modifiers", Err(MODIFIERS_UNRECORDED.to_owned())),
            ("infrastructure damage", damage.clone()),
            ("temporary resources", Err(TEMPORARY_UNRECORDED.to_owned())),
            (
                "resource produced",
                text(
                    "TRADEVIEW_RES_PRODUCED_DETAILS_LINE_PENALTY",
                    &[
                        ("NUM", Arg::Int32(5)),
                        ("TYPE", Arg::Text("Metal")),
                        ("LOC", Arg::Text("Ruhr")),
                        ("ORIGINCONTROLLER", Arg::Text("")),
                        ("MAX", Arg::Int32(7)),
                        ("PEN", Arg::Fixed5(80_000)),
                    ]
                )
            ),
            (
                "resource route",
                text(
                    "TRADEVIEW_RES_PRODUCED_DETAILS_LINE_ROUTE",
                    &[(
                        "DEL",
                        Arg::Text(
                            &text(
                                "TRADE_ROUTE_DELIVERY_TYPE_SEA",
                                &[
                                    ("VAL", Arg::Int32(4)),
                                    ("CON", Arg::Int32(3)),
                                    ("TOT", Arg::Int32(5)),
                                ]
                            )
                            .unwrap()
                        )
                    )]
                )
            ),
            ("resource modifiers", Err(MODIFIERS_UNRECORDED.to_owned())),
            ("infrastructure damage", damage),
        ]
    );
}

#[test]
fn a_route_that_delivers_nothing_locks_and_a_land_route_writes_no_delivery() {
    for (kind, key, delivery) in [
        (
            0,
            "TRADEVIEW_RES_PRODUCED_DETAILS_LINE_LOCKED",
            Some("held"),
        ),
        (
            3,
            "TRADEVIEW_RES_PRODUCED_DETAILS_LINE_LOCKED",
            Some("held"),
        ),
        (1, "TRADEVIEW_RES_PRODUCED_DETAILS_LINE_PENALTY", None),
    ] {
        let mut frame = fixture::frame();
        fixture::put(
            &mut frame,
            TableRows::TradeRoutes(vec![TradeRouteRow {
                country: GER,
                class: resource_origins::ORIGIN_ROUTE_CLASS,
                other: MIXED,
                kind,
                ..Default::default()
            }]),
        );
        let lines = hover(&frame, MIXED);
        let steel = lines
            .iter()
            .position(|(_, text)| text.as_ref().is_ok_and(|text| text.contains("Metal")))
            .unwrap();
        let mut args = vec![
            ("NUM", Arg::Int32(5)),
            ("TYPE", Arg::Text("Metal")),
            ("LOC", Arg::Text("Ruhr")),
            ("ORIGINCONTROLLER", Arg::Text("")),
            ("MAX", Arg::Int32(7)),
            ("PEN", Arg::Fixed5(80_000)),
        ];
        if key.ends_with("LOCKED") {
            args.truncate(5);
        }
        assert_eq!(lines[steel].1, text(key, &args), "kind {kind}");
        let delivery = delivery.unwrap_or_default();
        assert_eq!(
            lines[steel + 1],
            (
                "resource route",
                text(
                    "TRADEVIEW_RES_PRODUCED_DETAILS_LINE_ROUTE",
                    &[("DEL", Arg::Text(delivery))]
                )
            ),
            "kind {kind}"
        );
    }
}

#[test]
fn a_state_whose_controller_holds_no_origin_writes_nothing() {
    assert!(hover(&fixture::frame(), ORPHAN).is_empty());
}
