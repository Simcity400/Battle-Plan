use super::resource_name;
use crate::test_fixtures::{CAMPAIGN, records, source};
use replay::{
    container::Replay,
    frame::{DecodedFrame, TableView},
    rows::{CountryLinkRow, CountryRow, TableId, TableRows},
};
use std::sync::{Arc, atomic::AtomicBool};

/// The name is `PRODUCTION_MATERIALS_` and the upper-cased token through
/// the plain localiser (140BBC360 -> 142230F20), loaded as content loads.
#[test]
fn a_resource_is_named_as_the_games_resource_texts_name_it() {
    let localisation = crate::catalogue::tests::loaded(concat!(
        "l_english:\n",
        " PRODUCTION_MATERIALS_OIL:0 \"Oil [ROOT.GetName]\"\n",
        " PRODUCTION_MATERIALS_STEEL:0 \"\"\n",
        " PRODUCTION_MATERIALS_COAL:0 \"a¤b\"\n",
        " state_resource_oil:0 \"Not the name\"\n",
    ));
    let mut warnings = Vec::new();
    // Mixed case: toupper folds the token, so both name the same key.
    for token in ["oil", "Oil"] {
        assert_eq!(
            resource_name(&localisation, token, &mut warnings).as_deref(),
            Some("Oil [ROOT.GetName]")
        );
    }
    assert_eq!(
        resource_name(&localisation, "steel", &mut warnings).as_deref(),
        Some("")
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(
        resource_name(&localisation, "tungsten", &mut warnings).as_deref(),
        Some("PRODUCTION_MATERIALS_TUNGSTEN")
    );
    assert_eq!(resource_name(&localisation, "coal", &mut warnings), None);
    // The "C" locale's toupper leaves a byte above 0x7F.
    assert_eq!(
        resource_name(&localisation, "stål", &mut warnings).as_deref(),
        Some("PRODUCTION_MATERIALS_STåL")
    );
    assert_eq!(
        warnings,
        [
            "coal: no name: localisation PRODUCTION_MATERIALS_COAL holds the control byte 0x14 (an authored `¤`), which plain text cannot show",
        ]
    );
}

/// Germany (raw links 11 and `germany`, slot 7), Italy (21, slot 8) and Spain
/// (31, slot 9), with steel and oil. Deals are (handle, exporter, importer,
/// delivered).
fn resource_frame(
    hour: i32,
    steel: i64,
    germany: u32,
    deals: &[(u64, u32, u32, i64)],
) -> DecodedFrame {
    use replay::rows::{
        CountryResourceRow, LexicalTokenRow, ResourceDefinitionRow, ResourceExchangeRow,
    };
    let mut frame = source(1, hour, Some(1));
    let country = |id, canonical_slot, tag: &[u8]| CountryRow {
        id,
        canonical_slot,
        tag: tag.to_vec(),
        ..Default::default()
    };
    *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![
        country(21, 8, b"ITA"),
        country(31, 9, b"SPR"),
        country(germany, 7, b"GER"),
    ]));
    let link = |id, canonical_slot| CountryLinkRow { id, canonical_slot };
    *frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![
            link(11, 7),
            link(21, 8),
            link(31, 9),
            link(germany, 7),
        ]));
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: 501,
                literal: b"steel".to_vec(),
            },
            LexicalTokenRow {
                token: 502,
                literal: b"oil".to_vec(),
            },
        ]));
    *frame.tables[TableId::ResourceDefinitions.index()] =
        TableView::Complete(TableRows::ResourceDefinitions(vec![
            ResourceDefinitionRow {
                id: 1,
                token: 501,
                active: 1,
                factory_cost: 12_500,
            },
            ResourceDefinitionRow {
                id: 2,
                token: 502,
                active: 1,
                factory_cost: 12_500,
            },
        ]));
    let pools = |country, resource, produced| CountryResourceRow {
        country,
        resource,
        produced,
        imported: 800_000,
        to_export: 2_500_000,
        exported: 1_700_000,
        used: produced + 800_000 - 2_500_000,
        // Special projects take 0.4; the production lines need 30.
        special_total: produced + 800_000 - 2_500_000 - 40_000,
        production_demand: -3_000_000,
        ..Default::default()
    };
    *frame.tables[TableId::CountryResources.index()] =
        TableView::Complete(TableRows::CountryResources(vec![
            // Germany's rows name it by its other link.
            pools(11, 1, steel),
            pools(11, 2, 0),
            pools(21, 1, 99_900_000),
        ]));
    *frame.tables[TableId::ResourceExchanges.index()] =
        TableView::Complete(TableRows::ResourceExchanges(
            deals
                .iter()
                .map(|&(id, exporter, importer, delivered)| ResourceExchangeRow {
                    id,
                    exporter,
                    importer,
                    resource: 1,
                    delivered,
                    assigned_factories: 1,
                    required_factories: 1,
                    ..Default::default()
                })
                .collect(),
        ));
    frame
}

