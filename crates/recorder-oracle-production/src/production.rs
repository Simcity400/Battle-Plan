
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    io,
};

use content_reader_save::{Save, production};
use replay::campaign::Replay;
use replay_data::{Hour, rows::CountryRow, rows::ProductionLineRow, rows::TableId, rows::tables};
use replay_frame::frame::DecodedFrame;

use super::{Report, TokenNames, complete, country_tag, daily_read, invalid, required_country_tag};

type ManpowerEntries = BTreeMap<(u64, u8), Vec<(u32, u32, i32)>>;

fn equal<T: std::fmt::Debug + PartialEq>(
    report: &mut Report,
    label: impl AsRef<str>,
    actual: T,
    saved: T,
) {
    if actual != saved {
        report.difference(label, actual, saved);
    }
}

pub fn compare(
    frame: &DecodedFrame,
    saved: &Save,
    links: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let mut complete = report.attempt("ProductionLines", |report| {
        compare_lines(frame, saved, links, report)
    });
    if daily_read(&frame.metrics, TableId::ProductionLineModifiers) {
        complete &= report.attempt("ProductionLineModifiers", |report| {
            compare_modifiers(frame, &saved.production, links, report)
        });
    }
    complete &= report.attempt("AvailableEquipmentVariants", |report| {
        compare_available_variants(frame, &saved.production, links, report)
    });
    complete &= report.attempt("Stockpiles", |report| {
        compare_stockpiles(frame, saved, links, report)
    });
    complete &= report.attempt("DivisionEquipment", |report| {
        compare_division_equipment(frame, saved, links, report)
    });
    complete &= report.attempt("Variants", |report| {
        compare_variants(frame, &saved.production, links, tokens, report)
    });
    complete &= report.attempt("DesignCountryTemplates", |report| {
        compare_design_templates(frame, &saved.production, links, tokens, report)
    });
    complete &= report.attempt("ArmyUpgrades", |report| {
        compare_army_upgrades(frame, &saved.production, links, report)
    });
    complete &= report.attempt("ProductionLicenses", |report| {
        compare_licenses(frame, &saved.production, links, report)
    });
    complete &= report.attempt("ProductionLicenseRecords", |report| {
        compare_license_records(frame, &saved.production, links, report)
    });
    complete &= report.attempt("Deployment", |report| {
        compare_deployment(frame, &saved.production, links, tokens, report)
    });
    complete &= report.attempt("IndustrialOrganisations", |report| {
        compare_organisations(frame, &saved.production, links, tokens, report)
    });
    complete &= report.attempt("IndustrialOrganisationDetails", |report| {
        mios::compare(frame, &saved.production, links, tokens, report)
    });
    complete &= report.attempt("producer ProductionLineOutput", |report| {
        compare_line_output_countries(frame, &saved.production, links, report)
    });
    report.limitation(
        "serialized country tags compare resolved country identity and cannot certify equivalent raw country-link aliases",
        "ProductionLines, ProductionLineOutput, AvailableEquipmentVariants, Stockpiles, DivisionEquipment, VariantDefinitions, ArmyUpgradeGroups, ArmyUpgradeRequests, ArmyUpgradeDeliveries, ArmyUpgradeProduced, ProductionLicenses, DeploymentConveyors, DeploymentLines, DeploymentManpower, IndustrialOrganisations",
    );
    if complete {
        Ok(())
    } else {
        Err(invalid("one or more production comparisons failed"))
    }
}

fn compare_design_templates(
    frame: &DecodedFrame,
    saved: &production::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let countries = saved
        .countries
        .as_ref()
        .ok_or_else(|| invalid("save lacks design discount owners"))?;
    let demanded: BTreeSet<_> = complete::<tables::VariantDefinitions>(frame)?
        .iter()
        .map(|row| row.creator_link)
        .collect();
    let actual = complete::<tables::DesignCountryTemplates>(frame)?;
    let mut seen = BTreeSet::new();
    for row in actual {
        if !demanded.contains(&row.country) || !seen.insert(row.country) {
            return Err(invalid(
                "design discount owner roster differs from saved design demand",
            ));
        }
        if row.failure != 0 {
            return Err(invalid(format!(
                "design discount country {} acquisition failed ({})",
                row.country, row.failure
            )));
        }
        let tag = required_country_tag(row.country, links)?;
        let expected = countries
            .get(tag)
            .and_then(|country| country.production.as_ref())
            .ok_or_else(|| invalid("saved design discount country production absent"))?;
        let entries = row
            .entries()
            .map(|(uses, factor, equipment)| {
                Ok((
                    uses,
                    factor,
                    equipment
                        .iter()
                        .map(|token| {
                            tokens
                                .get(token)
                                .copied()
                                .map(content_reader_parser::parser::c_string)
                                .map(<[u8]>::to_vec)
                                .ok_or_else(|| {
                                    invalid(format!(
                                        "design discount token {token} has no spelling"
                                    ))
                                })
                        })
                        .collect::<io::Result<Vec<_>>>()?,
                ))
            })
            .collect::<io::Result<Vec<_>>>()?;
        equal(
            report,
            format!("country {tag:?}.design_discounts"),
            entries,
            expected
                .design_templates
                .iter()
                .map(|entry| (entry.uses, entry.factor.raw, entry.equipment.clone()))
                .collect(),
        );
    }
    if seen != demanded {
        return Err(invalid("design discount demanded country absent"));
    }
    report.compared("DesignCountryTemplates", actual.len());
    Ok(())
}

