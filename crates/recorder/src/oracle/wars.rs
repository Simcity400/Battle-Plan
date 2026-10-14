//! Ordered goal references joined to goal objects.
#[cfg(test)]
mod tests;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use installed_content_save::{Save, war_goals::WarGoal};
use replay::{
    frame::DecodedFrame,
    rows::{WarRelationGoalRow, tables},
};

use super::{CountryContext, Report, TokenNames, complete, invalid, required_country_tag};

pub(super) const ENDED_WAR_LIMITATION: &str =
    "Retired owned war occurrences have no comparable serialized live goal list.";

pub(super) const LIMITATIONS: &[&str] = &[
    "The text save does not serialize war relation retired bytes.",
    "The text save does not serialize diplomacy-owned war occurrence order.",
    "The game does not store grouped war identities, group start/end hours or participant join/leave hours.",
    "The text save does not serialize computed war groups, representative selection or displayed war names.",
    "Stored custom country/state text is exact only in the recording client's language.",
    "The text save does not serialize the cached subject predicate toward the recording viewpoint.",
    "The text save does not establish the raw division roster count or the controlled-core/offsite war overview factory score.",
    "The native default war naming prefix is executable data, not a text-save field.",
];

fn equal<T: std::fmt::Debug + PartialEq>(report: &mut Report, label: &str, actual: T, saved: T) {
    if actual != saved {
        report.difference(label, actual, saved);
    }
}

pub(super) fn compare(
    frame: &DecodedFrame,
    save: &Save,
    countries: &CountryContext<'_>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    for reason in LIMITATIONS {
        report.limitation(reason, "war overview");
    }
    let owned = complete::<tables::WarOwnedRelations>(frame)?;
    let goals = complete::<tables::WarRelationGoals>(frame)?;
    let scores = complete::<tables::WarOverviewCountries>(frame)?;
    let rules = complete::<tables::WarNameRules>(frame)?;
    if rules.len() != 1 || rules[0].index != 0 || rules[0].prefix.is_empty() {
        return Err(invalid("native war naming prefix row absent or invalid"));
    }
    let mut saved = BTreeMap::new();
    for relation in &save.war_relations {
        if saved
            .insert(
                (relation.first.as_slice(), relation.second.as_slice()),
                relation,
            )
            .is_some()
        {
            return Err(invalid("ambiguous saved war endpoints for goal comparison"));
        }
    }
    let mut lists: BTreeMap<_, Vec<&WarRelationGoalRow>> = BTreeMap::new();
    for goal in goals {
        if goal.side > 1 {
            return Err(invalid("war goal side outside 0/1"));
        }
        lists
            .entry((goal.owner, goal.relation_ordinal, goal.side))
            .or_default()
            .push(goal);
    }
    let mut live_owned = BTreeMap::<_, usize>::new();
    let mut score_owners = BTreeSet::new();
    for row in owned {
        score_owners.insert(row.owner);
        let first = required_country_tag(row.first_country, &countries.links)?;
        let second = required_country_tag(row.second_country, &countries.links)?;
        let owner = required_country_tag(row.owner, &countries.links)?;
        if owner != first && owner != second {
            return Err(invalid("owned war owner disagrees with endpoints"));
        }
        let label = format!(
            "WarOwnedRelations {} occurrence {} ({} -> {})",
            String::from_utf8_lossy(owner),
            row.ordinal,
            String::from_utf8_lossy(first),
            String::from_utf8_lossy(second)
        );
        let relation = if row.retired == 0 {
            *live_owned.entry((owner, first, second)).or_default() += 1;
            let relation = saved.get(&(first, second));
            if relation.is_none() {
                report.difference(&label, "live owned occurrence", "no saved relation");
            }
            relation
        } else {
            report.limitation(ENDED_WAR_LIMITATION, label.clone());
            None
        };
        for (side, count) in [row.first_goal_count, row.second_goal_count]
            .into_iter()
            .enumerate()
        {
            let mut actual = lists
                .remove(&(row.owner, row.ordinal, side as u32))
                .unwrap_or_default();
            actual.sort_by_key(|r| r.ordinal);
            if actual.len() != count as usize
                || actual
                    .iter()
                    .enumerate()
                    .any(|(i, r)| r.ordinal as usize != i)
            {
                return Err(invalid(
                    "war goal rows do not cover their complete owned list",
                ));
            }
            if let Some(relation) = relation {
                let expected = if side == 0 {
                    &relation.first_goals
                } else {
                    &relation.second_goals
                };
                let handles: Vec<_> = actual.iter().map(|r| r.handle).collect();
                equal(
                    report,
                    &format!("{label} side {side} ordered handles"),
                    handles.as_slice(),
                    expected.as_slice(),
                );
            }
            for goal in actual.into_iter().filter(|_| row.retired == 0) {
                compare_goal(
                    goal,
                    save.war_goals.get(&goal.handle),
                    countries,
                    tokens,
                    &format!("{label} side {side} goal {}", goal.ordinal),
                    report,
                )?;
            }
        }
    }
    if !lists.is_empty() {
        return Err(invalid("orphan war goal rows without an owned occurrence"));
    }
    for &(first, second) in saved.keys() {
        for owner in BTreeSet::from([first, second]) {
            let count = live_owned
                .get(&(owner, first, second))
                .copied()
                .unwrap_or(0);
            equal(
                report,
                &format!(
                    "WarOwnedRelations {} ({} -> {}) live occurrence coverage",
                    String::from_utf8_lossy(owner),
                    String::from_utf8_lossy(first),
                    String::from_utf8_lossy(second)
                ),
                count,
                1,
            );
        }
    }
    let recorded_scores: BTreeSet<_> = scores.iter().map(|r| r.country).collect();
    for owner in score_owners.difference(&recorded_scores) {
        report.difference(
            format!("WarOverviewCountries owner {owner} coverage"),
            "missing row",
            "owned war country row",
        );
    }
    for row in scores {
        let tag = required_country_tag(row.country, &countries.links)?;
        let country = save
            .countries
            .get(tag)
            .ok_or_else(|| invalid("war score country absent from save"))?;
        equal(
            report,
            &format!(
                "WarOverviewCountries {} major",
                String::from_utf8_lossy(tag)
            ),
            row.major != 0,
            country.major,
        );
        if row.major > 1 {
            report.limitation("The text save represents the major byte as a boolean; its nonzero raw value is not serialized.",String::from_utf8_lossy(tag).into_owned());
        }
    }
    report.compared("WarOwnedRelations", owned.len());
    report.compared("WarRelationGoals", goals.len());
    report.compared("WarOverviewCountries major", scores.len());
    report.compared("WarNameRules structurally checked", rules.len());
    Ok(())
}

