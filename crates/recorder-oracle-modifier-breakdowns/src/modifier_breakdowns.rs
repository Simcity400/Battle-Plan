use super::*;

pub fn balance(
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
                .map(|name| content_reader_parser::parser::c_string(name))
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
    report.limitation("the cached balance modifier trees are runtime-only: playback derives them from the ordered static modifier names the save writes, and `recorder derived` compares that derivation with native witness trees", "PowerBalanceModifierBreakdowns");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::*;
    use replay_frame::frame::Metrics;

    #[test]
    fn saved_balance_order_is_checked() {
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 0,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|i| {
                TableView::Complete(TableRows::empty(TableId::ALL[i])).into()
            }),
        };
        let pack = save::governance::parse(&content_reader_parser::parser::parse(b"countries={} power_balance={power_balances={{template=x value=0 left_side=l right_side=r trending_side=r modifier=b modifier=a modifier=b}}}").unwrap().items).unwrap();
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
