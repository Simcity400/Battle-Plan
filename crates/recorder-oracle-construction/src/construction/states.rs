use super::*;

fn earlier_states(
    report: &mut Report,
    table: &str,
    states: impl Iterator<Item = u32>,
    sampled: &dyn Fn(u32) -> bool,
) {
    let earlier: BTreeSet<u32> = states.filter(|&state| !sampled(state)).collect();
    if !earlier.is_empty() {
        report.limitation(
            &format!(
                "{} states' rows were read before this hour (sliced, daily) and are not compared; compare at a full-refresh hour",
                earlier.len()
            ),
            table,
        );
    }
}

pub fn state_pools(
    rows: &[replay_data::rows::StateResourceRow],
    saved: &BTreeMap<(u32, Vec<u8>), content_reader_save::Decimal>,
    names: &BTreeMap<u32, &[u8]>,
    sampled: &dyn Fn(u32) -> bool,
    report: &mut Report,
) -> io::Result<()> {
    earlier_states(
        report,
        "StateResources",
        rows.iter()
            .map(|row| row.state)
            .chain(saved.keys().map(|(state, _)| *state)),
        sampled,
    );
    let mut seen = BTreeSet::new();
    let mut matched = 0;
    let emitted: BTreeSet<_> = rows
        .iter()
        .filter(|row| row.pool > 0)
        .map(|row| row.state)
        .collect();
    let omitted_negative: BTreeSet<_> = rows
        .iter()
        .filter(|row| row.pool < 0 && !emitted.contains(&row.state) && sampled(row.state))
        .map(|row| row.state)
        .collect();
    if !omitted_negative.is_empty() {
        report.limitation(
            "the game omits resource blocks with no positive amounts; those omissions cannot certify the negative pool values",
            format!("StateResources states {omitted_negative:?}"),
        );
    }
    let mut captured_counts = BTreeMap::<u32, usize>::new();
    let mut unresolved = BTreeSet::new();
    for row in rows
        .iter()
        .filter(|row| row.pool != 0 && emitted.contains(&row.state) && sampled(row.state))
    {
        *captured_counts.entry(row.state).or_default() += 1;
        let name = match token(names, row.resource) {
            Ok(name) => name,
            Err(reason) => {
                unresolved.insert(row.state);
                report.limitation(
                    &format!("state resource {reason}; its pool cannot be matched by name"),
                    format!("state resources.{}.token({})", row.state, row.resource),
                );
                continue;
            }
        };
        let key = (row.state, name.to_vec());
        let subject = format!(
            "state resources.{}.{}",
            row.state,
            String::from_utf8_lossy(name)
        );
        seen.insert(key.clone());
        match saved.get(&key) {
            Some(amount) => {
                matched += 1;
                value(report, &format!("{subject}.pool"), row.pool, amount.raw);
            }
            None => report.difference(subject, row.pool, "missing saved pool"),
        }
    }
    let saved_pools = || {
        saved
            .iter()
            .filter(|((state, _), amount)| amount.raw != 0 && sampled(*state))
            .map(|(key, _)| key)
    };
    missing(
        report,
        "state resources",
        &seen,
        saved_pools()
            .filter(|(state, _)| !unresolved.contains(state))
            .cloned(),
    );
    for state in unresolved {
        let subject = format!("state resources.{state}");
        value(
            report,
            &format!("{subject}.row_count"),
            captured_counts[&state],
            saved_pools().filter(|(pool_state, _)| *pool_state == state).count(),
        );
        report.limitation(
            "unresolved resource tokens prevent a named pool census at this state; its nonzero pool count is compared without assigning names",
            subject,
        );
    }
    report.compared("StateResources.pool", matched);
    report.limitation(
        "the resource accumulator operands (additive, factor, cost) are not saved",
        "StateResources",
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn resource_inputs(
    rows: &[replay_data::rows::StateResourceInputRow],
    states: &[replay_data::rows::StateRow],
    saved: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    names: &BTreeMap<u32, &[u8]>,
    sampled: &dyn Fn(u32) -> bool,
    report: &mut Report,
) -> io::Result<()> {
    earlier_states(
        report,
        "StateResourceInputs",
        rows.iter().map(|row| row.state),
        sampled,
    );
    let infrastructure = token(
        names,
        engine_native_parts::cbuilding_database::INFRASTRUCTURE_TOKEN,
    )
    .map_err(|reason| invalid(format!("the infrastructure building {reason}")))?;
    let controllers: BTreeMap<u32, u32> = states
        .iter()
        .map(|state| (state.id, state.controller))
        .collect();
    let captured: BTreeSet<u32> = rows.iter().map(|row| row.state).collect();
    for state in states
        .iter()
        .filter(|state| sampled(state.id) && !captured.contains(&state.id))
    {
        report.difference(
            format!("StateResourceInputs.{}", state.id),
            "absent",
            "the row of a state SliceReads dates to this hour",
        );
    }
    let mut matched = 0;
    for row in rows.iter().filter(|row| sampled(row.state)) {
        let subject = format!("StateResourceInputs.{}", row.state);
        let controller = controllers
            .get(&row.state)
            .ok_or_else(|| invalid(format!("{subject} has no captured state")))?;
        let origin = country_tag(*controller, links)?
            .and_then(|tag| saved.country_resources.get(tag))
            .is_some_and(|resources| resources.origin_states.contains(&row.state));
        matched += 1;
        value(
            report,
            &format!("{subject}.origin"),
            row.origin != 0,
            origin,
        );
        if row.origin == 0 {
            continue;
        }
        let level = saved
            .state_buildings
            .get(&(row.state, infrastructure.to_vec()))
            .map(|building| i32::from(building.level));
        value(
            report,
            &format!("{subject}.infrastructure_level"),
            row.infrastructure_level,
            level,
        );
    }
    report.compared("StateResourceInputs", matched);
    report.limitation(
        "the resource gate, the global factor (state modifier 0x223), the controller's infrastructure modifier (0x292) and the supply-route test are runtime reads the save does not carry",
        "StateResourceInputs.gate, global_factor, infrastructure_modifier, supply_route",
    );
    Ok(())
}

type OverrideEntries = Vec<(u32, Vec<(u32, i64)>)>;

#[allow(clippy::too_many_arguments)]
pub fn building_overrides(
    rows: &[replay_data::rows::CountryBuildingResourceRow],
    reads: &[replay_data::rows::SliceReadRow],
    resources: &[replay_data::rows::ResourceDefinitionRow],
    states: &[replay_data::rows::StateRow],
    saved: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    names: &BTreeMap<u32, &[u8]>,
    hour: replay_data::Hour,
    report: &mut Report,
) -> io::Result<()> {
    let mut captured: BTreeMap<u32, OverrideEntries> = BTreeMap::new();
    let mut ordinals: BTreeMap<u32, u32> = BTreeMap::new();
    for row in rows {
        let next = ordinals.entry(row.country).or_default();
        if row.ordinal != *next {
            return Err(invalid(
                "building resource override ordinals are not contiguous",
            ));
        }
        *next += 1;
        let entries = captured.entry(row.country).or_default();
        if row.entry as usize == entries.len() {
            entries.push((row.building, Vec::new()));
        } else if row.entry as usize + 1 != entries.len()
            || entries[row.entry as usize].0 != row.building
        {
            return Err(invalid(
                "building resource override entries are not contiguous",
            ));
        }
        if row.count != 0 {
            entries
                .last_mut()
                .unwrap()
                .1
                .push((row.resource, row.amount));
        }
    }
    for row in rows {
        if row.count as usize != captured[&row.country][row.entry as usize].1.len() {
            return Err(invalid(
                "a building resource override's element count disagrees with its rows",
            ));
        }
    }
    let by_index: BTreeMap<u32, u32> = resources.iter().map(|row| (row.id, row.token)).collect();
    let controllers: BTreeSet<u32> = states
        .iter()
        .filter_map(|state| links.get(&state.controller).map(|country| country.id))
        .collect();
    let mut read = BTreeSet::new();
    let mut earlier = BTreeSet::new();
    let mut unread = BTreeSet::new();
    for &country in captured.keys().chain(&controllers) {
        match replay_data::sliced::read_hour(
            reads,
            replay_data::rows::TableId::CountryBuildingResources,
            country,
        ) {
            Some(read_at) if read_at == hour => {
                read.insert(country);
            }
            Some(_) => {
                earlier.insert(country);
            }
            None if captured.contains_key(&country) => {
                return Err(invalid(format!(
                    "country {country}'s building resource overrides have no SliceReads hour"
                )));
            }
            None => {
                unread.insert(country);
            }
        }
    }
    let no_entries = OverrideEntries::new();
    let mut compared = 0;
    for &country in &read {
        let tag = required_country_tag(country, links)?;
        let subject = format!("CountryBuildingResources.{}", String::from_utf8_lossy(tag));
        let entries = captured.get(&country).unwrap_or(&no_entries);
        let expected = saved
            .country_resources
            .get(tag)
            .map(|resources| resources.building_overrides.as_slice())
            .unwrap_or_default();
        value(
            report,
            &format!("{subject}.entries"),
            entries.len(),
            expected.len(),
        );
        for (ordinal, ((building, elements), (saved_building, saved_elements))) in
            entries.iter().zip(expected).enumerate()
        {
            compared += 1;
            let subject = format!("{subject}[{ordinal}]");
            match token(names, *building) {
                Ok(name) => value(
                    report,
                    &format!("{subject}.building"),
                    name,
                    saved_building.as_slice(),
                ),
                Err(reason) => report.limitation(
                    &format!(
                        "building {reason}; the override's building cannot be matched by name"
                    ),
                    format!("{subject}.token({building})"),
                ),
            }
            value(
                report,
                &format!("{subject}.count"),
                elements.len(),
                saved_elements.len(),
            );
            for (element, (&(resource, amount), (index, saved_amount))) in
                elements.iter().zip(saved_elements).enumerate()
            {
                let subject = format!("{subject}.{element}");
                let saved_resource = match *index {
                    0 => Some(0),
                    index => u32::try_from(index)
                        .ok()
                        .and_then(|index| by_index.get(&index).copied()),
                };
                match saved_resource {
                    Some(saved_resource) => value(
                        report,
                        &format!("{subject}.resource"),
                        resource,
                        saved_resource,
                    ),
                    None => report.difference(
                        format!("{subject}.resource"),
                        resource,
                        format!("saved index {index} is not in the recorded resource database"),
                    ),
                }
                report.scalar(&subject, "amount", Some(amount), Some(saved_amount));
            }
        }
    }
    for (tag, resources) in report.canonical_countries(&saved.country_resources) {
        if resources.building_overrides.is_empty() {
            continue;
        }
        let subject = format!("CountryBuildingResources.{}", String::from_utf8_lossy(tag));
        let Some(country) = links
            .values()
            .find(|country| country.tag == *tag)
            .map(|country| country.id)
        else {
            report.difference(subject, "no captured country", "present");
            continue;
        };
        if !read.contains(&country) && !earlier.contains(&country) && !unread.contains(&country) {
            report.limitation(
                "the walk reads building-resource overrides for state controllers only, and this country controls no captured state",
                subject,
            );
        }
    }
    if !earlier.is_empty() {
        report.limitation(
            &format!(
                "{} countries' rows were read before this hour (sliced, daily) and are not compared; compare at a full-refresh hour",
                earlier.len()
            ),
            "CountryBuildingResources",
        );
    }
    if !unread.is_empty() {
        report.limitation(
            &format!(
                "{} state controllers have no rows and no SliceReads hour (the family's scan did not see them), so no read is known to have found them without overrides; not compared",
                unread.len()
            ),
            "CountryBuildingResources",
        );
    }
    report.compared("CountryBuildingResources", compared);
    Ok(())
}

pub fn state_cores(
    rows: &[replay_data::rows::StateCoreRow],
    countries: &BTreeMap<Vec<u8>, content_reader_save::Country>,
    links: &BTreeMap<u32, &CountryRow>,
    reads: &[replay_data::rows::SliceReadRow],
    hour: i32,
    report: &mut Report,
) -> io::Result<()> {
    let sampled = |state: u32| {
        replay_data::sliced::read_hour(reads, replay_data::rows::TableId::StateCores, state)
            == Some(hour)
    };
    let rows: Vec<_> = rows.iter().filter(|row| sampled(row.state)).collect();
    membership(
        "cores",
        rows.iter()
            .map(|row| (row.state, Some(row.cores.as_slice()))),
        countries.iter().map(|(tag, country)| (tag, &country.cores)),
        links,
        &sampled,
        report,
    )?;
    let claims = rows.iter().map(|row| {
        let label = format!("state {} claims", row.state);
        let claims = report.recorded(&row.claims, "StateCores.claims", label);
        (row.state, claims.map(Vec::as_slice))
    });
    let claims: Vec<_> = claims.collect();
    membership(
        "claims",
        claims.into_iter(),
        countries
            .iter()
            .map(|(tag, country)| (tag, &country.claims)),
        links,
        &sampled,
        report,
    )?;
    report.compared("StateCores", rows.len());
    report.limitation(
        "the save writes core and claim membership from the country side, so a state's raw alias spelling and list order are not certified",
        "StateCores",
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn state_strategic_locations(
    rows: &[replay_data::rows::StateCoreRow],
    states: &BTreeMap<u32, content_reader_save::State>,
    provinces: &BTreeMap<u32, content_reader_save::ProvinceOverride>,
    state_of: &BTreeMap<u32, u32>,
    names: &BTreeMap<u32, &[u8]>,
    reads: &[replay_data::rows::SliceReadRow],
    hour: i32,
    report: &mut Report,
) -> io::Result<()> {
    let sampled = |state: u32| {
        replay_data::sliced::read_hour(reads, replay_data::rows::TableId::StateCores, state)
            == Some(hour)
    };
    let spell = |token: u32| -> io::Result<Vec<u8>> {
        names.get(&token).map(|name| name.to_vec()).ok_or_else(|| {
            invalid(format!(
                "strategic location token {token} has no recorded spelling"
            ))
        })
    };
    let mut compared = 0;
    for row in rows.iter().filter(|row| sampled(row.state)) {
        let label = format!("state {} strategic locations", row.state);
        let Some(pairs) = report.recorded(
            &row.strategic_locations,
            "StateCores.strategic_locations",
            label,
        ) else {
            continue;
        };
        let captured = pairs
            .iter()
            .map(|&pair| Ok((spell((pair >> 32) as u32)?, pair as u32)))
            .collect::<io::Result<Vec<_>>>()?;
        let saved = states
            .get(&row.state)
            .map(|state| state.strategic_locations.clone())
            .unwrap_or_default();
        if captured != saved {
            report.difference(
                format!("state {} strategic locations", row.state),
                format!("{captured:?}"),
                format!("{saved:?}"),
            );
        }
        let label = format!("state {} province strategic locations", row.state);
        let Some(entries) = report.recorded(
            &row.province_strategic_locations,
            "StateCores.province_strategic_locations",
            label,
        ) else {
            continue;
        };
        let mut captured: BTreeMap<u32, Vec<Vec<u8>>> = BTreeMap::new();
        for &entry in entries {
            captured
                .entry((entry >> 32) as u32)
                .or_default()
                .push(spell(entry as u32)?);
        }
        let saved: BTreeMap<u32, Vec<Vec<u8>>> = provinces
            .iter()
            .filter(|(id, province)| {
                state_of.get(id) == Some(&row.state) && !province.strategic_locations.is_empty()
            })
            .map(|(&id, province)| (id, province.strategic_locations.clone()))
            .collect();
        if captured != saved {
            report.difference(
                format!("state {} province strategic locations", row.state),
                format!("{captured:?}"),
                format!("{saved:?}"),
            );
        }
        compared += 1;
    }
    report.compared("StateCores strategic locations", compared);
    Ok(())
}

fn membership<'a>(
    kind: &str,
    rows: impl Iterator<Item = (u32, Option<&'a [u32]>)>,
    countries: impl Iterator<Item = (&'a Vec<u8>, &'a Vec<u32>)>,
    links: &BTreeMap<u32, &CountryRow>,
    sampled: &dyn Fn(u32) -> bool,
    report: &mut Report,
) -> io::Result<()> {
    let mut expected: BTreeMap<u32, Vec<&[u8]>> = BTreeMap::new();
    for (tag, states) in countries {
        for &state in states {
            expected.entry(state).or_default().push(tag.as_slice());
        }
    }
    for (state, recorded) in rows {
        let mut saved = expected.remove(&state).unwrap_or_default();
        let Some(recorded) = recorded else {
            continue;
        };
        let mut captured = recorded
            .iter()
            .map(|&link| required_country_tag(link, links))
            .collect::<io::Result<Vec<_>>>()?;
        captured.sort_unstable();
        saved.sort_unstable();
        if captured != saved {
            report.difference(
                format!("state {state} {kind}"),
                format!("{captured:?}"),
                format!("{saved:?}"),
            );
        }
    }
    for state in expected.keys().filter(|state| sampled(**state)) {
        report.difference(
            format!("state {state} {kind}"),
            "no row",
            format!("saved {kind}"),
        );
    }
    Ok(())
}

pub fn state_modifiers(
    rows: &[replay_data::rows::StateDynamicModifierRow],
    saved: &BTreeMap<u32, Vec<content_reader_save::governance::DynamicModifier>>,
    links: &BTreeMap<u32, &CountryRow>,
    sampled: &dyn Fn(u32) -> bool,
    report: &mut Report,
) -> io::Result<()> {
    earlier_states(
        report,
        "StateDynamicModifiers",
        rows.iter()
            .map(|row| row.state)
            .chain(saved.keys().copied()),
        sampled,
    );
    let mut seen = BTreeSet::new();
    let mut matched = 0;
    for row in rows.iter().filter(|row| sampled(row.state)) {
        if !seen.insert((row.state, row.ordinal)) {
            return Err(invalid("duplicate state dynamic modifier ordinal"));
        }
        let subject = format!("state dynamic_modifier.{}.{}", row.state, row.ordinal);
        let Some(modifier) = saved
            .get(&row.state)
            .and_then(|list| list.get(row.ordinal as usize))
        else {
            report.difference(subject, "captured modifier", "missing saved modifier");
            continue;
        };
        matched += 1;
        value(
            report,
            &format!("{subject}.token"),
            row.token.as_slice(),
            modifier.token.as_slice(),
        );
        match (&modifier.scope_country, (row.scope_country as i32) > 0) {
            (Some(tag), true) => value(
                report,
                &format!("{subject}.country"),
                required_country_tag(row.scope_country, links)?,
                tag.as_slice(),
            ),
            (None, false) => {}
            (saved, _) => report.difference(
                format!("{subject}.country"),
                row.scope_country,
                format!("{saved:?}"),
            ),
        }
        match modifier.scope_state {
            Some(state) => value(report, &format!("{subject}.state"), row.scope_state, state),
            None if row.scope_state > 0 => report.difference(
                format!("{subject}.state"),
                row.scope_state,
                "missing positive state",
            ),
            None => {}
        }
    }
    missing(
        report,
        "state dynamic_modifier",
        &seen,
        saved
            .iter()
            .filter(|(state, _)| sampled(**state))
            .flat_map(|(&state, list)| (0..list.len() as u32).map(move |ordinal| (state, ordinal))),
    );
    report.compared("StateDynamicModifiers", matched);
    report.limitation(
        "save omission proves only a nonpositive scope; the raw omitted values stay captured",
        "StateDynamicModifiers",
    );
    Ok(())
}