fn compare_modifiers(
    frame: &DecodedFrame,
    saved: &production::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let countries = saved
        .countries
        .as_ref()
        .ok_or_else(|| invalid("save lacks production country roster"))?;
    let mut rows = complete::<tables::ProductionLineModifiers>(frame)?
        .iter()
        .map(|row| (row.line, row))
        .collect::<BTreeMap<_, _>>();
    let mut matched = 0;
    for (tag, country) in countries {
        let country = country
            .production
            .as_ref()
            .ok_or_else(|| invalid("saved country lacks production"))?;
        for expected in &country.lines {
            let label = format!("production modifiers {}", expected.base.id);
            let Some(row) = rows.remove(&expected.base.id) else {
                report.difference(label, "absent", "present");
                continue;
            };
            equal(
                report,
                format!("{label}.country"),
                required_country_tag(row.country, links)?,
                tag.as_slice(),
            );
            equal(report, format!("{label}.kind"), row.kind, expected.kind);
            equal(
                report,
                format!("{label}.active_factories"),
                row.active_factories,
                expected.base.active_factories.unwrap_or(0),
            );
            equal(
                report,
                format!("{label}.variant"),
                row.variant,
                expected.variant,
            );
            equal(
                report,
                format!("{label}.organisation"),
                row.organisation,
                expected.mio.unwrap_or(0),
            );
            equal(
                report,
                format!("{label}.factory_efficiencies"),
                row.factory_efficiencies.as_slice(),
                expected.factory_efficiencies.as_slice(),
            );
            match row.converting {
                replay_data::rows::Recorded::Value(converting) => equal(
                    report,
                    format!("{label}.converting"),
                    converting,
                    expected.converting,
                ),
                replay_data::rows::Recorded::NotRecorded => report.limitation(
                    "the line's conversion control is not recorded at this recording's format",
                    "ProductionLineModifiers.converting",
                ),
            }
            matched += 1;
        }
    }
    for line in rows.keys() {
        report.difference(format!("production modifiers {line}"), "present", "absent");
    }
    report.compared("ProductionLineModifiers", matched);
    report.limitation("native country source trees, runtime defines, power, MIO cache/dirty state, line resource operand and derived production predicates are not certified by this saved factory-array comparison", "CountryProductionModifiers, ProductionLineModifiers");
    Ok(())
}

fn compare_line_output_countries(
    frame: &DecodedFrame,
    saved: &production::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let countries = saved
        .countries
        .as_ref()
        .ok_or_else(|| invalid("save lacks production country roster"))?;
    let mut saved_lines = BTreeMap::new();
    for (tag, country) in report.canonical_countries(countries) {
        let production = country
            .production
            .as_ref()
            .ok_or_else(|| invalid("saved country lacks serialized production"))?;
        for line in &production.lines {
            if saved_lines.insert(line.base.id, tag.as_slice()).is_some() {
                return Err(invalid("duplicate full saved production line handle"));
            }
        }
    }
    let mut seen = BTreeSet::new();
    let mut matched = 0;
    let mut ended = 0;
    for row in complete::<tables::ProductionLineOutput>(frame)? {
        let label = format!("production line output {}:{}", row.line, row.ordinal);
        if let Some(&tag) = saved_lines.get(&row.line) {
            seen.insert(row.line);
            matched += 1;
            equal(
                report,
                format!("{label}.country"),
                required_country_tag(row.country, links)?,
                tag,
            );
            equal(report, format!("{label}.ended"), row.ended, false);
        } else if row.ended {
            ended += 1;
        } else {
            report.difference(label, "live", "absent from saved production");
        }
    }
    for line in saved_lines.keys().filter(|line| !seen.contains(line)) {
        report.difference(
            format!("production line output {line}"),
            "absent",
            "live in saved production",
        );
    }
    if ended != 0 {
        report.limitation(
            "ended production lines are absent from the save, so their producing countries cannot be checked",
            format!("ProductionLineOutput: {ended} rows"),
        );
    }
    report.compared("ProductionLineOutput.country", matched);
    Ok(())
}

fn compare_organisations(
    frame: &DecodedFrame,
    saved: &production::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let countries = saved
        .countries
        .as_ref()
        .ok_or_else(|| invalid("save lacks production country roster"))?;
    let mut organisations = BTreeMap::new();
    for row in complete::<tables::IndustrialOrganisations>(frame)? {
        if row.organisation == 0 || organisations.insert(row.organisation, row).is_some() {
            return Err(invalid("null or duplicate organisation reference"));
        }
    }
    let mut history = BTreeMap::new();
    for row in complete::<tables::IndustrialOrganisationHistory>(frame)? {
        if history
            .insert((row.organisation, row.variant), row)
            .is_some()
        {
            return Err(invalid("duplicate organisation history key"));
        }
    }
    let (mut matched, mut history_matched) = (0, 0);
    for (tag, country) in report.canonical_countries(countries) {
        let Some(production) = country.production.as_ref() else {
            continue;
        };
        for (ordinal, expected) in production.organisations.iter().enumerate() {
            let label = format!("organisation {}", expected.id);
            let Some(actual) = organisations.remove(&expected.id) else {
                report.difference(label, "absent", "present");
                continue;
            };
            matched += 1;
            for (field, reference) in [("country", actual.country), ("owner", actual.owner)] {
                equal(
                    report,
                    format!("{label}.{field}"),
                    required_country_tag(reference, links)?,
                    tag.as_slice(),
                );
            }
            equal(
                report,
                format!("{label}.ordinal"),
                actual.ordinal as usize,
                ordinal,
            );
            compare_token(
                tokens,
                actual.token,
                &expected.key,
                &format!("{label}.key"),
                TokenWriter::Token,
                report,
            )?;
            for (field, captured, saved) in [
                ("name", &actual.name, &expected.name),
                ("icon", &actual.icon, &expected.icon),
            ] {
                let label = format!("{label}.{field}");
                let Some(saved) = saved else {
                    report.difference(label, "written", "absent");
                    continue;
                };
                let end = captured
                    .iter()
                    .position(|&byte| byte == 0)
                    .unwrap_or(captured.len());
                equal(report, &label, &captured[..end], saved.as_slice());
                if end < captured.len() {
                    report.limitation("text string serialization stops at the first NUL; the captured suffix is not present in the save", label);
                }
            }
            for entry in &expected.history {
                let label = format!("{label}.history {}", entry.variant);
                match history.remove(&(expected.id, entry.variant)) {
                    Some(actual) => {
                        history_matched += 1;
                        equal(report, format!("{label}.units"), actual.units, entry.units);
                        equal(
                            report,
                            format!("{label}.design_date"),
                            actual.design_hour,
                            entry
                                .design_date
                                .map(|date| date.engine_hour())
                                .transpose()?
                                .into(),
                        );
                    }
                    None => report.difference(label, "absent", "present"),
                }
            }
        }
    }
    for id in organisations.keys() {
        report.difference(format!("organisation {id}"), "present", "absent");
    }
    for (organisation, variant) in history.keys() {
        report.difference(
            format!("organisation {organisation}.history {variant}"),
            "present",
            "absent",
        );
    }
    report.compared("IndustrialOrganisations", matched);
    report.compared("IndustrialOrganisationHistory", history_matched);
    Ok(())
}

