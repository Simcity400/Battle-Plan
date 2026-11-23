use super::*;
use content_reader_save::construction::Manpower;
use replay_data::rows::{FactionManpowerRow, FactionProgramRow, FactionRuleRow};
fn group<T>(
    rows: &[T],
    owner: impl Fn(&T) -> u64,
    ordinal: impl Fn(&T) -> u32,
) -> io::Result<BTreeMap<u64, Vec<&T>>> {
    let mut groups: BTreeMap<u64, Vec<&T>> = BTreeMap::new();
    for row in rows {
        let owner = owner(row);
        if owner == 0 {
            return Err(invalid("null faction companion owner"));
        }
        let values = groups.entry(owner).or_default();
        if ordinal(row) as usize != values.len() {
            return Err(invalid("noncontiguous faction companion ordinals"));
        }
        values.push(row);
    }
    Ok(groups)
}
fn compare<T: std::fmt::Debug + PartialEq>(
    actual: Vec<T>,
    expected: &[T],
    owner: u64,
    field: &str,
    report: &mut Report,
) {
    report.scalar_checks += actual.len().max(expected.len()).max(1);
    if actual != expected {
        report.difference(format!("faction {owner}.{field}"), actual, expected);
    }
}
pub fn variables(frame: &DecodedFrame, save: &Save, report: &mut Report) -> io::Result<()> {
    let rows = complete::<tables::FactionVariables>(frame)?;
    let mut groups = group(rows, |row| row.faction, |row| row.ordinal)?;
    for (&id, saved) in &save.factions {
        let actual = groups
            .remove(&id)
            .ok_or_else(|| invalid("faction variable owner is missing"))?;
        let seed = actual
            .first()
            .and_then(|row| row.seed)
            .ok_or_else(|| invalid("faction variable seed occurrence missing"))?;
        let entries = &actual[1..];
        let empty = content_reader_save::intel::Variables::default();
        let expected = saved.companions.variables.as_ref().unwrap_or(&empty);
        if let Some(expected_seed) = expected.seed {
            compare(vec![seed], &[expected_seed], id, "variables.seed", report);
        } else if seed != 1 {
            report.difference(format!("faction {id}.variables.seed"), seed, 1);
        }
        let mut values: BTreeMap<Vec<u8>, Vec<i64>> = BTreeMap::new();
        for entry in entries {
            if entry.name.contains(&0) || entry.name == b"random" {
                return Err(invalid(
                    "faction variable literal serialization is unproved for interior NUL or reserved random key",
                ));
            }
            values
                .entry(entry.name.clone())
                .or_default()
                .push(entry.value_raw);
        }
        let mut saved_values: BTreeMap<Vec<u8>, Vec<i64>> = BTreeMap::new();
        for entry in &expected.entries {
            saved_values
                .entry(entry.name.clone())
                .or_default()
                .push(entry.value.raw);
        }
        if expected.reserved_entries != 0 {
            return Err(invalid(
                "faction variable save contains ambiguous random entries",
            ));
        }
        for value in values.values_mut().chain(saved_values.values_mut()) {
            value.sort_unstable();
        }
        report.scalar_checks += entries.len().max(expected.entries.len()).max(1);
        if values != saved_values {
            report.difference(format!("faction {id}.variables"), values, saved_values);
        }
    }
    if !groups.is_empty() {
        return Err(invalid("faction variable rows have absent owners"));
    }
    report.compared("FactionVariables", rows.len());
    Ok(())
}
pub fn rules(frame: &DecodedFrame, save: &Save, report: &mut Report) -> io::Result<()> {
    let rows = complete::<tables::FactionRules>(frame)?;
    let mut groups = group(rows, |row: &FactionRuleRow| row.faction, |row| row.ordinal)?;
    let tokens: TokenNames<'_> = complete::<tables::LexicalTokens>(frame)?
        .iter()
        .map(|row| (row.token, row.literal.as_slice()))
        .collect();
    for (&id, saved) in &save.factions {
        let actual = groups
            .remove(&id)
            .unwrap_or_default()
            .into_iter()
            .map(|row| {
                tokens
                    .get(&row.token)
                    .map(|literal| content_reader_parser::parser::c_string(literal).to_vec())
                    .ok_or_else(|| invalid("faction rule token is absent from LexicalTokens"))
            })
            .collect::<io::Result<Vec<_>>>()?;
        compare(
            actual,
            &saved.companions.rules,
            id,
            "rule_status.rules",
            report,
        );
    }
    for id in groups.keys() {
        report.difference(format!("faction {id}.rule_status"), "present", "absent");
    }
    report.compared("FactionRules", rows.len());
    Ok(())
}
pub fn programmes(frame: &DecodedFrame, save: &Save, report: &mut Report) -> io::Result<()> {
    let rows = complete::<tables::FactionPrograms>(frame)?;
    let mut groups = group(
        rows,
        |row: &FactionProgramRow| row.faction,
        |row| row.ordinal,
    )?;
    for (&id, saved) in &save.factions {
        let actual = groups
            .remove(&id)
            .unwrap_or_default()
            .into_iter()
            .map(|row| row.reference)
            .collect();
        compare(
            actual,
            &saved.companions.programmes,
            id,
            "faction_programs",
            report,
        );
    }
    for id in groups.keys() {
        report.difference(
            format!("faction {id}.faction_programs"),
            "present",
            "absent",
        );
    }
    report.compared("FactionPrograms", rows.len());
    Ok(())
}
pub fn manpower(
    frame: &DecodedFrame,
    save: &Save,
    countries: &CountryContext<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let rows = complete::<tables::FactionManpower>(frame)?;
    let mut groups = group(
        rows,
        |row: &FactionManpowerRow| row.faction,
        |row| row.ordinal,
    )?;
    for (&id, saved) in &save.factions {
        let actual = groups
            .remove(&id)
            .unwrap_or_default()
            .into_iter()
            .filter(|row| row.value > 0)
            .map(|row| {
                let tag = if row.country == 0 {
                    let [reserved] = complete::<tables::ReservedCountryNaming>(frame)? else {
                        return Err(invalid(
                            "faction manpower raw zero requires one reserved naming row",
                        ));
                    };
                    reserved.tag.as_slice()
                } else {
                    required_country_tag(row.country, &countries.links)?
                };
                Ok(Manpower {
                    source_country: tag.to_vec(),
                    amount: row.value,
                })
            })
            .collect::<io::Result<Vec<_>>>()?;
        compare(
            actual,
            &saved.companions.manpower,
            id,
            "manpower_pool",
            report,
        );
    }
    for id in groups.keys() {
        report.difference(format!("faction {id}.manpower_pool"), "present", "absent");
    }
    report.compared("FactionManpower", rows.len());
    Ok(())
}

