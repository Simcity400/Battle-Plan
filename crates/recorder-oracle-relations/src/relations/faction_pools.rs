use super::*;

pub fn projection(frame: &DecodedFrame, save: &Save, report: &mut Report) -> io::Result<()> {
    let rows = complete::<tables::FactionProjection>(frame)?;
    let mut owners = BTreeMap::new();
    for row in rows {
        if row.faction == 0 || owners.insert(row.faction, row).is_some() {
            return Err(invalid("invalid faction projection owner"));
        }
    }
    for (&id, saved) in &save.factions {
        let row = owners
            .remove(&id)
            .ok_or_else(|| invalid("faction projection owner missing"))?;
        report.scalar(
            &format!("faction {id}"),
            "power_projection_from_effects",
            (row.effects_raw != 0).then_some(row.effects_raw),
            saved.companions.projection_effects.as_ref(),
        );
    }
    if !owners.is_empty() {
        return Err(invalid("faction projection has absent owners"));
    }
    if !rows.is_empty() {
        for field in ["projection_raw", "influence_raw"] {
            report.limitation("faction projection/influence totals are unsaved retained refresh outputs; verify the matching faction screen",format!("FactionProjection.{field}"));
        }
    }
    report.compared("FactionProjection", rows.len());
    Ok(())
}

pub fn resources(frame: &DecodedFrame, save: &Save, report: &mut Report) -> io::Result<()> {
    let rows = complete::<tables::FactionResources>(frame)?;
    let tokens: TokenNames<'_> = complete::<tables::LexicalTokens>(frame)?
        .iter()
        .map(|row| (row.token, row.literal.as_slice()))
        .collect();
    let mut owners: BTreeMap<u64, Vec<&replay_data::rows::FactionResourceRow>> = BTreeMap::new();
    for row in rows {
        let values = owners.entry(row.faction).or_default();
        if row.ordinal as usize != values.len() {
            return Err(invalid("noncontiguous faction resource slots"));
        }
        values.push(row);
    }
    let system = save
        .faction_accounts
        .as_ref()
        .ok_or_else(|| invalid("faction system extracted owner unavailable"))?;
    for (id, expected) in std::iter::once((0, system.extracted.as_ref())).chain(
        save.factions
            .iter()
            .map(|(&id, saved)| (id, saved.companions.extracted.as_ref())),
    ) {
        let expected = expected.ok_or_else(|| invalid("save lacks faction extracted pool"))?;
        let actual = owners.remove(&id).unwrap_or_default();
        let actual = actual
            .into_iter()
            .filter(|row| row.raw != 0)
            .collect::<Vec<_>>();
        let label = if id == 0 {
            "faction_system.extracted".to_owned()
        } else {
            format!("faction {id}.extracted")
        };
        report.scalar_checks += 1;
        if actual.len() != expected.len() {
            report.difference(format!("{label} count"), actual.len(), expected.len());
        }
        for (index, (row, (name, amount))) in actual.into_iter().zip(expected).enumerate() {
            let spelling = tokens
                .get(&row.token)
                .ok_or_else(|| invalid("nonzero faction resource lacks token spelling"))?;
            report.scalar_checks += 1;
            let spelling = content_reader_parser::parser::c_string(spelling);
            if spelling != name {
                report.difference(format!("{label}[{index}] token"), spelling, name);
            }
            report.scalar(
                &label,
                &format!("[{index}] amount"),
                Some(row.raw),
                Some(amount),
            );
        }
    }
    if !owners.is_empty() {
        return Err(invalid("faction resources have absent owners"));
    }
    report.compared("FactionResources", rows.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::FactionResourceRow;
    use replay_data::rows::LexicalTokenRow;
    use replay_data::rows::TableRows;
    use replay_frame::frame::TableView;
    #[test]
    fn faction_resources_compare_system_slot_zero_repeated_tokens_negative_and_omitted_zero() {
        let (mut frame, mut save, _, _) = crate::fixtures::fixture();
        save.faction_accounts = Some(content_reader_save::faction_accounts::Accounts {
            extracted: Some(vec![
                (
                    b"mod_resource".to_vec(),
                    content_reader_save::Decimal::parse(b"-2").unwrap(),
                ),
                (
                    b"mod_resource".to_vec(),
                    content_reader_save::Decimal::parse(b"3").unwrap(),
                ),
            ]),
            ..Default::default()
        });
        *frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
                token: 9,
                literal: b"mod_resource".to_vec(),
            }]));
        let rows = vec![
            FactionResourceRow {
                faction: 0,
                ordinal: 0,
                token: 9,
                raw: -200000,
            },
            FactionResourceRow {
                faction: 0,
                ordinal: 1,
                token: 0,
                raw: 0,
            },
            FactionResourceRow {
                faction: 0,
                ordinal: 2,
                token: 9,
                raw: 300000,
            },
        ];
        *frame.tables[TableId::FactionResources.index()] =
            TableView::Complete(TableRows::FactionResources(rows.clone()));
        let mut report = Report::default();
        resources(&frame, &save, &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        let mut changed = rows;
        changed[2].raw += 100000;
        *frame.tables[TableId::FactionResources.index()] =
            TableView::Complete(TableRows::FactionResources(changed));
        resources(&frame, &save, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
    }
}