mod mios;

mod line_output;
#[cfg(test)]
use line_output::LINE_OUTPUT_TABLES;
pub use line_output::compare_line_output;

fn compare_available_variants(
    frame: &DecodedFrame,
    saved: &production::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let countries = saved
        .countries
        .as_ref()
        .ok_or_else(|| invalid("save lacks production country roster"))?;
    let mut recorded = BTreeMap::<(Vec<u8>, u8), Vec<(u32, u64)>>::new();
    let mut candidates = 0;
    for row in complete::<tables::AvailableEquipmentVariants>(frame)? {
        if row.roster_kind == 2 {
            candidates += 1;
            continue;
        }
        recorded
            .entry((
                required_country_tag(row.country, links)?.to_vec(),
                row.roster_kind,
            ))
            .or_default()
            .push((row.ordinal, row.variant));
    }
    let mut compared = 0;
    for (tag, country) in report.canonical_countries(countries) {
        let production = country.production.as_ref().ok_or_else(|| {
            invalid(format!(
                "{} lacks serialized production",
                String::from_utf8_lossy(tag)
            ))
        })?;
        for (roster_kind, name, expected) in [
            (
                0,
                "available_equipments",
                production.available_variants.as_deref(),
            ),
            (
                1,
                "foreign_lease_equipments",
                Some(production.foreign_lease_variants.as_slice()),
            ),
        ] {
            let label = format!("{}.{name}", String::from_utf8_lossy(tag));
            let expected =
                expected.ok_or_else(|| invalid(format!("{label} wrapper absent from save")))?;
            let actual = recorded
                .remove(&(tag.clone(), roster_kind))
                .unwrap_or_default();
            if actual
                .iter()
                .enumerate()
                .any(|(index, (ordinal, _))| *ordinal as usize != index)
            {
                return Err(invalid(format!("{label} is not a dense native order")));
            }
            equal(
                report,
                format!("{label}.count"),
                actual.len(),
                expected.len(),
            );
            for (index, ((_, actual), expected)) in actual.iter().zip(expected).enumerate() {
                compared += 1;
                equal(report, format!("{label}[{index}]"), *actual, *expected);
            }
        }
    }
    for ((tag, roster_kind), rows) in recorded {
        report.difference(
            format!("{} roster {roster_kind}", String::from_utf8_lossy(&tag)),
            rows.len(),
            "country absent from the saved production roster",
        );
    }
    report.compared("AvailableEquipmentVariants", compared);
    report.limitation(
        "the best-variant candidate list has no serializer and is rebuilt from the rosters on load, so its rows decode but are not certified against a save",
        format!("AvailableEquipmentVariants best-variant candidates: {candidates} decoded rows"),
    );
    Ok(())
}

fn compare_licenses(
    frame: &DecodedFrame,
    saved: &production::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let countries = saved
        .countries
        .as_ref()
        .ok_or_else(|| invalid("save lacks production country roster"))?;
    let mut recorded: BTreeMap<Vec<u8>, Vec<(u32, u64)>> = BTreeMap::new();
    for row in complete::<tables::ProductionLicenses>(frame)? {
        recorded
            .entry(required_country_tag(row.country, links)?.to_vec())
            .or_default()
            .push((row.ordinal, row.variant));
    }
    for (tag, entries) in &recorded {
        for (position, (ordinal, _)) in entries.iter().enumerate() {
            if *ordinal as usize != position {
                return Err(invalid(format!(
                    "captured licences for {} are not a dense ordinal sequence",
                    String::from_utf8_lossy(tag)
                )));
            }
        }
    }
    let mut compared = 0;
    for (tag, country) in report.canonical_countries(countries) {
        let Some(production) = country.production.as_ref() else {
            continue;
        };
        let label = format!("{} production licences", String::from_utf8_lossy(tag));
        let actual = recorded.remove(tag).unwrap_or_default();
        let Some(expected) = production.licenses.as_ref() else {
            report.difference(label, actual.len(), "missing serialized licence container");
            continue;
        };
        if actual.len() != expected.len() {
            report.difference(format!("{label}.count"), actual.len(), expected.len());
        }
        for (index, ((_, actual), expected)) in actual.iter().zip(expected).enumerate() {
            compared += 1;
            equal(report, format!("{label} {index}"), *actual, *expected);
        }
    }
    for (tag, entries) in recorded {
        report.difference(
            format!("{} production licences", String::from_utf8_lossy(&tag)),
            entries.len(),
            "country absent from the saved production roster",
        );
    }
    report.compared("ProductionLicenses", compared);
    Ok(())
}

