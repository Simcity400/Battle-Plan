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
            ..Default::default()
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
