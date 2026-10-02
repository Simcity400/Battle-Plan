use super::*;

pub(super) fn reserve(frame: &DecodedFrame, report: &mut Report) -> io::Result<()> {
    let rows = complete::<tables::ReserveModifierBreakdowns>(frame)?;
    let units: BTreeMap<_, _> = complete::<tables::Divisions>(frame)?
        .iter()
        .map(|r| (r.id, r))
        .collect();
    let mut expected = BTreeSet::new();
    for row in complete::<tables::LandCombatDivisions>(frame)?
        .iter()
        .filter(|r| r.role == 2)
    {
        let unit = units
            .get(&row.division)
            .ok_or_else(|| invalid("reserve breakdown division is absent from Divisions"))?;
        let country = if unit.expeditionary_owner as i32 > 0 {
            unit.expeditionary_owner
        } else {
            unit.roster_owner
        };
        expected.insert((0, country as u64));
        if row.context.leader_modifier_kind != 0 {
            expected.insert((row.context.leader_modifier_kind, row.commander));
        }
    }
    let actual: BTreeSet<_> = rows.iter().map(|r| (r.kind, r.owner)).collect();
    if actual != expected {
        report.difference("ReserveModifierBreakdowns sources", actual, expected);
    }
    report.compared("ReserveModifierBreakdowns", rows.len());
    report.limitation("cached reserve modifier tree structure, display bytes and getter values are runtime-only; the oracle checks the combat source joins", "ReserveModifierBreakdowns");
    Ok(())
}

pub(super) fn balance(
    frame: &DecodedFrame,
    pack: &save::governance::Pack,
    report: &mut Report,
) -> io::Result<()> {
    let rows = complete::<tables::PowerBalanceModifierBreakdowns>(frame)?;
    let saved = pack.power_balances.as_deref().unwrap_or_default();
    let records: BTreeSet<_> = rows.iter().map(|r| r.record).collect();
    let expected: BTreeSet<_> = (0..saved.len()).map(|i| i as u32).collect();
    if records != expected {
        report.difference("PowerBalanceModifierBreakdowns records", records, expected);
    }
    for row in rows {
        if let Some(saved) = saved.get(row.record as usize) {
            let names: Vec<_> = row
                .sources
                .iter()
                .map(|name| installed_content::parser::c_string(name))
                .collect();
            if names
                .iter()
                .zip(&row.sources)
                .any(|(saved, raw)| saved.len() != raw.len())
            {
                report.limitation("native text serialization ends at the first NUL; captured trailing bytes are not save-comparable", "PowerBalanceModifierBreakdowns");
            }
            let expected: Vec<_> = saved.modifiers.iter().map(Vec::as_slice).collect();
            if names != expected {
                report.difference(
                    format!("PowerBalanceModifierBreakdowns {} source order", row.record),
                    names,
                    expected,
                );
            }
        }
    }
    report.compared("PowerBalanceModifierBreakdowns", rows.len());
    report.limitation("cached balance modifier child structure and getter values are runtime-only; the save writes the ordered static modifier names", "PowerBalanceModifierBreakdowns");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::{frame::Metrics, rows::*};

    #[test]
    fn source_joins_and_saved_balance_order_are_checked() {
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 0,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|i| {
                TableView::Complete(TableRows::empty(TableId::ALL[i])).into()
            }),
        };
        *frame.tables[TableId::LandCombatDivisions.index()] =
            TableView::Complete(TableRows::LandCombatDivisions(vec![
                LandCombatDivisionRow {
                    division: 7,
                    role: 2,
                    commander: 9,
                    context: LandDivisionContext {
                        leader_modifier_kind: 2,
                        ..Default::default()
                    },
                    ..Default::default()
                },
            ]));
        let mut report = Report::default();
        for (expeditionary_owner, country) in [(11, 11), (0, 12), (u32::MAX, 12)] {
            *frame.tables[TableId::Divisions.index()] =
                TableView::Complete(TableRows::Divisions(vec![DivisionRow {
                    id: 7,
                    roster_owner: 12,
                    logical_owner: 10,
                    expeditionary_owner,
                    ..Default::default()
                }]));
            *frame.tables[TableId::ReserveModifierBreakdowns.index()] =
                TableView::Complete(TableRows::ReserveModifierBreakdowns(
                    [(0, country), (2, 9)]
                        .into_iter()
                        .map(|(kind, owner)| ReserveModifierBreakdownRow {
                            kind,
                            owner,
                            nodes: vec![ModifierBreakdownNode::default()],
                        })
                        .collect(),
                ));
            reserve(&frame, &mut report).unwrap();
            assert!(report.differences.is_empty());
        }
        *frame.tables[TableId::ReserveModifierBreakdowns.index()] =
            TableView::Complete(TableRows::ReserveModifierBreakdowns(Vec::new()));
        reserve(&frame, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);

        let pack = save::governance::parse(&installed_content::parser::parse(b"countries={} power_balance={power_balances={{template=x value=0 left_side=l right_side=r trending_side=r modifier=b modifier=a modifier=b}}}").unwrap().items).unwrap();
        let row = |sources| PowerBalanceModifierBreakdownRow {
            record: 0,
            sources,
            daily: vec![ModifierBreakdownNode::default()],
            weekly: vec![ModifierBreakdownNode::default()],
        };
        *frame.tables[TableId::PowerBalanceModifierBreakdowns.index()] =
            TableView::Complete(TableRows::PowerBalanceModifierBreakdowns(vec![row(vec![
                b"b\0tail".to_vec(),
                b"a".to_vec(),
                b"b".to_vec(),
            ])]));
        let mut report = Report::default();
        balance(&frame, &pack, &mut report).unwrap();
        assert!(report.differences.is_empty());
        assert_eq!(report.limitations.len(), 2);
        *frame.tables[TableId::PowerBalanceModifierBreakdowns.index()] =
            TableView::Complete(TableRows::PowerBalanceModifierBreakdowns(vec![row(vec![
                b"a".to_vec(),
                b"b".to_vec(),
            ])]));
        balance(&frame, &pack, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
    }
}