fn compare_license_records(
    frame: &DecodedFrame,
    saved: &production::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let countries = saved
        .countries
        .as_ref()
        .ok_or_else(|| invalid("save lacks production country roster"))?;
    let mut recorded: BTreeMap<Vec<u8>, Vec<&replay_data::rows::ProductionLicenseRecordRow>> =
        BTreeMap::new();
    for row in complete::<tables::ProductionLicenseRecords>(frame)? {
        recorded
            .entry(required_country_tag(row.owner, links)?.to_vec())
            .or_default()
            .push(row);
    }
    for (tag, entries) in &recorded {
        for (position, row) in entries.iter().enumerate() {
            if row.ordinal as usize != position {
                return Err(invalid(format!(
                    "captured licence records for {} are not a dense ordinal sequence",
                    String::from_utf8_lossy(tag)
                )));
            }
        }
    }
    let mut compared = 0;
    for (tag, country) in report.canonical_countries(countries) {
        let Some(production) = country.production.as_ref() else {
            continue;
        };
        let label = format!("{} licence records", String::from_utf8_lossy(tag));
        let actual = recorded.remove(tag).unwrap_or_default();
        let Some(expected) = production.license_records.as_ref() else {
            report.difference(label, actual.len(), "missing serialized licence container");
            continue;
        };
        if actual.len() != expected.len() {
            report.difference(format!("{label}.count"), actual.len(), expected.len());
        }
        for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
            compared += 1;
            let field = format!("{label} {index}");
            equal(
                report,
                format!("{field}.owner"),
                required_country_tag(actual.owner, links)?,
                expected.owner.as_slice(),
            );
            equal(
                report,
                format!("{field}.giver"),
                required_country_tag(actual.giver, links)?,
                expected.giver.as_slice(),
            );
            equal(
                report,
                format!("{field}.lended_cic"),
                actual.lended_cic,
                expected.lended_cic,
            );
            equal(
                report,
                format!("{field}.required_cic"),
                actual.required_cic,
                expected.required_cic,
            );
            equal(
                report,
                format!("{field}.start_date"),
                actual.start_date,
                expected.start_date.engine_hour()?,
            );
            equal(
                report,
                format!("{field}.parent"),
                actual.parent,
                expected.parent,
            );
            equal(
                report,
                format!("{field}.equipment"),
                actual.equipment.as_slice(),
                expected.equipment.as_slice(),
            );
        }
    }
    for (tag, entries) in recorded {
        report.difference(
            format!("{} licence records", String::from_utf8_lossy(&tag)),
            entries.len(),
            "country absent from the saved production roster",
        );
    }
    report.compared("ProductionLicenseRecords", compared);
    report.limitation(
        "the engine also keeps every licence in the giver's reverse licence array; that index is never serialized and is not recorded, so a licence is compared once, from its owner's roster. Serializer emits no counterpart for the qword, which is recorded raw",
        "ProductionLicenseRecords, ProductionLicenseRecords.operand_raw",
    );
    Ok(())
}

fn compare_deployment(
    frame: &DecodedFrame,
    saved: &production::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let countries = saved
        .countries
        .as_ref()
        .ok_or_else(|| invalid("save lacks production country roster"))?;
    let mut conveyors = BTreeMap::new();
    for row in complete::<tables::DeploymentConveyors>(frame)? {
        if conveyors.insert(row.handle, row).is_some() {
            return Err(invalid("duplicate full deployment conveyor handle"));
        }
    }
    let mut lines = BTreeMap::new();
    for row in complete::<tables::DeploymentLines>(frame)? {
        if lines.insert(row.handle, row).is_some() {
            return Err(invalid("duplicate full deployment line handle"));
        }
    }
    let mut manpower: ManpowerEntries = BTreeMap::new();
    for row in complete::<tables::DeploymentManpower>(frame)? {
        manpower.entry((row.line, row.side)).or_default().push((
            row.ordinal,
            row.source_country,
            row.amount,
        ));
    }
    let mut equipment: BTreeMap<u64, Vec<(u32, u64, i64)>> = BTreeMap::new();
    for row in complete::<tables::DeploymentEquipment>(frame)? {
        equipment.entry(row.line).or_default().push((
            row.ordinal,
            row.variant_handle,
            row.amount_raw,
        ));
    }
    let orphans: BTreeSet<_> = manpower
        .keys()
        .map(|&(line, _)| ("DeploymentManpower", line))
        .chain(equipment.keys().map(|&line| ("DeploymentEquipment", line)))
        .filter(|(_, line)| !lines.contains_key(line))
        .collect();
    for (table, line) in orphans {
        report.difference(
            format!("{table} line {line}"),
            "present",
            "no captured deployment line",
        );
    }

    let mut compared = 0;
    let mut missing_role_tokens = 0;
    let mut undecidable_pools = 0;
    for (tag, country) in report.canonical_countries(countries) {
        let Some(expected_queue) = country.deployment.as_ref() else {
            continue;
        };
        for (ordinal, expected) in expected_queue.iter().enumerate() {
            let label = format!(
                "{} deployment conveyor {}",
                String::from_utf8_lossy(tag),
                expected.id
            );
            let Some(actual) = conveyors.remove(&expected.id) else {
                report.difference(label, "absent", "present");
                continue;
            };
            compared += 1;
            equal(
                report,
                format!("{label}.country"),
                required_country_tag(actual.country, links)?,
                tag.as_slice(),
            );
            equal(
                report,
                format!("{label}.ordinal"),
                actual.ordinal as usize,
                ordinal,
            );
            equal(
                report,
                format!("{label}.template"),
                actual.template,
                expected.template,
            );
            let name_label = format!("{label}.name");
            let name = super::serialized_text(report, &name_label, &actual.name);
            equal(report, name_label, name, expected.name.as_slice());
            equal(
                report,
                format!("{label}.amount"),
                actual.amount,
                expected.amount,
            );
            equal(
                report,
                format!("{label}.location"),
                actual.location,
                expected.location,
            );
            equal(
                report,
                format!("{label}.priority"),
                actual.priority,
                expected.priority,
            );
            match (tokens.get(&actual.role_token), expected.role.as_ref()) {
                (Some(text), Some(role)) => {
                    if *text != role.as_slice() {
                        report.difference(
                            format!("{label}.role"),
                            text.escape_ascii().to_string(),
                            role.escape_ascii().to_string(),
                        );
                    }
                }
                (None, _) => missing_role_tokens += 1,
                (_, None) => report.difference(
                    format!("{label}.role"),
                    actual.role_token,
                    "missing serialized role",
                ),
            }
            match expected.closed {
                Some(closed) => equal(
                    report,
                    format!("{label}.closed"),
                    actual.closed != 0,
                    closed,
                ),
                None => report.difference(
                    format!("{label}.closed"),
                    actual.closed,
                    "missing serialized closed flag",
                ),
            }
            match (&actual.exile_country, &expected.government_in_exile_tag) {
                (None, None) => {}
                (Some(raw), Some(tag)) => {
                    let captured = country_tag(*raw, links)?.ok_or_else(|| {
                        invalid("captured exile country reference resolves to none")
                    })?;
                    equal(
                        report,
                        format!("{label}.government_in_exile_tag"),
                        captured,
                        tag.as_slice(),
                    );
                }
                (None, Some(tag)) => {
                    report.difference(
                        format!("{label}.government_in_exile_tag"),
                        "absent",
                        String::from_utf8_lossy(tag).into_owned(),
                    );
                }
                (Some(raw), None) => {
                    report.difference(
                        format!("{label}.government_in_exile_tag"),
                        format!("present reference {raw}"),
                        "omitted, so not positive",
                    );
                }
            }
            compare_lines_of(
                &label,
                actual.handle,
                expected,
                &mut lines,
                &manpower,
                &equipment,
                links,
                report,
                &mut undecidable_pools,
            )?;
        }
    }
    for row in conveyors.values() {
        report.difference(
            format!("deployment conveyor {}", row.handle),
            "present",
            "absent from the saved deployment queue",
        );
    }
    for row in lines.values() {
        report.difference(
            format!("deployment line {}", row.handle),
            "present",
            "absent from the saved deployment queue",
        );
    }
    report.compared("DeploymentConveyors", compared);
    if missing_role_tokens != 0 {
        report.limitation(
            "the recorded lexical dictionary lacks a spelling for one or more conveyor role tokens; the raw token is kept and no role text is inferred from game content",
            format!("DeploymentConveyors.role_token: {missing_role_tokens} unresolved"),
        );
    }
    report.limitation(
        "The deployment writer omits training, max_training and max_manpower unless the stored word is strictly positive, and the game omits a manpower entry unless its count is strictly positive; omitted values therefore prove only that the captured word is not positive",
        "DeploymentLines.training_raw, max_training_raw, max_manpower; DeploymentManpower.amount",
    );
    if undecidable_pools != 0 {
        report.limitation(
            "an omitted equipment wrapper is gated by the game on the pool's allocation array, which this table does not record; the omission is only decidable when the captured allocation count is zero",
            format!("DeploymentEquipment: {undecidable_pools} undecidable omissions"),
        );
    }
    Ok(())
}

