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

/// The hour `hour` of a scenario: a deal's convoys and a region's efficiency move, a deal names
/// an unknown exporter, a region's country has no identity, the regional table is unavailable at
/// 8, a deal goes at 9, a second country of the importer's slot appears at 10 and names the same
/// region at 11 and 12, and a lend-lease route turns bad at 11.
fn scenario(hour: i32) -> DecodedFrame {
    let mut at = frame(hour);
    let mut holders = vec![(1, 10, "IMP"), (2, 20, "EXP")];
    if hour >= 10 {
        holders.push((3, 10, "IMP"));
    }
    countries(&mut at, &holders);
    let mut regional = vec![
        (1, 7, if hour >= 6 { 90_000 } else { 100_000 }),
        (1, 8, 40_000),
    ];
    if hour >= 7 {
        regional.push((50, 7, 10_000));
    }
    if (11..=12).contains(&hour) {
        regional.push((3, 7, 20_000 + i64::from(hour)));
    }
    regional.sort_by_key(|&(country, region, _)| (country, region));
    if hour == 8 {
        unavailable(&mut at, TableId::RegionalConvoyEfficiency);
    } else {
        set(
            &mut at,
            TableRows::RegionalConvoyEfficiency(
                regional
                    .into_iter()
                    .map(|(country, region, efficiency)| RegionalConvoyEfficiencyRow {
                        country,
                        region,
                        efficiency,
                    })
                    .collect(),
            ),
        );
    }
    let mut deals = Vec::new();
    if hour < 9 {
        deals.push(ResourceExchangeRow {
            id: 1,
            exporter: 2,
            importer: 1,
            resource: 0,
            convoys_assigned: if hour >= 6 { 3 } else { 2 },
            convoys_required: 5,
            efficiency_a: 80_000,
            efficiency_b: 90_000,
            delivered: i64::from(hour),
            ..Default::default()
        });
    }
    if hour >= 7 {
        deals.push(ResourceExchangeRow {
            id: 2,
            exporter: 99,
            importer: 1,
            resource: 0,
            convoys_assigned: 1,
            convoys_required: 1,
            efficiency_a: 100_000,
            efficiency_b: 100_000,
            ..Default::default()
        });
    }
    set(&mut at, TableRows::ResourceExchanges(deals));
    set(
        &mut at,
        TableRows::ConvoyClients(vec![ConvoyClientRow {
            handle: 9,
            category: LEND_LEASE,
            owner: 2,
            other: 1,
            bad: hour >= 11,
            convoys_assigned: 4,
            convoys_required: 4,
            ..Default::default()
        }]),
    );
    at
}

/// What `table` changed from `before` to `now` in rows, as an operator hands it on: whole where
/// either is not complete, else the keys written and deleted.
fn edit<T: replay_data::rows::TableSchema>(before: &DecodedFrame, now: &DecodedFrame) -> Option<(TableId, Edit)>
where
    T::Row: RowCodec + PartialEq,
{
    let (Ok(old), Ok(new)) = (before.complete::<T>(), now.complete::<T>()) else {
        return Some((T::ID, Edit::Whole));
    };
    let held: BTreeMap<u128, &T::Row> = old.iter().map(|row| (row.key(), row)).collect();
    let current: BTreeMap<u128, &T::Row> = new.iter().map(|row| (row.key(), row)).collect();
    let written: Vec<u128> = current
        .iter()
        .filter(|(key, row)| held.get(key) != Some(row))
        .map(|(&key, _)| key)
        .collect();
    let deleted: Vec<u128> = held
        .keys()
        .filter(|key| !current.contains_key(key))
        .copied()
        .collect();
    (!written.is_empty() || !deleted.is_empty()).then_some((T::ID, Edit::Rows { written, deleted }))
}

/// Reading only each hour's changed rows publishes what reading the census whole every hour
/// does, through value changes, an unavailable table, a deleted row, a holder change and two
/// countries of one slot naming one field.
#[test]
fn reading_only_changed_rows_publishes_what_a_whole_read_does() {
    // Each reply publishes its state: every view replays the hours.
    for view in [ConvoyView::Regions, ConvoyView::Trade] {
        let mut whole = Prepared::default();
        let mut changed = Prepared::default();
        let mut before: Option<DecodedFrame> = None;
        for hour in 5..=13 {
            let at = scenario(hour);
            whole.step(&at);
            let facts = FrameChanges::alone(&at, replay_data::format::FormatVersion::CURRENT);
            let edits: Option<Vec<(TableId, Edit)>> = before.as_ref().map(|before| {
                let mut edits = Vec::new();
                if before.complete::<tables::Countries>().ok()
                    != at.complete::<tables::Countries>().ok()
                    || before.complete::<tables::CountryLinks>().ok()
                        != at.complete::<tables::CountryLinks>().ok()
                {
                    edits.push((TableId::Countries, Edit::Whole));
                }
                edits.extend(edit::<tables::RegionalConvoyEfficiency>(before, &at));
                edits.extend(edit::<tables::ResourceExchanges>(before, &at));
                edits.extend(edit::<tables::ConvoyClients>(before, &at));
                edits
            });
            changed.observe_edits(&at, &facts, edits.as_deref()).unwrap();
            before = Some(at);
        }
        assert_eq!(
            serde_json::to_value(ask(&mut changed, view, 13)).unwrap(),
            serde_json::to_value(ask(&mut whole, view, 13)).unwrap(),
            "{view:?}"
        );
    }
}
