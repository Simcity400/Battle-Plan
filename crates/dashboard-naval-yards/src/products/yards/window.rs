//! A country's yard groups (ports or refit lines) and the tokens its window names.
use super::*;

/// Resolve only tokens that the accepted, selected window references.
pub fn present(
    query: &Query<'_>,
    window: &KeyedWindow,
    state: &State,
) -> io::Result<(BTreeMap<String, YardChoice>, Vec<String>)> {
    dashboard_text::metadata::check_cancel(query.cancelled)?;
    let mut warnings = BTreeMap::<String, u64>::new();
    let mut demanded = BTreeSet::new();
    for series in &window.series {
        let parts: Vec<_> = series.name.split('|').collect();
        if parts.get(1) == Some(&"refusal") {
            if let Some(Operand::Refusal(why)) = parts
                .get(2)
                .and_then(|id| id.parse::<usize>().ok())
                .and_then(|id| state.tokens.get(id))
            {
                let count: u64 = series
                    .runs
                    .iter()
                    .map(|(a, b, n, _)| {
                        n.parse::<u64>()
                            .unwrap_or(0)
                            .saturating_mul((i64::from(*b) - i64::from(*a) + 1) as u64)
                    })
                    .sum();
                *warnings.entry(why.clone()).or_default() += count;
            }
        } else if is_label(parts.last().copied().unwrap_or_default()) {
            for (_, _, value, _) in &series.runs {
                if let Ok(id) = value.parse::<usize>() {
                    demanded.insert(id);
                }
            }
        }
    }
    let cat = query
        .content
        .map(|options| catalogue::catalogue(query.source, options))
        .transpose();
    let cat = match cat {
        Ok(cat) => cat,
        Err(why) => {
            warnings.insert(format!("Naval yard content names unavailable: {why}"), 1);
            None
        }
    };
    let mut tokens = BTreeMap::new();
    for id in demanded {
        dashboard_text::metadata::check_cancel(query.cancelled)?;
        let Some(operand) = state.tokens.get(id) else {
            return Err(io::Error::other("Naval yard token is missing"));
        };
        if let Operand::Port(port) = operand {
            if let Err(why) = &port.override_name {
                *warnings.entry(why.clone()).or_default() += 1;
            }
            if port
                .override_name
                .as_ref()
                .is_ok_and(|name| name.is_empty())
                && let Err(why) = &port.owner_tag
            {
                *warnings.entry(why.clone()).or_default() += 1;
            }
        }
        if let Operand::ShipRebuilt { naming, group, .. } = operand
            && ship_names::needs_groups(naming, group)
        {
            if let Some(cat) = cat.as_ref() {
                if let Ok(groups) = ship_names::groups(cat) {
                    for note in &groups.1 {
                        warnings.entry(note.clone()).or_insert(1);
                    }
                }
            }
        }
        let choice = match resolve(operand, cat.as_ref()) {
            Ok(choice) => choice,
            Err(why) => {
                *warnings.entry(why).or_default() += 1;
                YardChoice {
                    id: raw_id(operand),
                    name: "Name unavailable".into(),
                }
            }
        };
        tokens.insert(id.to_string(), choice);
    }
    let mut warnings: Vec<_> = warnings
        .into_iter()
        .map(|(why, count)| format!("{why} ({count})"))
        .collect();
    if let Some(cat) = cat {
        warnings.extend(cat.verification.iter().chain(&cat.warnings).cloned());
        warnings.extend(cat.refusal_lines());
    }
    Ok((tokens, warnings))
}
pub fn groups(
    window: &KeyedWindow,
    tokens: &BTreeMap<String, YardChoice>,
    refits: bool,
) -> Vec<YardGroup> {
    let mut groups = BTreeMap::<(String, String), (Option<String>, BTreeSet<String>)>::new();
    for series in &window.series {
        let parts: Vec<_> = series.name.split('|').collect();
        if parts.len() < 3 || parts[1].parse::<u64>().is_err() {
            continue;
        }
        let group = groups
            .entry((parts[0].into(), parts[1].into()))
            .or_default();
        if (parts[2] == "name" || (refits && parts[2] == "targetName"))
            && let Some((_, _, id, _)) = series.runs.last()
        {
            group.0 = tokens.get(id).map(|choice| choice.name.clone());
        }
        if !refits && parts.get(2) == Some(&"ship") && parts.len() == 5 && parts[4] == "present" {
            group.1.insert(parts[3].into());
        }
        if refits && parts.get(2) == Some(&"ship") && parts.len() == 5 && parts[4] == "present" {
            group.1.insert(parts[3].into());
        }
    }
    let mut ship_names = BTreeMap::new();
    for series in &window.series {
        let parts: Vec<_> = series.name.split('|').collect();
        if parts.len() == 5 && parts[2] == "ship" && matches!(parts[4], "name" | "nameRebuilt") {
            if let Some((_, hour, token, _)) = series.runs.last() {
                let key = (
                    parts[0].to_owned(),
                    parts[1].to_owned(),
                    parts[3].to_owned(),
                );
                let entry = ship_names.entry(key).or_insert((i32::MIN, String::new()));
                if *hour >= entry.0 {
                    *entry = (
                        *hour,
                        tokens
                            .get(token)
                            .map(|choice| choice.name.clone())
                            .unwrap_or_else(|| "Ship name unavailable".into()),
                    );
                }
            }
        }
    }
    groups
        .into_iter()
        .map(|((country, id), (name, ships))| YardGroup {
            country: country.clone(),
            id: id.clone(),
            name: name.unwrap_or_else(|| "Name unavailable".into()),
            ships: ships
                .into_iter()
                .map(|ship| YardChoice {
                    name: ship_names
                        .get(&(country.clone(), id.clone(), ship.clone()))
                        .map(|(_, name)| name.clone())
                        .unwrap_or_else(|| "Ship name unavailable".into()),
                    id: ship,
                })
                .collect(),
        })
        .collect()
}