fn compare_lines_of(
    parent: &str,
    conveyor: u64,
    expected: &production::Conveyor,
    lines: &mut BTreeMap<u64, &replay_data::rows::DeploymentLineRow>,
    manpower: &ManpowerEntries,
    equipment: &BTreeMap<u64, Vec<(u32, u64, i64)>>,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
    undecidable_pools: &mut usize,
) -> io::Result<()> {
    for (ordinal, expected) in expected.lines.iter().enumerate() {
        let label = format!("{parent} line {}", expected.id);
        let Some(actual) = lines.remove(&expected.id) else {
            report.difference(label, "absent", "present");
            continue;
        };
        equal(
            report,
            format!("{label}.conveyor"),
            actual.conveyor,
            conveyor,
        );
        equal(
            report,
            format!("{label}.ordinal"),
            actual.ordinal as usize,
            ordinal,
        );
        match expected.amount {
            Some(amount) => equal(report, format!("{label}.amount"), actual.amount, amount),
            None => report.difference(
                format!("{label}.amount"),
                actual.amount,
                "missing serialized amount",
            ),
        }
        match expected.name_order {
            Some(order) => equal(
                report,
                format!("{label}.name_order"),
                actual.name_order,
                order,
            ),
            None if actual.name_order != 0 => report.difference(
                format!("{label}.name_order"),
                actual.name_order,
                "omitted, so zero",
            ),
            None => {}
        }
        match expected.name_override.as_ref() {
            Some(saved) => {
                name_field(
                    report,
                    &format!("{label}.override"),
                    &actual.custom_name,
                    Some(saved.as_slice()),
                );
            }
            None if !actual.custom_name.is_empty() => report.difference(
                format!("{label}.override"),
                String::from_utf8_lossy(&actual.custom_name).into_owned(),
                "omitted, so empty",
            ),
            None => {}
        }
        match expected.name_override_programmatic {
            Some(saved) => equal(
                report,
                format!("{label}.override_set_programmatically"),
                actual.custom_flag_b != 0,
                saved,
            ),
            None if actual.custom_flag_b != 0 => report.difference(
                format!("{label}.override_set_programmatically"),
                format!("present flag {}", actual.custom_flag_b),
                "absent",
            ),
            None => {}
        }
        equal(
            report,
            format!("{label}.custom_flag_a"),
            actual.custom_flag_a != 0,
            expected.name_ordered.unwrap_or(true),
        );
        let Some(deployment) = expected.deployment.as_ref() else {
            report.difference(
                format!("{label}.military_deployment"),
                "present",
                "missing serialized deployment object",
            );
            continue;
        };
        omitted_not_positive(
            report,
            &label,
            "training",
            actual.training_raw,
            deployment.training.as_ref(),
        );
        omitted_not_positive(
            report,
            &label,
            "max_training",
            actual.max_training_raw,
            deployment.max_training.as_ref(),
        );
        equal(
            report,
            format!("{label}.priority"),
            actual.distributable_priority,
            deployment.priority.unwrap_or(1),
        );
        match (actual.halting_reason_present, deployment.halting_reason) {
            (0, None) => {}
            (0, Some(saved)) => {
                report.difference(format!("{label}.halting_reason"), "absent", saved)
            }
            (_, Some(saved)) => equal(
                report,
                format!("{label}.halting_reason"),
                actual.halting_reason,
                saved,
            ),
            (present, None) => report.difference(
                format!("{label}.halting_reason"),
                format!("present flag {present}"),
                "absent",
            ),
        }
        match deployment.max_manpower {
            Some(value) => equal(
                report,
                format!("{label}.max_manpower"),
                actual.max_manpower,
                value,
            ),
            None if actual.max_manpower > 0 => report.difference(
                format!("{label}.max_manpower"),
                actual.max_manpower,
                "omitted, so not positive",
            ),
            None => {}
        }
        for (side, expected) in [
            (0_u8, deployment.manpower_have.as_ref()),
            (1, deployment.manpower_need.as_ref()),
        ] {
            let actual = manpower.get(&(actual.handle, side));
            let empty = Vec::new();
            let actual = actual.unwrap_or(&empty);
            for (position, (ordinal, _, _)) in actual.iter().enumerate() {
                if *ordinal as usize != position {
                    return Err(invalid("captured deployment manpower is not dense"));
                }
            }
            let Some(expected) = expected else {
                report.difference(
                    format!("{label}.manpower[{side}]"),
                    actual.len(),
                    "missing serialized manpower container",
                );
                continue;
            };
            let actual: Vec<_> = actual.iter().filter(|(_, _, amount)| *amount > 0).collect();
            if actual.len() != expected.len() {
                report.difference(
                    format!("{label}.manpower[{side}].count"),
                    actual.len(),
                    expected.len(),
                );
            }
            for (index, ((_, country, amount), expected)) in actual.iter().zip(expected).enumerate()
            {
                let field = format!("{label}.manpower[{side}][{index}]");
                equal(
                    report,
                    format!("{field}.tag"),
                    required_country_tag(*country, links)?,
                    expected.tag.as_slice(),
                );
                equal(report, format!("{field}.value"), *amount, expected.value);
            }
        }
        let captured = equipment.get(&actual.handle);
        let empty = Vec::new();
        let captured = captured.unwrap_or(&empty);
        for (position, (ordinal, _, _)) in captured.iter().enumerate() {
            if *ordinal as usize != position {
                return Err(invalid("captured deployment equipment is not dense"));
            }
        }
        match deployment.equipment.as_ref() {
            None => {
                if actual.equipment_allocations != 0 {
                    *undecidable_pools += 1;
                }
            }
            Some(pool) => {
                if actual.equipment_allocations == 0 {
                    report.difference(
                        format!("{label}.equipment_allocations"),
                        0,
                        "written wrapper requires a nonzero allocation row",
                    );
                }
                match pool.allow_zero_entries {
                    Some(allow) => equal(
                        report,
                        format!("{label}.allow_zero_entries"),
                        actual.allow_zero_entries != 0,
                        allow,
                    ),
                    None => report.difference(
                        format!("{label}.allow_zero_entries"),
                        actual.allow_zero_entries,
                        "missing serialized allow_zero_entries",
                    ),
                }
                let retained: Vec<_> = captured
                    .iter()
                    .filter(|(_, _, amount)| *amount != 0 || actual.allow_zero_entries != 0)
                    .collect();
                if retained.len() != pool.equipment.len() {
                    report.difference(
                        format!("{label}.equipment.count"),
                        retained.len(),
                        pool.equipment.len(),
                    );
                }
                for (index, ((_, variant, amount), expected)) in
                    retained.iter().zip(&pool.equipment).enumerate()
                {
                    let field = format!("{label}.equipment[{index}]");
                    equal(
                        report,
                        format!("{field}.variant"),
                        *variant,
                        expected.variant,
                    );
                    equal(
                        report,
                        format!("{field}.amount"),
                        *amount,
                        expected.amount.raw,
                    );
                }
            }
        }
    }
    Ok(())
}