fn compare_goal(
    row: &WarRelationGoalRow,
    saved: Option<&WarGoal>,
    countries: &CountryContext<'_>,
    tokens: &TokenNames<'_>,
    label: &str,
    report: &mut Report,
) -> io::Result<()> {
    if row.resolved > 1 {
        return Err(invalid("war goal resolved flag outside 0/1"));
    }
    equal(
        report,
        &format!("{label}.resolved"),
        row.resolved != 0,
        saved.is_some(),
    );
    let Some(saved) = saved.filter(|_| row.resolved != 0) else {
        return Ok(());
    };
    let token = tokens
        .get(&row.type_token)
        .copied()
        .ok_or_else(|| invalid("war goal type token lacks recorded spelling"))?;
    equal(
        report,
        &format!("{label}.type"),
        token,
        saved.kind.as_slice(),
    );
    for (field, raw, expected) in [
        ("actor", row.actor, saved.actor.as_slice()),
        ("recipient", row.recipient, saved.recipient.as_slice()),
    ] {
        equal(
            report,
            &format!("{label}.{field}"),
            required_country_tag(raw, &countries.links)?,
            expected,
        );
    }
    equal(
        report,
        &format!("{label}.state_count"),
        row.state_count as usize,
        saved.states.len(),
    );
    if let Some(&state) = saved.states.first() {
        equal(
            report,
            &format!("{label}.first_state"),
            row.first_state,
            state,
        );
    }
    equal(
        report,
        &format!("{label}.country_count"),
        row.country_count as usize,
        saved.countries.len(),
    );
    if let Some(country) = saved.countries.first() {
        equal(
            report,
            &format!("{label}.first_country"),
            required_country_tag(row.first_country, &countries.links)?,
            country.as_slice(),
        );
    }
    Ok(())
}
