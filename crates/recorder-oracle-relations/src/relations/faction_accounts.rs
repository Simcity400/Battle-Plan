use super::*;
pub fn compare(
    frame: &DecodedFrame,
    save: &Save,
    countries: &CountryContext<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let all = complete::<tables::FactionAccounts>(frame)?;
    let rows = all.iter().filter(|row| row.kind == 0).collect::<Vec<_>>();
    let expected = save
        .faction_accounts
        .as_ref()
        .ok_or_else(|| invalid("save lacks faction_system.countries"))?;
    report.scalar_checks += 1;
    if rows.len() != expected.count as usize {
        report.difference("faction_system.countries size", rows.len(), expected.count);
    }
    let members = complete::<tables::FactionMembers>(frame)?;
    let tokens: TokenNames<'_> = complete::<tables::LexicalTokens>(frame)?
        .iter()
        .map(|row| (row.token, row.literal.as_slice()))
        .collect();
    let spell = |token: u32| -> io::Result<Vec<u8>> {
        tokens
            .get(&token)
            .map(|value| content_reader_parser::parser::c_string(value).to_vec())
            .ok_or_else(|| invalid("faction account definition lacks token spelling"))
    };
    let mut saved = expected.entries.clone();
    let zero = content_reader_save::Decimal::parse(b"0")?;
    for (index, row) in rows.iter().enumerate() {
        if row.account as usize != index {
            return Err(invalid("noncontiguous faction account indices"));
        }
        let active = if (row.country as i32) > 0 {
            let country = countries
                .links
                .get(&row.country)
                .ok_or_else(|| invalid("faction account country has no recorded link"))?;
            members.iter().any(|member| {
                countries
                    .links
                    .get(&member.country)
                    .is_some_and(|value| value.canonical_slot == country.canonical_slot)
            })
        } else {
            false
        };
        let admitted = active || !row.completed_goals.is_empty();
        let expected = saved.remove(&row.account);
        let label = format!("faction_system.countries[{}]", row.account);
        if !admitted {
            if expected.is_some() {
                report.difference(label, "omitted", "present");
            }
            continue;
        }
        let Some(expected) = expected else {
            report.difference(label, "present", "omitted");
            continue;
        };
        report.scalar_checks += 1;
        let influence = replay_data::rows::FactionAccountRow::statistics(all, row.account, 1)
            .collect::<Vec<_>>();
        if influence.len() != expected.influence_count as usize {
            report.difference(
                format!("{label}.influence size"),
                influence.len(),
                expected.influence_count,
            );
        }
        for (index, value) in influence.iter().enumerate() {
            report.scalar(
                &label,
                &format!("influence[{index}]"),
                Some(*value),
                Some(expected.influence.get(&(index as u32)).unwrap_or(&zero)),
            );
        }
        report.scalar(
            &label,
            "initiative",
            Some(row.initiative()),
            Some(&expected.initiative),
        );
        report.scalar(
            &label,
            "contribution",
            Some(row.contribution_raw),
            Some(&expected.contribution),
        );
        report.scalar(
            &label,
            "war_score_breakdown",
            Some(row.previous_war_score_raw),
            Some(&expected.previous_score),
        );
        if expected.gain.len() != 2 {
            return Err(invalid(
                "faction account contribution_gain must contain both native operands",
            ));
        }
        for (index, value) in [row.contribution_gain_a_raw, row.contribution_gain_b_raw]
            .into_iter()
            .enumerate()
        {
            report.scalar(
                &label,
                &format!("contribution_gain[{index}]"),
                Some(value),
                Some(&expected.gain[index]),
            );
        }
        let mut upgrades = row
            .upgrade_groups
            .iter()
            .copied()
            .zip(row.upgrade_tokens.iter().copied())
            .collect::<Vec<_>>();
        upgrades.sort_unstable_by_key(|value| value.0);
        let upgrades = upgrades
            .into_iter()
            .map(|(_, token)| spell(token))
            .collect::<io::Result<Vec<_>>>()?;
        if upgrades != expected.upgrades {
            report.difference(
                format!("{label}.member_upgrades"),
                upgrades,
                expected.upgrades,
            );
        }
        let completed = row
            .completed_goals
            .iter()
            .filter(|value| **value != u64::MAX)
            .map(|value| spell(*value as u32))
            .collect::<io::Result<Vec<_>>>()?;
        if completed != expected.completed {
            report.difference(
                format!("{label}.completed_faction_goals"),
                completed,
                expected.completed,
            );
        }
    }
    if !saved.is_empty() {
        report.difference(
            "faction_system unmatched account indices",
            Vec::<u32>::new(),
            saved.keys().copied().collect::<Vec<_>>(),
        );
    }
    if !rows.is_empty() {
        report.limitation("account country references select save admission but are not independently serialized; verify the member identity in the matching screen","FactionAccounts.country");
        for field in ["contribution_subject_gain", "population_growth"] {
            report.limitation("effective country accumulator operands are not independently serialized in faction accounts; compare the demanded getter and the dated contribution explanation in integration", format!("FactionAccounts.{field}"));
        }
        for field in [
            "projection",
            "influence_share_raw",
            "projection_share_raw",
            "projection_rank",
        ] {
            report.limitation("member projection, normalized shares and rank are unsaved retained refresh outputs; verify the matching faction member screen",format!("FactionAccounts.{field}"));
        }
    }
    report.compared("FactionAccounts", rows.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::FactionAccountRow;
    use replay_data::rows::LexicalTokenRow;
    use replay_data::rows::TableRows;
    use replay_frame::frame::TableView;
    #[test]
    fn faction_account_oracle_preserves_departed_history_sparse_admission_and_gain_only_changes() {
        let (mut frame, mut save, _, _) = crate::fixtures::fixture();
        let number = |text: &str| content_reader_save::Decimal::parse(text.as_bytes()).unwrap();
        save.faction_accounts = Some(content_reader_save::faction_accounts::Accounts {
            count: 2,
            entries: BTreeMap::from([(
                1,
                content_reader_save::faction_accounts::Account {
                    influence_count: 2,
                    influence: BTreeMap::from([(1, number("-1"))]),
                    initiative: number("0"),
                    contribution: number("1"),
                    gain: vec![number("3"), number("4")],
                    previous_score: number("-5"),
                    upgrades: vec![b"mod_upgrade".to_vec(); 2],
                    completed: vec![b"mod_upgrade".to_vec(); 2],
                },
            )]),
            ..Default::default()
        });
        *frame.tables[TableId::FactionMembers.index()] =
            TableView::Complete(TableRows::FactionMembers(vec![]));
        *frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
                token: 9,
                literal: b"mod_upgrade".to_vec(),
            }]));
        let rows = vec![
            FactionAccountRow {
                account: 0,
                country: 0,
                initiative_raw: 900000,
                ..Default::default()
            },
            FactionAccountRow {
                account: 1,
                country: u32::MAX,
                contribution_raw: 100000,
                contribution_gain_a_raw: 300000,
                contribution_gain_b_raw: 400000,
                previous_war_score_raw: -500000,
                upgrade_groups: vec![7, 2],
                upgrade_tokens: vec![9, 9],
                completed_goals: vec![u64::MAX, 9, 9],
                ..Default::default()
            },
            FactionAccountRow {
                account: 1,
                kind: 1,
                ordinal: 0,
                statistic_raw: 0,
                ..Default::default()
            },
            FactionAccountRow {
                account: 1,
                kind: 1,
                ordinal: 1,
                statistic_raw: -100000,
                ..Default::default()
            },
        ];
        *frame.tables[TableId::FactionAccounts.index()] =
            TableView::Complete(TableRows::FactionAccounts(rows.clone()));
        let countries = super::super::super::country_context(&frame).unwrap();
        let mut report = Report::default();
        compare(&frame, &save, &countries, &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        for field in ["contribution_subject_gain", "population_growth"] {
            assert!(
                report
                    .limitations
                    .values()
                    .flatten()
                    .any(|subject| subject == &format!("FactionAccounts.{field}"))
            );
        }
        let mut changed = rows;
        changed[1].contribution_gain_b_raw += 100000;
        *frame.tables[TableId::FactionAccounts.index()] =
            TableView::Complete(TableRows::FactionAccounts(changed));
        let countries = super::super::super::country_context(&frame).unwrap();
        compare(&frame, &save, &countries, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
    }
}