pub fn unlocks(frame: &DecodedFrame, save: &Save, report: &mut Report) -> io::Result<()> {
    let rows = complete::<tables::FactionUnlocks>(frame)?;
    let mut actual = BTreeMap::new();
    for row in rows {
        if row.faction == 0
            || actual
                .insert(
                    row.faction,
                    (Some(row.research != 0), Some(row.manpower != 0)),
                )
                .is_some()
        {
            return Err(invalid("invalid faction unlock owner"));
        }
    }
    for (&id, saved) in &save.factions {
        report.scalar_checks += 2;
        let expected = (
            saved.companions.research,
            saved.companions.manpower_unlocked,
        );
        let value = actual.remove(&id);
        if value != Some(expected) {
            report.difference(format!("faction {id}.unlocks"), value, expected);
        }
    }
    for id in actual.keys() {
        report.difference(format!("faction {id}.unlocks"), "present", "absent");
    }
    report.compared("FactionUnlocks", rows.len());
    Ok(())
}
pub fn research(
    frame: &DecodedFrame,
    save: &Save,
    countries: &CountryContext<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let rows = complete::<tables::FactionResearch>(frame)?;
    let mut owners = group(rows, |row| row.faction, |row| row.ordinal)?;
    for (&id, saved) in &save.factions {
        let occurrences = owners
            .remove(&id)
            .ok_or_else(|| invalid("faction research owner missing"))?;
        let row = occurrences
            .first()
            .ok_or_else(|| invalid("faction research header missing"))?;
        let bonuses = occurrences
            .iter()
            .skip(1)
            .map(|row| row.bonus_raw)
            .collect::<Vec<_>>();
        let tags = row
            .countries
            .iter()
            .map(|raw| {
                if *raw == 0 {
                    let [reserved] = complete::<tables::ReservedCountryNaming>(frame)? else {
                        return Err(invalid("faction research country zero naming unavailable"));
                    };
                    Ok(reserved.tag.clone())
                } else {
                    Ok(required_country_tag(*raw, &countries.links)?.to_vec())
                }
            })
            .collect::<io::Result<Vec<_>>>()?;
        compare(
            tags,
            &saved.companions.sharing_countries,
            id,
            "upgrades.tech_sharing_group.countries",
            report,
        );
        report.scalar_checks += 1;
        if bonuses.len() != saved.companions.sharing_bonuses.len() {
            report.difference(
                format!("faction {id}.research bonus count"),
                bonuses.len(),
                saved.companions.sharing_bonuses.len(),
            );
        }
        for (index, (actual, expected)) in bonuses
            .iter()
            .zip(&saved.companions.sharing_bonuses)
            .enumerate()
        {
            report.scalar(
                &format!("faction {id}"),
                &format!("research bonus[{index}]"),
                Some(*actual),
                Some(expected),
            );
        }
    }
    if !owners.is_empty() {
        return Err(invalid("faction research contains absent owners"));
    }
    if !rows.is_empty() {
        report.limitation("faction scientist base is an unsaved retained sharing cache; verify the matching research sharing screen","FactionResearch.scientist_base_raw");
    }
    report.compared("FactionResearch", rows.len());
    Ok(())
}

