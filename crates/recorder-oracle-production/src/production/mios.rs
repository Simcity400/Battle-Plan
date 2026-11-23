use super::*;
use engine_executable::modifier_keys;
use replay_data::rows::Recorded;

const UNSELECTED_POLICY: u32 = 0x4c17;

pub(super) fn compare(
    frame: &DecodedFrame,
    saved: &production::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let countries = saved
        .countries
        .as_ref()
        .ok_or_else(|| invalid("save lacks organisation countries"))?;
    if !report.refresh_is_current(TableId::IndustrialOrganisationDetails, frame.hour) {
        report.limitation(
            "MIO details are a daily census",
            format!(
                "IndustrialOrganisationDetails refreshes on {}",
                replay_data::cadence::refresh_text(TableId::IndustrialOrganisationDetails)
            ),
        );
        return Ok(());
    }
    let mut policies = BTreeMap::new();
    for row in complete::<tables::IndustrialOrganisationPolicies>(frame)? {
        if policies
            .insert(required_country_tag(row.country, links)?, row)
            .is_some()
        {
            return Err(invalid("duplicate country MIO policies"));
        }
    }
    let mut rows = BTreeMap::new();
    for row in complete::<tables::IndustrialOrganisationDetails>(frame)? {
        if rows.insert(row.organisation, row).is_some() {
            return Err(invalid("duplicate organisation details"));
        }
    }
    let name = |token: u32| -> io::Result<&[u8]> {
        tokens
            .get(&token)
            .copied()
            .map(content_reader_parser::parser::c_string)
            .ok_or_else(|| {
                invalid(format!(
                    "organisation token {token} missing from LexicalTokens"
                ))
            })
    };
    let names = |list: &[u32]| -> io::Result<Vec<Vec<u8>>> {
        list.iter()
            .map(|&token| name(token).map(<[u8]>::to_vec))
            .collect()
    };
    let mut design_costs = BTreeMap::<&[u8], Vec<(u32, i64)>>::new();
    if let Ok(rows) = frame.complete::<tables::DesignCountryModifiers>() {
        for row in rows.iter().filter(|row| {
            row.modifier == modifier_keys::MILITARY_INDUSTRIAL_ORGANIZATION_POLICY_COST
                && row.failure == 0
        }) {
            if let Some(value) = row.value {
                design_costs
                    .entry(required_country_tag(row.country, links)?)
                    .or_default()
                    .push((row.country, value));
            }
        }
    }
    let mut matched = 0;
    let mut policy_matched = 0;
    for (tag, country) in report.canonical_countries(countries) {
        let Some(production) = &country.production else {
            continue;
        };
        let policy = policies
            .remove(tag.as_slice())
            .ok_or_else(|| invalid(format!("country {tag:?} MIO policies absent")))?;
        policy_matched += 1;
        match policy.cost_modifier_raw {
            Recorded::NotRecorded => report.difference(
                format!("country {tag:?}.policy_cost_modifier"),
                "not recorded",
                "readable",
            ),
            Recorded::Value(None) => report.difference(
                format!("country {tag:?}.policy_cost_modifier"),
                "unread",
                "readable",
            ),
            Recorded::Value(Some(value)) => {
                for (raw, design) in design_costs.get(tag.as_slice()).into_iter().flatten() {
                    equal(
                        report,
                        format!("country {tag:?}.policy_cost_modifier (design read {raw})"),
                        value,
                        *design,
                    );
                }
            }
        }
        equal(
            report,
            format!("country {tag:?}.policies"),
            policy
                .policies
                .iter()
                .map(|row| Ok((name(row.token)?.to_vec(), row.cost, row.days)))
                .collect::<io::Result<Vec<_>>>()?,
            production
                .policies
                .iter()
                .map(|row| (row.key.clone(), row.cost, row.cooldown_days))
                .collect(),
        );
        for expected in &production.organisations {
            let label = format!("organisation {}", expected.id);
            let Some(actual) = rows.remove(&expected.id) else {
                report.difference(&label, "details absent", "present");
                continue;
            };
            matched += 1;
            equal(
                report,
                format!("{label}.holding_country"),
                required_country_tag(actual.country, links)?,
                tag.as_slice(),
            );
            let state = &actual.state;
            let saved = &expected.details;
            for (field, value) in [
                ("research_bonus", state.research_bonus_raw),
                ("funds", state.funds_raw),
                ("research_assign_cost", state.research_assign_cost_raw),
                ("production_assign_cost", state.production_assign_cost_raw),
                ("add_mio_funds_gain_factor", state.funds_gain_factor_raw),
            ] {
                equal(
                    report,
                    format!("{label}.{field}"),
                    Some(value),
                    saved.fixed.get(field.as_bytes()).map(|value| value.raw),
                );
            }
            for (field, value) in [
                ("task_capacity", state.task_capacity),
                ("size", state.size),
                ("points", state.points),
                ("design_team_change_cost", state.design_team_change_cost),
            ] {
                equal(
                    report,
                    format!("{label}.{field}"),
                    Some(value),
                    saved.integers.get(field.as_bytes()).copied(),
                );
            }
            equal(
                report,
                format!("{label}.upgrades"),
                Some(state.upgrades != 0),
                saved.upgrades,
            );
            equal(
                report,
                format!("{label}.unlocked"),
                names(&actual.unlocked)?,
                saved.unlocked.clone(),
            );
            equal(
                report,
                format!("{label}.queued_trait"),
                names(&actual.queued)?,
                saved.queued.clone(),
            );
            equal(
                report,
                format!("{label}.allowed_policies"),
                names(&actual.allowed_policies)?,
                saved.allowed_policies.clone(),
            );
            equal(
                report,
                format!("{label}.policy"),
                if state.policy == UNSELECTED_POLICY {
                    None
                } else {
                    Some(name(state.policy)?.to_vec())
                },
                saved.policy.clone(),
            );
            equal(
                report,
                format!("{label}.cooldown"),
                state.cooldown_hour,
                saved.cooldown.map(|date| date.engine_hour()).transpose()?,
            );
            let variables = saved.variables.as_ref().ok_or_else(|| {
                invalid(format!("{label}.variables missing unconditional payload"))
            })?;
            equal(
                report,
                format!("{label}.variables.seed"),
                actual.variables.seed,
                Some(variables.seed.unwrap_or(1)),
            );
            let mut captured = actual
                .variables
                .entries
                .iter()
                .map(|entry| {
                    (
                        content_reader_parser::parser::c_string(&entry.name).to_vec(),
                        entry.value_raw,
                    )
                })
                .collect::<Vec<_>>();
            let mut expected_vars = variables
                .entries
                .iter()
                .map(|entry| (entry.name.clone(), entry.value.raw))
                .collect::<Vec<_>>();
            captured.sort();
            expected_vars.sort();
            equal(
                report,
                format!("{label}.variables"),
                captured,
                expected_vars,
            );
            if variables.reserved_entries != 0 {
                return Err(invalid(format!(
                    "{label}: {} reserved-name variable emissions cannot be joined",
                    variables.reserved_entries
                )));
            }
            let flags = saved
                .flags
                .as_ref()
                .ok_or_else(|| invalid(format!("{label}.flags missing unconditional payload")))?;
            let mut captured_flags = BTreeMap::new();
            for flag in &actual.flags {
                let key = name(flag.token)?.to_vec();
                if captured_flags.insert(key, flag).is_some() {
                    return Err(invalid(
                        "organisation flags have duplicate serialized names",
                    ));
                }
            }
            for flag in flags {
                let Some(actual) = captured_flags.remove(&flag.name) else {
                    report.difference(format!("{label}.flag {:?}", flag.name), "absent", "present");
                    continue;
                };
                equal(
                    report,
                    format!("{label}.flag {:?}.value", flag.name),
                    actual.value,
                    flag.value,
                );
                equal(
                    report,
                    format!("{label}.flag {:?}.date", flag.name),
                    actual.set_hour,
                    flag.date.engine_hour()?,
                );
                equal(
                    report,
                    format!("{label}.flag {:?}.days", flag.name),
                    (actual.remaining_days > 0).then_some(actual.remaining_days),
                    flag.remaining_days,
                );
            }
            for key in captured_flags.keys() {
                report.difference(format!("{label}.flag {key:?}"), "present", "absent");
            }
        }
    }
    for key in rows.keys() {
        report.difference(format!("organisation {key}.details"), "present", "absent");
    }
    for tag in policies.keys() {
        report.difference(format!("country {tag:?}.policies"), "present", "absent");
    }
    report.compared("IndustrialOrganisationDetails", matched);
    report.compared("IndustrialOrganisationPolicies", policy_matched);
    report.limitation("policy country-context links, cached localised names, cached selection scores and the country's effective policy-cost modifier are runtime values omitted by the policy save writer", "IndustrialOrganisationPolicies");
    report.limitation("trait definitions, local coordinates, relative anchors, task slots, queue cursor, size-cost factor and source locations are runtime/content operands omitted by the organisation save writer", "IndustrialOrganisationDetails");
    Ok(())
}