fn omitted_not_positive(
    report: &mut Report,
    label: &str,
    field: &str,
    captured: i64,
    saved: Option<&content_reader_save::Decimal>,
) {
    match saved {
        Some(saved) => report.scalar(label, field, Some(captured), Some(saved)),
        None if captured > 0 => report.difference(
            format!("{label}.{field}"),
            captured,
            "omitted, so not positive",
        ),
        None => {}
    }
}

fn compare_lines(
    frame: &DecodedFrame,
    saved: &Save,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let countries = saved
        .production
        .countries
        .as_ref()
        .ok_or_else(|| invalid("save lacks production country roster"))?;
    let mut lines = BTreeMap::new();
    for line in complete::<tables::ProductionLines>(frame)? {
        if line.handle == 0 || lines.insert(line.handle, line).is_some() {
            return Err(invalid("null or duplicate full production line handle"));
        }
    }
    let mut saved_ids = BTreeSet::new();
    let mut line_matches = 0;
    for (tag, country) in report.canonical_countries(countries) {
        let country = country.production.as_ref().ok_or_else(|| {
            invalid(format!(
                "{} lacks serialized production",
                String::from_utf8_lossy(tag)
            ))
        })?;
        for expected in &country.lines {
            if !saved_ids.insert(expected.base.id) {
                return Err(invalid("duplicate full saved production line handle"));
            }
            let label = format!("production line {}", expected.base.id);
            let Some(actual) = lines.remove(&expected.base.id) else {
                report.difference(label, "absent", "present");
                continue;
            };
            line_matches += 1;
            compare_line_base(actual, &expected.base, &label, report);
            equal(
                report,
                format!("{label}.country"),
                required_country_tag(actual.country, links)?,
                tag.as_slice(),
            );
            equal(report, format!("{label}.kind"), actual.kind, expected.kind);
            equal(
                report,
                format!("{label}.variant"),
                actual.variant,
                expected.variant,
            );
            equal(
                report,
                format!("{label}.mio"),
                actual.mio,
                expected.mio.unwrap_or(0),
            );
            equal(
                report,
                format!("{label}.requested_factories"),
                actual.requested_factories,
                expected.requested_factories,
            );
            compare_efficiency(actual, expected, &label, report);
        }
    }
    for id in lines.keys() {
        report.difference(format!("production line {id}"), "present", "absent");
    }
    report.compared("ProductionLines", line_matches);
    report.limitation(
        "the maximum factories per line (the optional equipment-type or archetype value, else a define) is not serialized; the recorded value is used as the clamp when the saved efficiencies are averaged",
        "ProductionLines.max_factories",
    );
    Ok(())
}

