use super::*;
use crate::products::test_support::*;
use replay_data::rows::*;

fn base(hour: i32) -> DecodedFrame {
    let mut at = frame(hour);
    countries(&mut at, &[(1, 10, "IMP"), (2, 20, "EXP")]);
    at
}

fn ask(state: &mut Prepared, view: ConvoyView, through: i32) -> ConvoyHealthReply {
    let fixture = fixture();
    let flag = AtomicBool::new(false);
    let mut writer = fixture.writer();
    state.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    let regions = BTreeMap::from([(7, "Synthetic Sea".to_owned())]);
    query(
        &fixture.store,
        &fixture.key,
        &ConvoyHealthParams {
            branch_id: "1".into(),
            view,
            tag: "IMP".into(),
        },
        Some(through),
        &regions,
        &|list| {
            (
                list.iter().map(|(id, token)| (*id, format!("Name of {token}"))).collect(),
                Vec::new(),
            )
        },
        Vec::new(),
        &flag,
    )
    .unwrap()
}

#[test]
fn regional_efficiency_keeps_unobserved_regions_absent_and_names_through_content() {
    let mut state = Prepared::default();
    for (hour, rows) in [
        (5, vec![(7, 100_000), (8, 40_000)]),
        // Region 8 is no longer on a bad route: not observed, not full.
        (6, vec![(7, 90_000)]),
    ] {
        let mut at = base(hour);
        set(
            &mut at,
            TableRows::RegionalConvoyEfficiency(
                rows.into_iter()
                    .map(|(region, efficiency)| RegionalConvoyEfficiencyRow {
                        country: 1,
                        region,
                        efficiency,
                    })
                    .collect(),
            ),
        );
        state.step(&at);
    }
    let reply = ask(&mut state, ConvoyView::Regions, 6);
    assert_eq!(reply.regions.len(), 2);
    assert_eq!(reply.regions[0].name.as_deref(), Some("Synthetic Sea"));
    assert_eq!(
        reply.regions[0].efficiency,
        vec![EconomyRun(5, 5, "100000".into()), EconomyRun(6, 6, "90000".into())]
    );
    assert_eq!(reply.regions[1].efficiency, vec![EconomyRun(5, 5, "40000".into())]);
    assert!(reply.warnings.iter().any(|line| line.contains("ids are shown")));
    assert_eq!(reply.known[0], vec![EconomyRun(5, 6, "1".into())]);
}

#[test]
fn each_deal_keeps_its_own_truncated_product_and_lend_lease_has_no_efficiency() {
    let mut state = Prepared::default();
    let mut at = base(5);
    set(
        &mut at,
        TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 3,
            literal: b"synthetic_ore".to_vec(),
        }]),
    );
    set(
        &mut at,
        TableRows::ResourceDefinitions(vec![ResourceDefinitionRow {
            id: 0,
            token: 3,
            active: 1,
            factory_cost: 0,
        }]),
    );
    set(
        &mut at,
        TableRows::ResourceExchanges(vec![
            ResourceExchangeRow {
                id: 1,
                exporter: 2,
                importer: 1,
                efficiency_a: 99_999,
                efficiency_b: 50_001,
                convoys_assigned: 3,
                convoys_required: 5,
                ..Default::default()
            },
            ResourceExchangeRow {
                id: 2,
                exporter: 2,
                importer: 1,
                efficiency_a: -100_000,
                efficiency_b: 3,
                ..Default::default()
            },
        ]),
    );
    set(
        &mut at,
        TableRows::ConvoyClients(vec![ConvoyClientRow {
            handle: 9,
            category: 4,
            owner: 2,
            other: 1,
            bad: true,
            convoys_assigned: 1,
            convoys_required: 4,
            ..Default::default()
        }]),
    );
    state.step(&at);
    let reply = ask(&mut state, ConvoyView::Trade, 5);
    assert_eq!(reply.deals.len(), 2);
    // 99999 * 50001 / 100000, truncated.
    assert_eq!(reply.deals[0].efficiency[0].2, "50000");
    // Signed products truncate toward zero.
    assert_eq!(reply.deals[1].efficiency[0].2, "-3");
    assert_eq!(reply.deals[0].exporter[0].2, "20");
    assert_eq!(reply.deals[0].convoys[1][0].2, "5");
    assert_eq!(reply.resources[0].name.as_deref(), Some("Name of synthetic_ore"));
    let lease = &reply.leases[0];
    assert_eq!(lease.giver[0].2, "20");
    assert_eq!(lease.bad[0].2, "1");
    assert_eq!(lease.convoys[1][0].2, "4");
}