pub(super) fn compare_designs(
    frame: &DecodedFrame,
    saved: &production::Pack,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    if !report.refresh_is_current(TableId::VariantStats, frame.hour) {
        return Ok(());
    }
    let saved = saved
        .variants
        .as_ref()
        .ok_or_else(|| invalid("save lacks equipment variants"))?;
    let mut matched = 0;
    for actual in complete::<tables::VariantStats>(frame)? {
        let id = actual.variant;
        let Some(expected) = saved.get(&id) else {
            continue;
        };
        let label = format!("variant {id}");
        equal(
            report,
            format!("{label}.number_of_design_team_traits"),
            actual.organisation_trait_count,
            expected.organisation_marker.unwrap_or(0),
        );
        let definition = complete::<tables::VariantDefinitions>(frame)?
            .iter()
            .find(|row| row.id == id)
            .ok_or_else(|| invalid("design author definition absent"))?;
        let design = definition
            .design
            .require("VariantDefinitions.design")
            .map_err(invalid)?;
        if design.design_team != actual.organisation {
            return Err(invalid(
                "design author witness and frozen author sample differ",
            ));
        }
        let author = super::variants::author_live(frame, actual.organisation)?
            .then_some(actual.organisation);
        equal(
            report,
            format!("{label}.design_team"),
            author,
            expected.organisation,
        );
        let bonuses = actual
            .organisation_bonuses
            .iter()
            .map(|bonus| {
                let name = tokens
                    .get(&bonus.key)
                    .copied()
                    .ok_or_else(|| invalid(format!("design bonus token {} absent", bonus.key)))?;
                Ok((
                    content_reader_parser::parser::c_string(name).to_vec(),
                    bonus.value_raw,
                ))
            })
            .collect::<io::Result<Vec<_>>>()?;
        equal(
            report,
            format!("{label}.design_team_bonus"),
            bonuses,
            expected
                .organisation_bonuses
                .iter()
                .map(|(key, value)| (key.clone(), value.raw))
                .collect(),
        );

        matched += 1;
    }
    report.compared("VariantStats.organisation", matched);
    Ok(())
}