fn compare_efficiency(
    actual: &ProductionLineRow,
    expected: &production::Line,
    label: &str,
    report: &mut Report,
) {
    report.scalar_checks += 1;
    let count = expected
        .base
        .active_factories
        .unwrap_or(0)
        .min(actual.max_factories);
    let entries = usize::try_from(count).unwrap_or(0);
    let Some(saved) = expected.factory_efficiencies.get(..entries) else {
        report.difference(
            format!("{label}.factory_efficiencies"),
            format!("{entries} averaged entries"),
            format!("{} saved entries", expected.factory_efficiencies.len()),
        );
        return;
    };
    let sum = saved
        .iter()
        .fold(0_u64, |sum, &entry| sum.wrapping_add(entry as u64));
    equal(
        report,
        format!("{label}.efficiency"),
        actual.efficiency_raw,
        game_model::production::efficiency_average(sum, count),
    );
}

fn compare_stockpiles(
    frame: &DecodedFrame,
    saved: &Save,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let countries = saved
        .production
        .countries
        .as_ref()
        .ok_or_else(|| invalid("save lacks production country roster"))?;
    let mut stockpiles = BTreeMap::<Vec<u8>, Vec<_>>::new();
    for row in complete::<tables::Stockpiles>(frame)? {
        let tag = required_country_tag(row.country, links)?;
        stockpiles.entry(tag.to_vec()).or_default().push(row);
    }
    let mut stockpile_matches = 0;
    for (tag, country) in report.canonical_countries(countries) {
        let country = country.production.as_ref().ok_or_else(|| {
            invalid(format!(
                "{} lacks serialized production",
                String::from_utf8_lossy(tag)
            ))
        })?;
        let expected = country.stockpile.as_ref().ok_or_else(|| {
            invalid(format!(
                "{} lacks serialized stockpile",
                String::from_utf8_lossy(tag)
            ))
        })?;
        let mut rows = stockpiles.remove(tag).unwrap_or_default();
        rows.sort_unstable_by_key(|row| row.ordinal);
        stockpile_matches += compare_pool(
            rows.iter()
                .map(|row| (row.ordinal, row.variant_handle, row.amount_raw)),
            expected,
            &format!("{}.stockpile", String::from_utf8_lossy(tag)),
            report,
        )?;
    }
    for (tag, rows) in stockpiles {
        report.difference(
            format!("{}.stockpile", String::from_utf8_lossy(&tag)),
            rows.len(),
            "absent country",
        );
    }
    report.compared("Stockpiles", stockpile_matches);
    Ok(())
}

fn compare_division_equipment(
    frame: &DecodedFrame,
    saved: &Save,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let divisions = saved
        .production
        .divisions
        .as_ref()
        .ok_or_else(|| invalid("save lacks division equipment roster"))?;
    let mut banks = BTreeMap::<u64, Vec<_>>::new();
    for row in complete::<tables::DivisionEquipment>(frame)? {
        banks.entry(row.division).or_default().push(row);
    }
    let mut matched = 0;
    for (&id, expected) in divisions {
        let label = format!("division {id}.equipment");
        let base = saved
            .divisions
            .get(&id)
            .ok_or_else(|| invalid("equipment parent absent from saved division census"))?;
        let pool = expected
            .equipment
            .as_ref()
            .ok_or_else(|| invalid(format!("{label} wrapper absent from save")))?;
        let mut rows = banks.remove(&id).unwrap_or_default();
        rows.sort_unstable_by_key(|row| row.ordinal);
        for row in &rows {
            equal(
                report,
                format!("{label}[{}].template", row.ordinal),
                row.template,
                base.template.unwrap_or(0),
            );
            equal(
                report,
                format!("{label}[{}].country", row.ordinal),
                required_country_tag(row.country, links)?,
                expected.roster_owner.as_slice(),
            );
        }
        matched += compare_pool(
            rows.iter()
                .map(|row| (row.ordinal, row.variant_handle, row.amount_raw)),
            pool,
            &label,
            report,
        )?;
    }
    for id in banks.keys() {
        report.difference(
            format!("division {id}.equipment"),
            "present",
            "absent division",
        );
    }
    report.compared("DivisionEquipment", matched);
    Ok(())
}

#[derive(Clone, Copy)]
enum TokenWriter {
    Token,
    String,
}

fn compare_token(
    tokens: &BTreeMap<u32, &[u8]>,
    id: u32,
    saved: &[u8],
    label: &str,
    writer: TokenWriter,
    report: &mut Report,
) -> io::Result<()> {
    if id == engine_native_memory::cpdx_lexer::FIXED15_TOKEN && matches!(writer, TokenWriter::Token) {
        return Err(invalid(format!(
            "{label} uses reserved token 0x167; the token writer formats fixed15 rather than its lexical name"
        )));
    }
    if let Some(&actual) = tokens.get(&id) {
        if actual.contains(&0) {
            report.limitation(
                "recorded lexical spelling contains an interior NUL; the concrete text-stream behavior is unproved, so only this literal is unverified",
                format!("{label} (token {id})"),
            );
        } else {
            equal(report, label, actual, saved);
        }
    } else {
        report.limitation(
            "recorded lexical token spelling is unavailable; this field's saved literal is unverified",
            format!("{label} (token {id})"),
        );
    }
    Ok(())
}

fn ordinals(values: impl IntoIterator<Item = u32>, label: &str) -> io::Result<()> {
    for (index, ordinal) in values.into_iter().enumerate() {
        if usize::try_from(ordinal).ok() != Some(index) {
            return Err(invalid(format!(
                "{label} ordinals are not contiguous from zero"
            )));
        }
    }
    Ok(())
}

