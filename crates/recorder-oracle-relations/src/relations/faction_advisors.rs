use super::*;
pub fn compare(
    frame: &DecodedFrame,
    save: &Save,
    countries: &CountryContext<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let rows = complete::<tables::FactionAdvisorModifiers>(frame)?;
    let tokens: TokenNames<'_> = complete::<tables::LexicalTokens>(frame)?
        .iter()
        .map(|row| (row.token, row.literal.as_slice()))
        .collect();
    let definitions = complete::<tables::ModifierDefinitions>(frame)?;
    for (&faction, saved) in &save.factions {
        let actual = rows
            .iter()
            .filter(|row| row.faction == faction)
            .collect::<Vec<_>>();
        let headers = actual
            .iter()
            .filter(|row| row.kind == 0)
            .collect::<Vec<_>>();
        report.scalar_checks += 1;
        if headers.len() != saved.companions.advisor_slots.len() {
            report.difference(
                format!("faction {faction}.upgrades.slots size"),
                headers.len(),
                saved.companions.advisor_slots.len(),
            );
        }
        for (ordinal, expected) in saved.companions.advisor_slots.iter().enumerate() {
            let header = headers
                .iter()
                .find(|row| row.slot as usize == ordinal)
                .ok_or_else(|| invalid("saved advisor slot has no recorded header"))?;
            let label = format!("faction {faction}.upgrades.slots[{ordinal}]");
            report.scalar_checks += 3;
            let owner = country_tag(header.country, &countries.links)?;
            if owner != expected.owner.as_deref() {
                report.difference(format!("{label}.owner"), owner, expected.owner.as_deref());
            }
            let advisor = header
                .character
                .map(|handle| {
                    let id = (handle >> 32) as u32;
                    let character = complete::<tables::Characters>(frame)?
                        .iter()
                        .find(|row| row.id == id)
                        .ok_or_else(|| invalid("faction slot advisor character missing"))?;
                    tokens
                        .get(&(character.name_token_id as u32))
                        .copied()
                        .map(content_reader_parser::parser::c_string)
                        .ok_or_else(|| invalid("faction slot advisor token spelling missing"))
                })
                .transpose()?;
            if advisor != expected.advisor.as_deref() {
                report.difference(
                    format!("{label}.advisor"),
                    advisor,
                    expected.advisor.as_deref(),
                );
            }
            let date = (header.date_present != 0).then_some(header.date);
            if date != expected.date {
                report.difference(
                    format!("{label}.spymaster_change_date"),
                    date,
                    expected.date,
                );
            }
            for (kind, name, saved) in [
                (1, "spymaster", &expected.spymaster),
                (3, "modifier", &expected.modifier),
            ] {
                let actual_name = actual
                    .iter()
                    .find(|row| row.slot as usize == ordinal && row.kind == kind)
                    .ok_or_else(|| invalid("faction slot modifier name missing"))?;
                report.scalar_checks += 1;
                if content_reader_parser::parser::c_string(&actual_name.name) != saved.name.as_slice() {
                    report.difference(
                        format!("{label}.{name}.name"),
                        &actual_name.name,
                        &saved.name,
                    );
                }
                let mut actual_entries = Vec::new();
                for row in actual.iter().filter(|row| {
                    row.slot as usize == ordinal && row.kind == kind + 1 && row.raw != 0
                }) {
                    let definition = definitions
                        .iter()
                        .find(|definition| definition.index == row.modifier)
                        .ok_or_else(|| invalid("faction slot modifier definition missing"))?;
                    let key = tokens
                        .get(&definition.token)
                        .ok_or_else(|| invalid("faction slot modifier spelling missing"))?;
                    actual_entries.push((content_reader_parser::parser::c_string(key).to_vec(), row.raw));
                }
                actual_entries.sort_by(|a, b| a.0.cmp(&b.0));
                let mut expected_entries = saved
                    .entries
                    .iter()
                    .map(|(key, value)| (key.clone(), value.raw))
                    .collect::<Vec<_>>();
                expected_entries.sort_by(|a, b| a.0.cmp(&b.0));
                report.scalar_checks += actual_entries.len().max(expected_entries.len()).max(1);
                if actual_entries != expected_entries {
                    report.difference(format!("{label}.{name}"), actual_entries, expected_entries);
                }
            }
        }
    }
    if rows
        .iter()
        .any(|row| !save.factions.contains_key(&row.faction))
    {
        return Err(invalid("faction advisor rows name absent faction"));
    }
    report.compared("FactionAdvisorModifiers", rows.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::*;
    use replay_frame::frame::TableView;
    #[test]
    fn modifier_ownership_oracle_compares_names_and_each_nonzero_source_independently() {
        let (mut frame, mut save, _, _) = crate::fixtures::fixture();
        save.factions.clear();
        let expected = content_reader_save::faction_companions::Modifier {
            name: b"Synthetic".to_vec(),
            entries: vec![(
                b"synthetic_modifier".to_vec(),
                content_reader_save::Decimal::parse(b"-0.25").unwrap(),
            )],
        };
        save.factions.insert(
            7,
            content_reader_save::Faction {
                id: 7,
                name: None,
                members: vec![],
                icon: None,
                template: None,
                color: None,
                color_overridden: None,
                theatres: vec![],
                advisor_slots: vec![None],
                goals: Default::default(),
                companions: content_reader_save::faction_companions::Snapshot {
                    advisor_slots: vec![content_reader_save::faction_companions::AdvisorSlot {
                        modifier: expected.clone(),
                        spymaster: expected,
                        ..Default::default()
                    }],
                    ..Default::default()
                },
            },
        );
        let rows = vec![
            FactionAdvisorModifierRow {
                faction: 7,
                ..Default::default()
            },
            FactionAdvisorModifierRow {
                faction: 7,
                ordinal: 1,
                kind: 1,
                name: b"Synthetic".to_vec(),
                ..Default::default()
            },
            FactionAdvisorModifierRow {
                faction: 7,
                ordinal: 2,
                kind: 2,
                modifier: 9,
                raw: -25000,
                ..Default::default()
            },
            FactionAdvisorModifierRow {
                faction: 7,
                ordinal: 3,
                kind: 3,
                name: b"Synthetic".to_vec(),
                ..Default::default()
            },
            FactionAdvisorModifierRow {
                faction: 7,
                ordinal: 4,
                kind: 4,
                modifier: 9,
                raw: -25000,
                ..Default::default()
            },
        ];
        *frame.tables[TableId::FactionAdvisorModifiers.index()] =
            TableView::Complete(TableRows::FactionAdvisorModifiers(rows.clone()));
        *frame.tables[TableId::ModifierDefinitions.index()] =
            TableView::Complete(TableRows::ModifierDefinitions(vec![
                ModifierDefinitionRow {
                    index: 9,
                    token: 77,
                    ..Default::default()
                },
            ]));
        *frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
                token: 77,
                literal: b"synthetic_modifier".to_vec(),
            }]));
        let countries = super::super::super::country_context(&frame).unwrap();
        let mut report = Report::default();
        compare(&frame, &save, &countries, &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        let mut changed = rows;
        changed[4].raw = 50000;
        *frame.tables[TableId::FactionAdvisorModifiers.index()] =
            TableView::Complete(TableRows::FactionAdvisorModifiers(changed));
        let countries = super::super::super::country_context(&frame).unwrap();
        compare(&frame, &save, &countries, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
    }
}