#[test]
fn resource_values_run_over_recorded_hours_and_never_cross_gaps() {
    use crate::products::resource_balance::{TradeDirection, scan};
    let import = [(0x33, 21, 9001, 800_000)];
    let both = [(0x33, 21, 9001, 800_000), (0x44, 11, 31, 1_700_000)];
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                resource_frame(100, 30_100_000, 9001, &import),
                resource_frame(101, 30_100_000, 9001, &import),
                // Every table is read every hour, so a new value shows at once.
                resource_frame(102, 1, 9001, &import),
                resource_frame(103, 30_100_000, 9001, &both),
                resource_frame(104, 31_000_000, 9001, &both),
                // Hour 105 is missing: 106 starts anew.
                resource_frame(106, 31_000_000, 9001, &both),
                resource_frame(107, 31_000_000, 9001, &both),
                // Another country took the tag: its pools and deals start anew.
                // Italy's deal names Germany's old link, so it is not the new
                // Germany's.
                resource_frame(108, 31_000_000, 9002, &both),
                resource_frame(109, 31_000_000, 9002, &[]),
            ],
        )],
    )
    .unwrap();
    let scanned = scan(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
    assert_eq!(
        scanned.resources,
        [(1, "steel".to_owned()), (2, "oil".to_owned())]
    );
    let steel: Vec<_> = scanned.series[&1]
        .iter()
        .map(|run| {
            (
                run.first_hour,
                run.last_hour,
                run.starts_segment,
                run.produced_raw.as_str(),
            )
        })
        .collect();
    assert_eq!(
        steel,
        [
            (100, 101, true, "30100000"),
            (102, 102, false, "1"),
            (103, 103, false, "30100000"),
            (104, 104, false, "31000000"),
            (106, 107, true, "31000000"),
            (108, 109, true, "31000000"),
        ]
    );
    let run = &scanned.series[&1][0];
    assert_eq!(
        (
            run.available_raw.as_str(),
            run.after_projects_raw.as_str(),
            run.to_export_raw.as_str(),
            run.production_raw.as_str()
        ),
        ("28400000", "28360000", "2500000", "-3000000")
    );
    // Italy's rows are not Germany's.
    assert_eq!(scanned.series.len(), 2);
    let trades: Vec<_> = scanned
        .trades
        .iter()
        .map(|trade| {
            (
                trade.direction,
                trade.partner.as_str(),
                trade.first_hour,
                trade.last_hour,
                trade.starts_segment,
            )
        })
        .collect();
    assert_eq!(
        trades,
        [
            (TradeDirection::Import, "ITA", 100, 104, true),
            (TradeDirection::Export, "SPR", 103, 104, true),
            (TradeDirection::Import, "ITA", 106, 107, true),
            (TradeDirection::Export, "SPR", 106, 107, true),
            (TradeDirection::Export, "SPR", 108, 108, true),
        ]
    );
    assert_eq!(
        scanned
            .deal_hours
            .iter()
            .map(|span| (span.first_hour, span.last_hour))
            .collect::<Vec<_>>(),
        [(100, 104), (106, 109)]
    );
    assert_eq!(scanned.trades[0].delivered_raw, "800000");
    assert_eq!(scanned.unnamed + scanned.unresolved, 0);
}

