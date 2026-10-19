//! Compare complete goal inventories and justification serializer projections.
use super::*;
use installed_content_save::{Save, SavedDate};
use replay_data::rows::WargoalTargetRow;
use replay_data::rows::tables;
use replay_frame::frame::DecodedFrame;
use std::{collections::BTreeMap, io};

fn equal<T: std::fmt::Debug + PartialEq>(r: &mut Report, label: &str, a: T, b: T) {
    if a != b {
        r.difference(label, a, b);
    }
}

pub fn compare(
    frame: &DecodedFrame,
    save: &Save,
    countries: &CountryContext<'_>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let goals = complete::<tables::CountryWargoals>(frame)?;
    let targets = complete::<tables::WargoalTargets>(frame)?;
    let justifications = complete::<tables::WargoalJustifications>(frame)?;
    let goal_owners: std::collections::BTreeSet<_> =
        goals.iter().map(|r| (r.country, r.ordinal)).collect();
    let justification_owners: std::collections::BTreeSet<_> = justifications
        .iter()
        .map(|r| (r.country, r.ordinal))
        .collect();
    let mut lists = BTreeMap::<_, Vec<&WargoalTargetRow>>::new();
    for row in targets {
        let tag = required_country_tag(row.country, &countries.links)?;
        if !save.wargoal_countries.contains_key(tag)
            || match row.kind {
                0..=2 => !goal_owners.contains(&(row.country, row.goal)),
                3 => row.goal != 0,
                4 => !justification_owners.contains(&(row.country, row.goal)),
                _ => true,
            }
        {
            return Err(invalid(
                "war-goal target has no matching saved inventory owner",
            ));
        }
        lists
            .entry((row.country, row.kind, row.goal))
            .or_default()
            .push(row);
    }
    for values in lists.values_mut() {
        values.sort_by_key(|r| r.ordinal);
        if values
            .iter()
            .enumerate()
            .any(|(i, r)| i != r.ordinal as usize)
        {
            return Err(invalid("war-goal targets lack complete native order"));
        }
    }
    let list = |country, kind, goal| -> Vec<u64> {
        lists
            .get(&(country, kind, goal))
            .into_iter()
            .flatten()
            .map(|r| r.value)
            .collect()
    };
    let mut by_country = BTreeMap::<_, Vec<_>>::new();
    for row in goals {
        by_country
            .entry(required_country_tag(row.country, &countries.links)?.to_vec())
            .or_default()
            .push(row);
    }
    let mut just_by_country = BTreeMap::<_, Vec<_>>::new();
    for row in justifications {
        just_by_country
            .entry(required_country_tag(row.country, &countries.links)?.to_vec())
            .or_default()
            .push(row);
    }
    // Every saved country participates, including complete empty inventories.
    for (tag, inventory) in &save.wargoal_countries {
        let mut actual = by_country.remove(tag).unwrap_or_default();
        actual.sort_by_key(|r| r.ordinal);
        if actual
            .iter()
            .enumerate()
            .any(|(i, r)| r.ordinal as usize != i)
        {
            return Err(invalid("war-goal object roster lacks native order"));
        }
        let label = format!("CountryWargoals {}", String::from_utf8_lossy(tag));
        equal(
            report,
            &label,
            actual.iter().map(|r| r.handle).collect::<Vec<_>>(),
            inventory.goals.clone(),
        );
        // Empty object rosters can still have an available-reference roster.
        let raw = frame
            .complete::<tables::Countries>()?
            .iter()
            .find(|c| c.tag == *tag)
            .map(|c| c.id);
        let available = raw.map(|id| list(id, 3, 0)).unwrap_or_default();
        equal(
            report,
            &format!("{label}.available"),
            available,
            inventory.available.clone(),
        );
        for row in actual {
            let saved = save
                .war_goals
                .get(&row.handle)
                .ok_or_else(|| invalid("recorded war-goal object lacks saved handle"))?;
            let subject = format!("{label}[{}]", row.ordinal);
            equal(
                report,
                &format!("{subject}.actor"),
                required_country_tag(row.actor, &countries.links)?,
                saved.actor.as_slice(),
            );
            equal(
                report,
                &format!("{subject}.recipient"),
                required_country_tag(row.recipient, &countries.links)?,
                saved.recipient.as_slice(),
            );
            let stored_name = tokens
                .get(&row.name_token)
                .copied()
                .ok_or_else(|| invalid("war-goal serialized name lacks spelling"))?;
            equal(
                report,
                &format!("{subject}.name"),
                stored_name,
                saved.name.as_slice(),
            );
            let spelling = tokens
                .get(&row.goal_type)
                .copied()
                .ok_or_else(|| invalid("war-goal type lacks recorded spelling"))?;
            equal(
                report,
                &format!("{subject}.type"),
                spelling,
                saved.kind.as_slice(),
            );
            equal(
                report,
                &format!("{subject}.states"),
                list(row.country, 0, row.ordinal),
                saved.states.iter().map(|v| *v as u64).collect(),
            );
            for (kind, name, expected) in [
                (1, "liberate", &saved.countries),
                (2, "puppets", &saved.puppets),
            ] {
                let actual = list(row.country, kind, row.ordinal)
                    .into_iter()
                    .map(|r| required_country_tag(r as u32, &countries.links).map(Vec::from))
                    .collect::<io::Result<Vec<_>>>()?;
                equal(
                    report,
                    &format!("{subject}.{name}"),
                    actual,
                    expected.clone(),
                );
            }
            // The native serializer writes only dates beyond its initial-year gate.
            let expires = row.serialized_expiry();
            equal(
                report,
                &format!("{subject}.expire"),
                expires,
                saved.expires.map(SavedDate::engine_hour).transpose()?,
            );
        }
        let mut actual = just_by_country.remove(tag).unwrap_or_default();
        actual.sort_by_key(|r| r.ordinal);
        equal(
            report,
            &format!("{label}.justification_count"),
            actual.len(),
            inventory.justifications.len(),
        );
        for (row, saved) in actual.into_iter().zip(&inventory.justifications) {
            let subject = format!(
                "WargoalJustifications {}[{}]",
                String::from_utf8_lossy(tag),
                row.ordinal
            );
            equal(
                report,
                &format!("{subject}.ordinal"),
                row.ordinal,
                saved.ordinal,
            );
            equal(
                report,
                &format!("{subject}.owner"),
                required_country_tag(row.actor, &countries.links)?,
                saved.actor.as_slice(),
            );
            equal(
                report,
                &format!("{subject}.target"),
                required_country_tag(row.recipient, &countries.links)?,
                saved.recipient.as_slice(),
            );
            let spelling = tokens
                .get(&row.goal_type)
                .copied()
                .ok_or_else(|| invalid("justification type lacks recorded spelling"))?;
            equal(
                report,
                &format!("{subject}.type"),
                spelling,
                saved.kind.as_slice(),
            );
            for (field, actual, saved) in [
                ("total_cost", row.total_cost_raw, saved.total_cost.as_ref()),
                ("daily_cost", row.daily_cost_raw, saved.daily_cost.as_ref()),
                ("points", row.progress_raw, saved.points.as_ref()),
            ] {
                equal(
                    report,
                    &format!("{subject}.{field}"),
                    (actual > 0).then_some(actual),
                    saved.map(|v| v.raw),
                );
            }
            equal(
                report,
                &format!("{subject}.threat"),
                (row.threat_raw != 0).then_some(row.threat_raw),
                saved.threat.as_ref().map(|v| v.raw),
            );
            equal(
                report,
                &format!("{subject}.pay_upfront"),
                row.pay_upfront != 0,
                saved.pay_upfront,
            );
            for (field, actual, saved) in [
                ("start", row.start, saved.start),
                ("end", row.end, saved.end),
            ] {
                equal(
                    report,
                    &format!("{subject}.{field}"),
                    (actual != SavedDate::INVALID_HOUR).then_some(actual),
                    saved.map(SavedDate::engine_hour).transpose()?,
                );
            }
            equal(
                report,
                &format!("{subject}.states"),
                list(row.country, 4, row.ordinal),
                saved.states.iter().map(|v| *v as u64).collect(),
            );
        }
    }
    if !by_country.is_empty() || !just_by_country.is_empty() {
        return Err(invalid(
            "recorded war-goal owner is absent from saved country inventory",
        ));
    }
    report.compared("CountryWargoals", goals.len());
    report.compared("WargoalTargets", targets.len());
    report.compared("WargoalJustifications", justifications.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use installed_content_save::{
        Decimal,
        war_goals::{CountryInventory, Justification, WarGoal},
    };
    use replay_data::rows;
    use replay_data::rows::TableId;
    use replay_data::rows::TableRows;
    use replay_frame::frame::TableView;
    #[test]
    fn wargoal_oracle_compares_later_targets_expiry_and_justification_progress() {
        let (mut frame, mut save, _, _) = crate::fixtures::fixture();
        let handle = (4u64 << 32) | 123;
        save.war_goals.insert(
            handle,
            WarGoal {
                actor: b"GER".to_vec(),
                recipient: b"GER".to_vec(),
                kind: b"synthetic".to_vec(),
                name: b"synthetic".to_vec(),
                states: vec![1, 77, 1],
                countries: vec![b"GER".to_vec()],
                puppets: vec![b"GER".to_vec()],
                expires: None,
            },
        );
        save.wargoal_countries.insert(
            b"GER".to_vec(),
            CountryInventory {
                goals: vec![handle],
                available: vec![handle],
                justifications: vec![Justification {
                    ordinal: 2,
                    actor: b"GER".to_vec(),
                    recipient: b"GER".to_vec(),
                    kind: b"synthetic".to_vec(),
                    total_cost: Some(Decimal::parse(b"90").unwrap()),
                    daily_cost: Some(Decimal::parse(b"1").unwrap()),
                    points: Some(Decimal::parse(b"17.5").unwrap()),
                    start: None,
                    end: None,
                    pay_upfront: true,
                    threat: None,
                    states: vec![77, 1],
                }],
            },
        );
        frame.tables[TableId::CountryWargoals.index()] =
            TableView::Complete(TableRows::CountryWargoals(vec![rows::CountryWargoalRow {
                country: 101,
                handle,
                actor: 101,
                recipient: 101,
                goal_type: 42,
                name_token: 42,
                ..Default::default()
            }]))
            .into();
        let target = |kind, goal, ordinal, value| rows::WargoalTargetRow {
            country: 101,
            kind,
            goal,
            ordinal,
            value,
        };
        frame.tables[TableId::WargoalTargets.index()] =
            TableView::Complete(TableRows::WargoalTargets(vec![
                target(0, 0, 0, 1),
                target(0, 0, 1, 77),
                target(0, 0, 2, 1),
                target(1, 0, 0, 101),
                target(2, 0, 0, 101),
                target(3, 0, 0, handle),
                target(4, 2, 0, 77),
                target(4, 2, 1, 1),
            ]))
            .into();
        frame.tables[TableId::WargoalJustifications.index()] =
            TableView::Complete(TableRows::WargoalJustifications(vec![
                rows::WargoalJustificationRow {
                    country: 101,
                    ordinal: 2,
                    actor: 101,
                    recipient: 101,
                    goal_type: 42,
                    total_cost_raw: 9000000,
                    daily_cost_raw: 100000,
                    progress_raw: 1750000,
                    start: SavedDate::INVALID_HOUR,
                    end: SavedDate::INVALID_HOUR,
                    pay_upfront: 1,
                    threat_raw: 0,
                },
            ]))
            .into();
        let countries = country_context(&frame).unwrap();
        let tokens = BTreeMap::from([(42, b"synthetic".as_slice())]);
        let mut report = Report::default();
        compare(&frame, &save, &countries, &tokens, &mut report).unwrap();
        assert!(
            report.difference_lines().is_empty(),
            "{:?}",
            report.difference_lines()
        );
        save.war_goals.get_mut(&handle).unwrap().states[2] = 999;
        save.wargoal_countries
            .get_mut(b"GER".as_slice())
            .unwrap()
            .justifications[0]
            .points = Some(Decimal::parse(b"18").unwrap());
        let mut report = Report::default();
        compare(&frame, &save, &countries, &tokens, &mut report).unwrap();
        assert!(
            report
                .difference_lines()
                .iter()
                .any(|line| line.contains(".states"))
        );
        assert!(
            report
                .difference_lines()
                .iter()
                .any(|line| line.contains(".points"))
        );
        frame.tables[TableId::WargoalTargets.index()] =
            TableView::Complete(TableRows::WargoalTargets(vec![target(0, 99, 0, 1)])).into();
        let countries = country_context(&frame).unwrap();
        assert!(
            compare(&frame, &save, &countries, &tokens, &mut Report::default()).is_err(),
            "orphan targets must not count as compared"
        );
    }
}