fn name_field(report: &mut Report, label: &str, actual: &[u8], saved: Option<&[u8]>) {
    equal(
        report,
        format!("{label}.present"),
        !actual.is_empty(),
        saved.is_some(),
    );
    if let Some(saved) = saved {
        let end = content_reader_parser::parser::c_string(actual).len();
        equal(report, label, &actual[..end], saved);
        if end < actual.len() {
            report.limitation("text string serialization stops at the first NUL; the captured suffix is not present in the save", label);
        }
    }
}

fn icon_field(report: &mut Report, label: &str, active: u8, icon: &[u8], saved: Option<&[u8]>) {
    equal(
        report,
        format!("{label}.present"),
        active != 0,
        saved.is_some(),
    );
    if let Some(saved) = saved.filter(|_| active != 0) {
        let end = icon
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(icon.len());
        equal(report, label, &icon[..end], saved);
        if end < icon.len() {
            report.limitation("text string serialization stops at the first NUL; the captured suffix is not present in the save", label);
        }
    }
}

mod variants;
pub use variants::compare_variant_archetypes;
use variants::compare_variants;

mod army_upgrades;
use army_upgrades::compare_army_upgrades;

fn compare_line_base(
    actual: &ProductionLineRow,
    expected: &production::LineBase,
    label: &str,
    report: &mut Report,
) {
    if actual.handle != expected.id {
        report.difference(format!("{label}.id"), actual.handle, expected.id);
    }
    for (name, captured, saved) in [
        (
            "active_factories",
            actual.active_factories,
            expected.active_factories.unwrap_or(0),
        ),
        ("priority", actual.priority, expected.priority),
        ("amount", actual.amount, expected.amount),
    ] {
        if captured != saved {
            report.difference(format!("{label}.{name}"), captured, saved);
        }
    }
    for (name, captured, saved) in [
        ("cost", actual.cost_raw, expected.cost.raw),
        (
            "speed",
            actual.speed_raw,
            expected.speed.as_ref().map_or(0, |value| value.raw),
        ),
    ] {
        report.scalar_checks += 1;
        if captured != saved {
            report.difference(format!("{label}.{name}"), captured, saved);
        }
    }
}

fn compare_pool(
    rows: impl IntoIterator<Item = (u32, u64, i64)>,
    expected: &production::Pool,
    label: &str,
    report: &mut Report,
) -> io::Result<usize> {
    let rows = rows.into_iter().collect::<Vec<_>>();
    for (index, &(ordinal, handle, _)) in rows.iter().enumerate() {
        if usize::try_from(ordinal).ok() != Some(index) {
            return Err(invalid(format!(
                "{label} complete native equipment ordinals are not contiguous from zero"
            )));
        }
        if handle == 0 {
            return Err(invalid(format!("{label} has a null equipment handle")));
        }
    }

    let compare_zeros = expected.allow_zero_entries.ok_or_else(|| {
        invalid(format!(
            "{label} lacks the serialized allow_zero_entries field"
        ))
    })?;
    let actual = rows
        .iter()
        .filter(|&&(_, _, amount)| compare_zeros || amount != 0)
        .collect::<Vec<_>>();
    let saved = expected
        .equipment
        .iter()
        .filter(|entry| compare_zeros || entry.amount.raw != 0)
        .collect::<Vec<_>>();
    if expected.allow_zero_entries == Some(false)
        && expected.equipment.iter().any(|entry| entry.amount.raw == 0)
    {
        return Err(invalid(format!(
            "{label} saves a zero equipment row while allow_zero_entries is false"
        )));
    }

    let mut pending = BTreeMap::<u64, VecDeque<usize>>::new();
    for (index, entry) in saved.iter().enumerate() {
        pending.entry(entry.variant).or_default().push_back(index);
    }
    let mut consumed = vec![false; saved.len()];
    let mut order = Vec::with_capacity(actual.len());
    let mut matched = 0;
    for (position, &&(_, handle, amount)) in actual.iter().enumerate() {
        let field = format!("{label}.visible_equipment[{position}]");
        match pending.get_mut(&handle).and_then(VecDeque::pop_front) {
            Some(index) => {
                matched += 1;
                consumed[index] = true;
                order.push(index);
                report.scalar(&field, "amount", Some(amount), Some(&saved[index].amount));
            }
            None => report.difference(format!("{field}.variant"), handle, "absent"),
        }
    }
    for (index, expected) in saved.iter().enumerate() {
        if !consumed[index] {
            report.difference(
                format!("{label}.visible_equipment.saved[{index}]"),
                "absent",
                expected,
            );
        }
    }
    if order.windows(2).any(|pair| pair[0] > pair[1]) {
        report.difference(
            format!("{label}.visible_equipment.order"),
            "native pool order",
            "serialized order",
        );
    }

    if !compare_zeros && rows.iter().any(|&(_, _, amount)| amount == 0) {
        report.limitation(
            "the pool writer omits zero equipment entries when allow_zero_entries is false; ordered nonzero entries are compared, while omitted zero membership and absolute native source ordinals cannot be certified by this save",
            label,
        );
    }
    Ok(matched)
}

#[cfg(test)]
mod tests;

pub fn compare_category_families(
    recorded: Result<&engine_executable::executable_tables::auxiliary::CategoryFamilies, String>,
    installed: Result<&engine_executable::executable_tables::auxiliary::CategoryFamilies, String>,
    report: &mut Report,
) -> io::Result<()> {
    let recorded = recorded.map_err(|reason| invalid(format!("equipment category families not recorded: {reason}")))?;
    match installed {
        Ok(installed) => {
            equal(report, "equipment category families", recorded, installed);
            report.compared("equipment category families", 1);
        }
        Err(reason) => report.limitation(
            "the installed executable's category families cannot certify the recorded ones",
            reason,
        ),
    }
    Ok(())
}