#[test]
fn resource_balance_replies_without_content_and_answers_again_from_the_cache() {
    use crate::products::{
        CountryParams, LoadContext, Product,
        resource_balance::{RESOURCE_DECIMALS, ResourceBalance},
    };
    let replay = Arc::new(
        Replay::from_records(
            CAMPAIGN.into(),
            &[records(
                1,
                vec![
                    resource_frame(100, 30_100_000, 9001, &[]),
                    resource_frame(101, 30_100_000, 9001, &[]),
                ],
            )],
        )
        .unwrap(),
    );
    let source = crate::Source::from_replay(
        crate::SourceKey {
            source_id: "resources".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        replay,
        &AtomicBool::new(false),
    )
    .unwrap();
    let params = CountryParams {
        branch_id: "1".into(),
        tag: "GER".into(),
    };
    let load = |params: &CountryParams| {
        ResourceBalance::load(
            &source,
            params,
            &LoadContext { content: None },
            &AtomicBool::new(false),
        )
        .unwrap()
    };
    let reply = load(&params);
    assert_eq!(reply.decimals, RESOURCE_DECIMALS);
    // Without content, resources keep their tokens and have no names or icons.
    assert_eq!(
        reply
            .resources
            .iter()
            .map(|resource| (
                resource.token.as_str(),
                resource.name.is_none(),
                resource.icon.is_none()
            ))
            .collect::<Vec<_>>(),
        [("steel", true, true), ("oil", true, true)]
    );
    assert_eq!(reply.series[0].runs[0].last_hour, 101);
    assert!(
        reply
            .warnings
            .iter()
            .any(|warning| warning.starts_with("Resource names and icons are unavailable"))
    );
    assert_eq!(load(&params).series, reply.series);
    let other = load(&CountryParams {
        tag: "XXX".into(),
        ..params
    });
    assert!(other.series.is_empty());
    assert!(
        other
            .warnings
            .iter()
            .any(|warning| warning.starts_with("XXX:"))
    );
}

#[test]
fn resource_pools_and_deals_keep_their_own_values_when_the_other_table_is_missing() {
    use crate::products::resource_balance::scan;
    let deal = [(0x33, 21, 9001, 800_000)];
    let missing = |mut frame: DecodedFrame, table: TableId| {
        *frame.tables[table.index()] = TableView::Unavailable {
            reason: "fixture".into(),
        };
        frame
    };
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                resource_frame(100, 30_100_000, 9001, &deal),
                // The deals were not captured at this hour: the pools still count,
                // and the deals are unknown, not none.
                missing(
                    resource_frame(101, 30_100_000, 9001, &deal),
                    TableId::ResourceExchanges,
                ),
                resource_frame(102, 30_100_000, 9001, &deal),
                // The pools were not captured at this hour: the deals still count.
                missing(
                    resource_frame(103, 30_100_000, 9001, &deal),
                    TableId::CountryResources,
                ),
                resource_frame(104, 30_100_000, 9001, &deal),
            ],
        )],
    )
    .unwrap();
    let scanned = scan(&replay, 1, "GER", &AtomicBool::new(false)).unwrap();
    let spans = |spans: &[crate::products::country_groups::HourSpan]| {
        spans
            .iter()
            .map(|span| (span.first_hour, span.last_hour))
            .collect::<Vec<_>>()
    };
    assert_eq!(spans(&scanned.deal_hours), [(100, 100), (102, 104)]);
    let steel: Vec<_> = scanned.series[&1]
        .iter()
        .map(|run| (run.first_hour, run.last_hour, run.starts_segment))
        .collect();
    assert_eq!(steel, [(100, 102, true), (104, 104, true)]);
    let deals: Vec<_> = scanned
        .trades
        .iter()
        .map(|trade| (trade.first_hour, trade.last_hour, trade.starts_segment))
        .collect();
    assert_eq!(deals, [(100, 100, true), (102, 104, true)]);
}