#[test]
fn a_row_without_its_holder_leaves_the_table_unknown_and_an_unknown_partner_keeps_the_row() {
    let mut state = Prepared::default();
    // Raw country 9 has no identity: the partner at hour 5, the deal's importer at hour 6.
    for (hour, importer) in [(5, 1), (6, 9)] {
        let mut at = base(hour);
        set(
            &mut at,
            TableRows::ResourceExchanges(vec![ResourceExchangeRow {
                id: 1,
                exporter: 9,
                importer,
                efficiency_a: 100_000,
                efficiency_b: 100_000,
                convoys_assigned: 3,
                convoys_required: 5,
                ..Default::default()
            }]),
        );
        set(
            &mut at,
            TableRows::ConvoyClients(vec![ConvoyClientRow {
                handle: 4,
                category: 4,
                owner: 9,
                other: 1,
                bad: false,
                ..Default::default()
            }]),
        );
        state.step(&at);
    }
    let reply = ask(&mut state, ConvoyView::Trade, 6);
    assert_eq!(reply.deals.len(), 1);
    assert!(reply.deals[0].exporter.is_empty());
    assert_eq!(reply.deals[0].efficiency, vec![EconomyRun(5, 5, "100000".into())]);
    assert_eq!(reply.deals[0].convoys[1], vec![EconomyRun(5, 5, "5".into())]);
    // Hour 6's deal belongs to no known country, so no deal at hour 6 is proven absent.
    assert_eq!(reply.known[1], vec![EconomyRun(5, 5, "1".into())]);
    assert_eq!(reply.leases.len(), 1);
    assert!(reply.leases[0].giver.is_empty());
    assert_eq!(reply.leases[0].bad, vec![EconomyRun(5, 6, "0".into())]);
    assert_eq!(reply.known[2], vec![EconomyRun(5, 6, "1".into())]);
    assert!(reply.warnings.iter().any(|line| line.contains("no identity")));
}

#[test]
fn a_resource_spelled_only_by_a_later_dictionary_is_learned_by_changes() {
    let mut state = Prepared::default();
    for (hour, spelled) in [(5, false), (6, true)] {
        let mut at = base(hour);
        if spelled {
            set(
                &mut at,
                TableRows::LexicalTokens(vec![LexicalTokenRow {
                    token: 3,
                    literal: b"synthetic_ore".to_vec(),
                }]),
            );
        }
        set(
            &mut at,
            TableRows::ResourceDefinitions(vec![ResourceDefinitionRow {
                id: 0,
                token: 3,
                active: 1,
                factory_cost: 0,
            }]),
        );
        let mut changes = FrameChanges {
            rows_changed: [hour == 5; TABLE_COUNT],
            table_read: [true; TABLE_COUNT],
            slot_read: [true; TABLE_COUNT],
            updates: None,
        };
        // Only the dictionary changes at the second hour.
        changes.rows_changed[TableId::LexicalTokens.index()] = true;
        state.observe_changes(&at, &changes).unwrap();
    }
    let reply = ask(&mut state, ConvoyView::Trade, 6);
    assert_eq!(reply.resources.len(), 1);
    assert_eq!(reply.resources[0].token, "synthetic_ore");
}