pub fn minimum_upgrades(frame: &DecodedFrame, save: &Save, report: &mut Report) -> io::Result<()> {
    let rows = complete::<tables::FactionMinimumUpgrades>(frame)?;
    let mut groups = group(rows, |row| row.faction, |row| row.ordinal)?;
    let tokens: TokenNames<'_> = complete::<tables::LexicalTokens>(frame)?
        .iter()
        .map(|row| (row.token, row.literal.as_slice()))
        .collect();
    for (&id, saved) in &save.factions {
        let mut values = groups.remove(&id).unwrap_or_default();
        values.sort_unstable_by_key(|row| row.group);
        let values = values
            .into_iter()
            .map(|row| {
                tokens
                    .get(&row.token)
                    .map(|literal| content_reader_parser::parser::c_string(literal).to_vec())
                    .ok_or_else(|| invalid("faction minimum upgrade token spelling unavailable"))
            })
            .collect::<io::Result<Vec<_>>>()?;
        compare(
            values,
            &saved.companions.minimum_upgrades,
            id,
            "upgrades.member_upgrades",
            report,
        );
    }
    if !groups.is_empty() {
        return Err(invalid("faction minimum upgrades have absent owners"));
    }
    report.compared("FactionMinimumUpgrades", rows.len());
    Ok(())
}
pub fn doctrine(frame: &DecodedFrame, save: &Save, report: &mut Report) -> io::Result<()> {
    let rows = complete::<tables::FactionDoctrine>(frame)?;
    let mut groups = group(rows, |row| row.faction, |row| row.ordinal)?;
    let tokens: TokenNames<'_> = complete::<tables::LexicalTokens>(frame)?
        .iter()
        .map(|row| (row.token, row.literal.as_slice()))
        .collect();
    for (&id, saved) in &save.factions {
        let values = groups
            .remove(&id)
            .unwrap_or_default()
            .into_iter()
            .map(|row| {
                tokens
                    .get(&row.token)
                    .map(|literal| content_reader_parser::parser::c_string(literal).to_vec())
                    .ok_or_else(|| invalid("faction doctrine folder token spelling unavailable"))
            })
            .collect::<io::Result<Vec<_>>>()?;
        compare(
            values,
            &saved.companions.doctrine_folders,
            id,
            "upgrades.doctrine.folder",
            report,
        );
    }
    if !groups.is_empty() {
        return Err(invalid("faction doctrine folders have absent owners"));
    }
    report.compared("FactionDoctrine", rows.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::LexicalTokenRow;
    use replay_data::rows::ReservedCountryNamingRow;
    use replay_data::rows::TableRows;
    use replay_frame::frame::TableView;
    fn fixture() -> (DecodedFrame, Save) {
        let (mut frame, mut save, _, _) = crate::fixtures::fixture();
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
                advisor_slots: vec![],
                goals: Default::default(),
                companions: content_reader_save::faction_companions::Snapshot {
                    advisor_slots: Vec::new(),
                    extracted: Some(Vec::new()),
                    projection_effects: None,
                    minimum_upgrades: Vec::new(),
                    doctrine_folders: Vec::new(),
                    sharing_countries: Vec::new(),
                    sharing_bonuses: Vec::new(),
                    variables: None,
                    research: Some(true),
                    manpower_unlocked: Some(false),
                    rules: vec![b"mod_rule".to_vec(), b"mod_rule".to_vec()],
                    programmes: vec![u64::MAX, 0],
                    manpower: vec![
                        Manpower {
                            source_country: b"RES".to_vec(),
                            amount: 3,
                        },
                        Manpower {
                            source_country: b"GER".to_vec(),
                            amount: 5,
                        },
                    ],
                },
            },
        );
        *frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
                token: 9,
                literal: b"mod_rule".to_vec(),
            }]));
        *frame.tables[TableId::FactionRules.index()] =
            TableView::Complete(TableRows::FactionRules(
                (0..2)
                    .map(|ordinal| FactionRuleRow {
                        faction: 7,
                        ordinal,
                        token: 9,
                    })
                    .collect(),
            ));
        *frame.tables[TableId::FactionPrograms.index()] =
            TableView::Complete(TableRows::FactionPrograms(vec![
                FactionProgramRow {
                    faction: 7,
                    ordinal: 0,
                    reference: u64::MAX,
                },
                FactionProgramRow {
                    faction: 7,
                    ordinal: 1,
                    reference: 0,
                },
            ]));
        *frame.tables[TableId::FactionManpower.index()] =
            TableView::Complete(TableRows::FactionManpower(
                [(-1, 0), (0, 0), (3, 0), (5, 102)]
                    .into_iter()
                    .enumerate()
                    .map(|(ordinal, (value, country))| FactionManpowerRow {
                        faction: 7,
                        ordinal: ordinal as u32,
                        country,
                        value,
                    })
                    .collect(),
            ));
        *frame.tables[TableId::ReservedCountryNaming.index()] =
            TableView::Complete(TableRows::ReservedCountryNaming(vec![
                ReservedCountryNamingRow {
                    tag: b"RES".to_vec(),
                    ..Default::default()
                },
            ]));
        *frame.tables[TableId::FactionUnlocks.index()] =
            TableView::Complete(TableRows::FactionUnlocks(vec![
                replay_data::rows::FactionUnlockRow {
                    faction: 7,
                    research: 2,
                    manpower: 0,
                },
            ]));
        (frame, save)
    }
    #[test]
    fn faction_companion_oracle_matches_order_null_references_and_positive_manpower_writer_gate() {
        let (frame, save) = fixture();
        let countries = super::super::super::country_context(&frame).unwrap();
        let mut report = Report::default();
        unlocks(&frame, &save, &mut report).unwrap();
        rules(&frame, &save, &mut report).unwrap();
        programmes(&frame, &save, &mut report).unwrap();
        manpower(&frame, &save, &countries, &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        let mut changed = save.clone();
        changed
            .factions
            .get_mut(&7)
            .unwrap()
            .companions
            .programmes
            .reverse();
        programmes(&frame, &changed, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
    }
    #[test]
    fn faction_companion_oracle_refuses_missing_tables_and_noncontiguous_occurrences() {
        let (mut frame, save) = fixture();
        *frame.tables[TableId::FactionRules.index()] = TableView::Unavailable {
            reason: "writer unavailable".into(),
        };
        assert!(rules(&frame, &save, &mut Report::default()).is_err());
        *frame.tables[TableId::FactionPrograms.index()] =
            TableView::Complete(TableRows::FactionPrograms(vec![FactionProgramRow {
                faction: 7,
                ordinal: 1,
                reference: 0,
            }]));
        assert!(programmes(&frame, &save, &mut Report::default()).is_err());
    }
}