#[test]
fn a_repeated_resource_token_is_its_own_resource_and_never_blanks_the_others() {
    use crate::products::{
        CountryParams, LoadContext, Product,
        resource_balance::{ResourceBalance, ordinals},
    };
    use replay::rows::{CountryResourceRow, ResourceDefinitionRow};
    // Resource 3 repeats resource 1's token, as a second declaration in content would.
    let repeated = |hour| {
        let mut frame = resource_frame(hour, 30_100_000, 9001, &[]);
        if let TableView::Complete(TableRows::ResourceDefinitions(rows)) =
            &mut *frame.tables[TableId::ResourceDefinitions.index()]
        {
            rows.push(ResourceDefinitionRow {
                id: 3,
                token: 501,
                active: 1,
                factory_cost: 12_500,
            });
        }
        if let TableView::Complete(TableRows::CountryResources(rows)) =
            &mut *frame.tables[TableId::CountryResources.index()]
        {
            rows.insert(
                2,
                CountryResourceRow {
                    country: 11,
                    resource: 3,
                    produced: 700_000,
                    ..Default::default()
                },
            );
        }
        frame
    };
    let replay = Arc::new(
        Replay::from_records(
            CAMPAIGN.into(),
            &[records(1, vec![repeated(100), repeated(101)])],
        )
        .unwrap(),
    );
    let source = crate::Source::from_replay(
        crate::SourceKey {
            source_id: "repeated".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        replay,
        &AtomicBool::new(false),
    )
    .unwrap();
    let reply = ResourceBalance::load(
        &source,
        &CountryParams {
            branch_id: "1".into(),
            tag: "GER".into(),
        },
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        reply
            .resources
            .iter()
            .map(|resource| (resource.id, resource.token.as_str(), resource.ordinal))
            .collect::<Vec<_>>(),
        [(1, "steel", 1), (2, "oil", 1), (3, "steel", 2)]
    );
    // Each keeps its own history; none overlaps another's.
    assert_eq!(
        reply
            .series
            .iter()
            .map(|series| (
                series.id,
                series.runs.len(),
                series.runs[0].produced_raw.as_str()
            ))
            .collect::<Vec<_>>(),
        [(1, 1, "30100000"), (2, 1, "0"), (3, 1, "700000")]
    );
    assert!(
        reply
            .warnings
            .iter()
            .any(|warning| warning.starts_with("steel is declared 2 times"))
    );
    assert_eq!(
        ordinals(&[
            (1, "a".into()),
            (2, "b".into()),
            (5, "a".into()),
            (9, "a".into())
        ]),
        [1, 1, 2, 3]
    );
}

#[test]
fn unknown_decode_settings_keep_resource_names_and_refuse_icons_by_token() {
    let (mut catalogue, _folder) = crate::catalogue::tests::folder_catalogue(&[
        (
            "interface/test.gui",
            "guiTypes = { containerWindowType = { name = resources_info_entry buttonType = { name = resources_icon spriteType = icon } } }",
        ),
        (
            "interface/test.gfx",
            "spriteTypes = { spriteType = { name = icon texturefile = gfx/icon.dds } }",
        ),
        (
            "common/resources/test.txt",
            "resources = { ore = { icon_frame = 1 } }",
        ),
        (
            "localisation/test_l_english.yml",
            "\u{feff}l_english:\n PRODUCTION_MATERIALS_ORE:0 \"Ore\"\n",
        ),
    ]);
    catalogue.guis = installed_content::gui::native::read_layers(&catalogue.content.layers).unwrap();
    catalogue.gui_decode = Err("settings.txt is unavailable".into());
    let mut cached = Default::default();
    let mut warnings = Vec::new();
    super::resource_content(
        &catalogue,
        &[("ore".into(), 1)],
        &mut cached,
        &mut warnings,
        &std::sync::atomic::AtomicBool::new(false),
    )
    .unwrap();
    let declared = &cached[&("ore".into(), 1)];
    assert_eq!(declared.name.as_deref(), Some("Ore"));
    assert!(declared.icon.is_none());
    assert!(
        warnings
            .iter()
            .any(|why| why.contains("ore: icon unavailable: settings.txt")),
        "{warnings:?}"
    );
}
