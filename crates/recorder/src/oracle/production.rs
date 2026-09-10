//! Production comparisons retain native ordering and serializer coverage limits.

use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use installed_content::save::{Save, production};
use replay::{
    frame::DecodedFrame,
    rows::{CountryRow, ProductionLineRow, tables},
};

use super::{Report, TokenNames, complete, country_tag, invalid, required_country_tag};

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

pub(super) fn compare(
    frame: &DecodedFrame,
    saved: &Save,
    links: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let mut complete = report.attempt("ProductionLines", |report| {
        compare_lines(frame, saved, links, report)
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
    complete &= report.attempt("ArmyUpgrades", |report| {
        compare_army_upgrades(frame, &saved.production, links, report)
    });
    report.limitation(
        "serialized country tags compare resolved country identity and cannot certify equivalent raw country-link aliases",
        "ProductionLines, Stockpiles, DivisionEquipment, VariantDefinitions, ArmyUpgradeGroups, ArmyUpgradeRequests, ArmyUpgradeDeliveries, ArmyUpgradeProduced",
    );
    if complete {
        Ok(())
    } else {
        Err(invalid("one or more production comparisons failed"))
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
    for (tag, country) in countries {
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
        }
    }
    for id in lines.keys() {
        report.difference(format!("production line {id}"), "present", "absent");
    }
    report.compared("ProductionLines", line_matches);
    Ok(())
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
    for (tag, country) in countries {
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
    /// ADEC0/ADCE0 pass the token and c_str to AAC50.
    Token,
    /// F7A6F0 resolves the token, then AD7A0/ADB70 emit its c_str directly.
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
    if id == 0x167 && matches!(writer, TokenWriter::Token) {
        return Err(invalid(format!(
            "{label} uses reserved token 0x167; AAC50 formats fixed15 rather than its lexical name"
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
    // BD4770 checks the native MSVC length; ordinary text output then passes
    // c_str through1424ADF40, whose text path stops at the first NUL.
    equal(
        report,
        format!("{label}.present"),
        !actual.is_empty(),
        saved.is_some(),
    );
    if let Some(saved) = saved {
        let end = actual
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(actual.len());
        equal(report, label, &actual[..end], saved);
        if end < actual.len() {
            report.limitation("text string serialization stops at the first NUL; the captured suffix is not present in the save", label);
        }
    }
}

fn compare_variants(
    frame: &DecodedFrame,
    saved: &production::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let mut complete = report.attempt("VariantDefinitions", |report| {
        compare_variant_definitions(frame, saved, links, tokens, report)
    });
    complete &= report.attempt("VariantModules", |report| {
        compare_variant_modules(frame, saved, tokens, report)
    });
    complete &= report.attempt("VariantUpgrades", |report| {
        compare_variant_upgrades(frame, saved, tokens, report)
    });
    complete &= report.attempt("VariantStats", |report| {
        compare_variant_stats(frame, saved, report)
    });
    report.limitation("BD4770 does not serialize the classifier_flags or mission_mask caches; their captured native operands have no direct save comparison", "VariantDefinitions.classifier_flags, VariantDefinitions.mission_mask");
    report.limitation("native boolean serialization maps every nonzero byte to yes; text flags cannot certify raw byte values beyond truthiness", "VariantDefinitions.append_variant_name, VariantDefinitions.obsolete, VariantModules.definition_active");
    report.limitation("191E6F0 and F7A6F0 serialize definition tokens through the lexical dictionary, without the independent definition MSVC string; token spellings do not certify those separate captured bytes", "VariantModules.definition_name, VariantUpgrades.definition_name");
    report.limitation("1F2A20 uses variant+18 as the saved registry key and BD4770 writes the variant body; neither writes the equipment definition's independent MSVC name bytes", "VariantDefinitions.equipment_name");
    report.limitation("null module definitions currently make the complete native module table unavailable; the oracle never treats a failed module walk as an empty saved module list", "VariantModules");

    if complete {
        Ok(())
    } else {
        Err(invalid("one or more variant comparisons failed"))
    }
}

fn compare_variant_definitions(
    frame: &DecodedFrame,
    saved: &production::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let saved_variants = saved
        .variants
        .as_ref()
        .ok_or_else(|| invalid("save lacks the equipment variant registry"))?;
    let mut definitions = BTreeMap::new();
    for row in complete::<tables::VariantDefinitions>(frame)? {
        if row.id == 0 || definitions.insert(row.id, row).is_some() {
            return Err(invalid("null or duplicate variant definition handle"));
        }
    }
    let mut definition_matches = 0;
    for (&id, expected) in saved_variants {
        let label = format!("variant {id}");
        let Some(actual) = definitions.remove(&id) else {
            report.difference(label, "absent", "present");
            continue;
        };
        definition_matches += 1;
        compare_token(
            tokens,
            actual.equipment_token,
            expected.definition.as_slice(),
            &format!("{label}.equipment_token"),
            TokenWriter::Token,
            report,
        )?;
        name_field(
            report,
            &format!("{label}.name"),
            &actual.display_name,
            expected.display_name.as_deref(),
        );
        name_field(
            report,
            &format!("{label}.position"),
            &actual.variant_name,
            expected.variant_name.as_deref(),
        );
        equal(
            report,
            format!("{label}.show_position"),
            actual.append_variant_name != 0,
            expected.append_variant_name.unwrap_or(true),
        );
        equal(
            report,
            format!("{label}.obsolete"),
            actual.obsolete != 0,
            expected.obsolete.unwrap_or(false),
        );
        for (field, reference, saved_tag) in [
            ("creator", actual.creator_link, &expected.creator),
            ("origin", actual.origin_link, &expected.origin),
        ] {
            let captured = country_tag(reference, links)?;
            if let (Some(captured), Some(saved_tag)) = (captured, saved_tag) {
                equal(
                    report,
                    format!("{label}.{field}"),
                    captured,
                    saved_tag.as_slice(),
                );
            } else if saved_tag.is_none() {
                report.difference(
                    format!("{label}.{field}"),
                    captured,
                    "missing serialized tag",
                );
            } else {
                report.limitation("BA5C20 maps a zero country reference through the unrecorded country-tag array entry0; its literal spelling is unmapped here, not an omitted field or an assumed tag", format!("{label}.{field}"));
            }
        }
    }
    for id in definitions.keys() {
        report.difference(format!("variant {id}"), "present", "absent");
    }
    report.compared("VariantDefinitions", definition_matches);
    Ok(())
}

fn compare_variant_modules(
    frame: &DecodedFrame,
    saved: &production::Pack,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let saved_variants = saved
        .variants
        .as_ref()
        .ok_or_else(|| invalid("save lacks the equipment variant registry"))?;
    let mut modules = BTreeMap::<u64, Vec<_>>::new();
    for row in complete::<tables::VariantModules>(frame)? {
        if row.variant == 0 {
            return Err(invalid("null variant module parent"));
        }
        modules.entry(row.variant).or_default().push(row);
    }
    let mut module_matches = 0;
    for (&id, expected) in saved_variants {
        let label = format!("variant {id}");
        let mut actual_modules = modules.remove(&id).unwrap_or_default();
        actual_modules.sort_unstable_by_key(|row| row.source_ordinal);
        ordinals(
            actual_modules.iter().map(|row| row.source_ordinal),
            &format!("{label}.modules"),
        )?;
        equal(
            report,
            format!("{label}.modules.present"),
            !actual_modules.is_empty(),
            expected.modules.is_some(),
        );
        let visible = actual_modules
            .iter()
            .filter(|row| row.definition_active != 0)
            .collect::<Vec<_>>();
        let expected_modules = expected.modules.as_deref().unwrap_or_default();
        for index in 0..visible.len().max(expected_modules.len()) {
            let field = format!("{label}.modules[{index}]");
            match (visible.get(index), expected_modules.get(index)) {
                (Some(actual), Some(expected)) => {
                    module_matches += 1;
                    compare_token(
                        tokens,
                        actual.slot,
                        expected.slot.as_slice(),
                        &format!("{field}.slot"),
                        TokenWriter::Token,
                        report,
                    )?;
                    compare_token(
                        tokens,
                        actual.definition_token,
                        expected.definition.as_slice(),
                        &format!("{field}.definition_token"),
                        TokenWriter::Token,
                        report,
                    )?;
                }
                (Some(_), None) => report.difference(field, "present", "absent"),
                (None, Some(_)) => report.difference(field, "absent", "present"),
                (None, None) => unreachable!(),
            }
        }
        if actual_modules.len() != visible.len() {
            report.limitation("191E6F0 omits inactive module definitions; active modules are compared in order, while inactive identities and absolute native source ordinals are not serialized", &label);
        }
    }
    for id in modules.keys() {
        report.difference(format!("variant {id}.modules"), "present", "absent variant");
    }
    report.compared("VariantModules", module_matches);
    Ok(())
}

fn compare_variant_upgrades(
    frame: &DecodedFrame,
    saved: &production::Pack,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let saved_variants = saved
        .variants
        .as_ref()
        .ok_or_else(|| invalid("save lacks the equipment variant registry"))?;
    let mut upgrades = BTreeMap::<u64, Vec<_>>::new();
    for row in complete::<tables::VariantUpgrades>(frame)? {
        if row.variant == 0 {
            return Err(invalid("null variant upgrade parent"));
        }
        upgrades.entry(row.variant).or_default().push(row);
    }
    let mut upgrade_matches = 0;
    for (&id, expected) in saved_variants {
        let label = format!("variant {id}");
        let mut actual_upgrades = upgrades.remove(&id).unwrap_or_default();
        actual_upgrades.sort_unstable_by_key(|row| row.source_ordinal);
        ordinals(
            actual_upgrades.iter().map(|row| row.source_ordinal),
            &format!("{label}.upgrades"),
        )?;
        equal(
            report,
            format!("{label}.upgrades.present"),
            !actual_upgrades.is_empty(),
            expected.upgrades.is_some(),
        );
        let expected_upgrades = expected.upgrades.as_deref().unwrap_or_default();
        for index in 0..actual_upgrades.len().max(expected_upgrades.len()) {
            let field = format!("{label}.upgrades[{index}]");
            match (actual_upgrades.get(index), expected_upgrades.get(index)) {
                (Some(actual), Some(expected)) => {
                    upgrade_matches += 1;
                    compare_token(
                        tokens,
                        actual.definition_token as u32,
                        expected.definition.as_slice(),
                        &format!("{field}.definition_token"),
                        TokenWriter::String,
                        report,
                    )?;
                    equal(
                        report,
                        format!("{field}.level"),
                        actual.level,
                        expected.level,
                    );
                }
                (Some(_), None) => report.difference(field, "present", "absent"),
                (None, Some(_)) => report.difference(field, "absent", "present"),
                (None, None) => unreachable!(),
            }
        }
    }
    for id in upgrades.keys() {
        report.difference(
            format!("variant {id}.upgrades"),
            "present",
            "absent variant",
        );
    }
    report.compared("VariantUpgrades", upgrade_matches);
    Ok(())
}

fn compare_variant_stats(
    frame: &DecodedFrame,
    saved: &production::Pack,
    report: &mut Report,
) -> io::Result<()> {
    let saved_variants = saved
        .variants
        .as_ref()
        .ok_or_else(|| invalid("save lacks the equipment variant registry"))?;
    let mut stats = BTreeSet::new();
    let mut census_matches = true;
    for row in complete::<tables::VariantStats>(frame)? {
        if row.variant == 0 || !stats.insert(row.variant) {
            return Err(invalid("null or duplicate variant stats parent"));
        }
        if !saved_variants.contains_key(&row.variant) {
            census_matches = false;
            report.difference(
                format!("variant {}.stats", row.variant),
                "present",
                "absent variant",
            );
        }
    }
    for id in saved_variants.keys() {
        if !stats.contains(id) {
            census_matches = false;
            report.difference(format!("variant {id}.stats"), "absent", "present variant");
        }
    }
    report.compared("VariantStats", 0);
    report.limitation("BD4770 does not serialize the ten native stat caches; saved definitions are not recomputed or substituted for speed, range, air attack, agility, maximum speed, reliability, air defense, thrust, fuel or build cost", format!("VariantStats: {} decoded rows, zero save comparisons", stats.len()));
    if census_matches {
        Ok(())
    } else {
        Err(invalid(
            "variant stats and complete save registry censuses differ",
        ))
    }
}

fn compare_army_upgrades(
    frame: &DecodedFrame,
    saved: &production::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut complete = report.attempt("ArmyUpgradeGroups", |report| {
        compare_army_upgrade_groups(frame, saved, links, report)
    });
    complete &= report.attempt("ArmyUpgradeRequests", |report| {
        compare_army_upgrade_requests(frame, saved, links, report)
    });
    complete &= report.attempt("ArmyUpgradeDeliveries", |report| {
        compare_army_upgrade_deliveries(frame, saved, links, report)
    });
    complete &= report.attempt("ArmyUpgradeProduced", |report| {
        compare_army_upgrade_produced(frame, saved, links, report)
    });
    complete &= report.attempt("ArmyUpgradeLinks", |_| check_army_upgrade_links(frame));
    report.limitation("delivery progress is serialized only for status1; inactive progress is absent in both the captured row contract and save", "ArmyUpgradeDeliveries.progress_raw");
    report.limitation("delivery allow_zero_entries is serialized as a boolean and cannot certify nonzero raw byte values beyond truthiness", "ArmyUpgradeDeliveries.allow_zero_entries");
    report.limitation("FE3F0 omits a null request equipment pointer; the native reader currently makes that request table unavailable, preserving the absent save handle without inventing a variant", "ArmyUpgradeRequests.equipment");
    if complete {
        Ok(())
    } else {
        Err(invalid("one or more army upgrade comparisons failed"))
    }
}

fn compare_army_upgrade_groups(
    frame: &DecodedFrame,
    saved: &production::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let divisions = saved
        .divisions
        .as_ref()
        .ok_or_else(|| invalid("save lacks army upgrade parent census"))?;
    let mut groups = BTreeMap::new();
    for row in complete::<tables::ArmyUpgradeGroups>(frame)? {
        if row.army == 0 || groups.insert(row.army, row).is_some() {
            return Err(invalid("null or duplicate army upgrade group parent"));
        }
    }
    let mut group_matches = 0;
    for (&army, division) in divisions {
        let expected = division.upgrades.as_ref();
        let actual = groups.remove(&army);
        let label = format!("army {army}.upgrades");
        if let Some(actual) = actual {
            group_matches += 1;
            equal(
                report,
                format!("{label}.country"),
                required_country_tag(actual.country, links)?,
                division.roster_owner.as_slice(),
            );
        }
        match (actual, expected) {
            (Some(actual), Some(expected)) => {
                equal(report, format!("{label}.priority"), actual.priority, expected.priority.unwrap_or(1));
            }
            (None, Some(_)) => report.difference(&label, "absent army group", "present nonempty group"),
            (Some(_), None) => report.limitation("FE0B0 omits an upgrade group when both child vectors are empty; an empty captured group's priority is not serialized", &label),
            (None, None) => {},
        }
    }
    for army in groups.keys() {
        report.difference(
            format!("army {army}.upgrades"),
            "present",
            "absent division",
        );
    }
    report.compared("ArmyUpgradeGroups", group_matches);
    Ok(())
}

fn compare_army_upgrade_requests(
    frame: &DecodedFrame,
    saved: &production::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let divisions = saved
        .divisions
        .as_ref()
        .ok_or_else(|| invalid("save lacks army upgrade parent census"))?;
    let mut requests = BTreeMap::<u64, Vec<_>>::new();
    for row in complete::<tables::ArmyUpgradeRequests>(frame)? {
        if row.army == 0 {
            return Err(invalid("null army upgrade request parent"));
        }
        requests.entry(row.army).or_default().push(row);
    }
    let mut request_matches = 0;
    for (&army, division) in divisions {
        let expected = division.upgrades.as_ref();
        let label = format!("army {army}.upgrades");
        let mut actual_requests = requests.remove(&army).unwrap_or_default();
        actual_requests.sort_unstable_by_key(|row| row.ordinal);
        ordinals(
            actual_requests.iter().map(|row| row.ordinal),
            &format!("{label}.requests"),
        )?;
        for row in &actual_requests {
            equal(
                report,
                format!("{label}.request[{}].country", row.ordinal),
                required_country_tag(row.country, links)?,
                division.roster_owner.as_slice(),
            );
        }
        let expected_requests = expected
            .map(|group| group.requests.as_slice())
            .unwrap_or_default();
        for index in 0..actual_requests.len().max(expected_requests.len()) {
            let field = format!("{label}.request[{index}]");
            match (actual_requests.get(index), expected_requests.get(index)) {
                (Some(actual), Some(expected)) => {
                    request_matches += 1;
                    equal(
                        report,
                        format!("{field}.equipment"),
                        Some(actual.equipment),
                        expected.equipment,
                    );
                    report.scalar(
                        &field,
                        "request",
                        Some(actual.request_raw),
                        Some(&expected.request),
                    );
                    report.scalar(
                        &field,
                        "total",
                        Some(actual.total_raw),
                        Some(&expected.total),
                    );
                }
                (Some(_), None) => report.difference(field, "present", "absent"),
                (None, Some(_)) => report.difference(field, "absent", "present"),
                (None, None) => unreachable!(),
            }
        }
    }
    for army in requests.keys() {
        report.difference(
            format!("army {army}.upgrades.requests"),
            "present",
            "absent division",
        );
    }
    report.compared("ArmyUpgradeRequests", request_matches);
    Ok(())
}

fn compare_army_upgrade_deliveries(
    frame: &DecodedFrame,
    saved: &production::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let divisions = saved
        .divisions
        .as_ref()
        .ok_or_else(|| invalid("save lacks army upgrade parent census"))?;
    let mut deliveries = BTreeMap::<u64, Vec<_>>::new();
    for row in complete::<tables::ArmyUpgradeDeliveries>(frame)? {
        if row.army == 0 {
            return Err(invalid("null army upgrade delivery parent"));
        }
        deliveries.entry(row.army).or_default().push(row);
    }
    let mut delivery_matches = 0;
    for (&army, division) in divisions {
        let expected = division.upgrades.as_ref();
        let label = format!("army {army}.upgrades");
        let mut actual_deliveries = deliveries.remove(&army).unwrap_or_default();
        actual_deliveries.sort_unstable_by_key(|row| row.ordinal);
        ordinals(
            actual_deliveries.iter().map(|row| row.ordinal),
            &format!("{label}.deliveries"),
        )?;
        for row in &actual_deliveries {
            equal(
                report,
                format!("{label}.delivery[{}].country", row.ordinal),
                required_country_tag(row.country, links)?,
                division.roster_owner.as_slice(),
            );
        }
        let expected_deliveries = expected
            .map(|group| group.deliveries.as_slice())
            .unwrap_or_default();
        for index in 0..actual_deliveries.len().max(expected_deliveries.len()) {
            let field = format!("{label}.delivery[{index}]");
            let expected = expected_deliveries.get(index);
            match (actual_deliveries.get(index), expected) {
                (Some(actual), Some(expected)) => {
                    delivery_matches += 1;
                    equal(
                        report,
                        format!("{field}.status"),
                        actual.status,
                        expected.status.unwrap_or(0),
                    );
                    equal(
                        report,
                        format!("{field}.progress"),
                        actual.progress_raw,
                        expected.progress.as_ref().map(|value| value.raw),
                    );
                    report.scalar_checks += usize::from(expected.progress.is_some()) + 1;
                    equal(
                        report,
                        format!("{field}.total_progress"),
                        actual.total_progress_raw,
                        expected
                            .total_progress
                            .as_ref()
                            .map_or(100_000, |value| value.raw),
                    );
                    equal(
                        report,
                        format!("{field}.allow_zero_entries"),
                        Some(actual.allow_zero_entries != 0),
                        expected.produced.allow_zero_entries,
                    );
                }
                (Some(_), None) => report.difference(&field, "present", "absent"),
                (None, Some(_)) => report.difference(&field, "absent", "present"),
                (None, None) => unreachable!(),
            }
        }
    }
    for army in deliveries.keys() {
        report.difference(
            format!("army {army}.upgrades.deliveries"),
            "present",
            "absent division",
        );
    }
    report.compared("ArmyUpgradeDeliveries", delivery_matches);
    Ok(())
}

fn compare_army_upgrade_produced(
    frame: &DecodedFrame,
    saved: &production::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let divisions = saved
        .divisions
        .as_ref()
        .ok_or_else(|| invalid("save lacks army upgrade parent census"))?;
    let mut produced = BTreeMap::<(u64, u32), Vec<_>>::new();
    for row in complete::<tables::ArmyUpgradeProduced>(frame)? {
        if row.army == 0 {
            return Err(invalid("null produced equipment army parent"));
        }
        produced
            .entry((row.army, row.delivery_ordinal))
            .or_default()
            .push(row);
    }
    let mut produced_matches = 0;
    for (&army, division) in divisions {
        let Some(expected) = &division.upgrades else {
            continue;
        };
        for (index, delivery) in expected.deliveries.iter().enumerate() {
            let ordinal =
                u32::try_from(index).map_err(|_| invalid("saved delivery index exceeds u32"))?;
            let mut rows = produced.remove(&(army, ordinal)).unwrap_or_default();
            rows.sort_unstable_by_key(|row| row.ordinal);
            for row in &rows {
                equal(
                    report,
                    format!(
                        "army {army}.upgrades.delivery[{index}].produced[{}].country",
                        row.ordinal
                    ),
                    required_country_tag(row.country, links)?,
                    division.roster_owner.as_slice(),
                );
            }
            produced_matches += compare_pool(
                rows.iter()
                    .map(|row| (row.ordinal, row.equipment, row.amount_raw)),
                &delivery.produced,
                &format!("army {army}.upgrades.delivery[{index}].produced"),
                report,
            )?;
        }
    }
    for ((army, delivery), rows) in produced {
        report.difference(
            format!("army {army}.upgrades.delivery[{delivery}].produced"),
            rows.len(),
            "absent delivery",
        );
    }
    report.compared("ArmyUpgradeProduced", produced_matches);
    Ok(())
}

fn check_army_upgrade_links(frame: &DecodedFrame) -> io::Result<()> {
    let mut groups = BTreeMap::new();
    for row in complete::<tables::ArmyUpgradeGroups>(frame)? {
        if row.army == 0 || groups.insert(row.army, row).is_some() {
            return Err(invalid("null or duplicate army upgrade group parent"));
        }
    }
    for row in complete::<tables::ArmyUpgradeRequests>(frame)? {
        let group = groups
            .get(&row.army)
            .ok_or_else(|| invalid("army upgrade request has no captured group"))?;
        if row.country != group.country {
            return Err(invalid(
                "army upgrade request country differs from its group",
            ));
        }
    }
    let mut delivery_keys = BTreeSet::new();
    for row in complete::<tables::ArmyUpgradeDeliveries>(frame)? {
        let group = groups
            .get(&row.army)
            .ok_or_else(|| invalid("army upgrade delivery has no captured group"))?;
        if row.country != group.country || !delivery_keys.insert((row.army, row.ordinal)) {
            return Err(invalid(
                "duplicate army delivery key or inconsistent country",
            ));
        }
    }
    for row in complete::<tables::ArmyUpgradeProduced>(frame)? {
        if !delivery_keys.contains(&(row.army, row.delivery_ordinal)) {
            return Err(invalid(
                "produced equipment has no captured army upgrade delivery",
            ));
        }
        if row.country != groups[&row.army].country {
            return Err(invalid(
                "produced equipment country differs from its army upgrade group",
            ));
        }
    }
    Ok(())
}

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
    // Base1414A36A0 explicitly emits speed and produced only when nonzero.
    // Their absent literal therefore establishes raw zero, unlike an unmapped
    // field or a serializer predicate that has not been inspected.
    for (name, captured, saved) in [
        ("cost", actual.cost_raw, expected.cost.raw),
        (
            "speed",
            actual.speed_raw,
            expected.speed.as_ref().map_or(0, |value| value.raw),
        ),
        (
            "produced",
            actual.produced_raw,
            expected.produced.as_ref().map_or(0, |value| value.raw),
        ),
    ] {
        report.scalar_checks += 1;
        if captured != saved {
            report.difference(format!("{label}.{name}"), captured, saved);
        }
    }
}

/// A complete native pool supplies every row, in source order. The tuple is
/// (native ordinal, full variant handle, signed fixed5 amount), directly mapped
/// from the caller's typed replay row; it is not a second recorded row format.
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

    // FFDB00 always calls the unconditional boolean writer1424AE850.
    // A missing policy is malformed/incomplete save data, not a native default.
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

    let mut matched = 0;
    for index in 0..actual.len().max(saved.len()) {
        let field = format!("{label}.visible_equipment[{index}]");
        match (actual.get(index), saved.get(index)) {
            (Some(&&(_, handle, amount)), Some(expected)) => {
                matched += 1;
                if handle != expected.variant {
                    report.difference(format!("{field}.variant"), handle, expected.variant);
                }
                report.scalar(&field, "amount", Some(amount), Some(&expected.amount));
            }
            (Some(actual), None) => report.difference(field, actual, "absent"),
            (None, Some(expected)) => report.difference(field, "absent", expected),
            (None, None) => unreachable!(),
        }
    }

    if !compare_zeros && rows.iter().any(|&(_, _, amount)| amount == 0) {
        report.limitation(
            "FFDB00 omits zero equipment entries when allow_zero_entries is false; ordered nonzero entries are compared, while omitted zero membership and absolute native source ordinals cannot be certified by this save",
            label,
        );
    }
    Ok(matched)
}

#[cfg(test)]
mod tests {
    use super::super::lexical_tokens;
    use super::*;
    use installed_content::{parser, save::Decimal};
    use replay::{
        frame::TableView,
        rows::{
            ArmyUpgradeDeliveryRow, ArmyUpgradeGroupRow, ArmyUpgradeProducedRow,
            ArmyUpgradeRequestRow, LexicalTokenRow, TableId, TableRows, VariantDefinitionRow,
            VariantModuleRow, VariantStatsRow, VariantUpgradeRow,
        },
    };

    fn pool(text: &[u8]) -> production::Pool {
        production::parse_pool(&parser::parse(text).unwrap().items).unwrap()
    }

    #[test]
    fn zero_omission_does_not_shift_duplicate_nonzero_bank_matches() {
        let saved = pool(
            b"equipment={id={id=1 type=70} amount=-0.00001}
              equipment={id={id=1 type=70} amount=2}
              allow_zero_entries=no",
        );
        let rows = [
            (0, 70, 0),
            (1, 0x1_0000_0046, -1),
            (2, 71, 0),
            (3, 0x1_0000_0046, 200_000),
        ];
        let mut report = Report::default();
        assert_eq!(compare_pool(rows, &saved, "pool", &mut report).unwrap(), 2);
        assert!(report.differences.is_empty());
        assert_eq!(report.scalar_checks, 2);
        assert_eq!(report.limitations.len(), 1);
        let mut changed = rows;
        changed[1].2 = 200_000;
        changed[3].2 = -1;
        let mut report = Report::default();
        compare_pool(changed, &saved, "pool", &mut report).unwrap();
        assert_eq!(report.differences.len(), 2);
    }

    #[test]
    fn zero_preserving_pool_checks_full_handles_and_zero_membership() {
        let saved = pool(
            b"equipment={id={id=4294967295 type=70} amount=0}
              equipment={id={id=0 type=70} amount=-92233720368547.75808}
              allow_zero_entries=yes",
        );
        let mut report = Report::default();
        compare_pool(
            [(0, 0xffff_ffff_0000_0046, 0), (1, 70, i64::MIN)],
            &saved,
            "pool",
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty());
        assert!(report.limitations.is_empty());
        let mut report = Report::default();
        compare_pool([(0, 70, i64::MIN)], &saved, "pool", &mut report).unwrap();
        assert_eq!(report.differences.len(), 3);
    }

    #[test]
    fn absent_policy_and_broken_native_order_are_explicit_errors() {
        let mut saved = production::Pool {
            equipment: vec![production::Equipment {
                variant: 70,
                amount: Decimal::parse(b"1").unwrap(),
            }],
            allow_zero_entries: None,
        };
        let mut report = Report::default();
        assert!(compare_pool([(0, 70, 100_000)], &saved, "pool", &mut report).is_err());
        saved.allow_zero_entries = Some(false);
        for rows in [[(1, 70, 100_000)], [(0, 0, 100_000)]] {
            assert!(compare_pool(rows, &saved, "pool", &mut report).is_err());
        }
    }

    #[test]
    fn base_line_compares_omitted_zero_and_signed_fixed5_without_narrowing() {
        let doc = parser::parse(
            b"id={id=4294967295 type=56} cost=92233720368547.75807 priority=-1 amount=-2147483648",
        )
        .unwrap();
        let expected = production::parse_line_base(&doc.items).unwrap();
        let mut actual = ProductionLineRow {
            handle: 0xffff_ffff_0000_0038,
            cost_raw: i64::MAX,
            priority: -1,
            amount: i32::MIN,
            ..Default::default()
        };
        let mut report = Report::default();
        compare_line_base(&actual, &expected, "line", &mut report);
        assert!(report.differences.is_empty());
        assert_eq!(report.scalar_checks, 3);
        actual.speed_raw = -1;
        actual.produced_raw = 1;
        actual.cost_raw -= 1;
        actual.active_factories = -1;
        compare_line_base(&actual, &expected, "line", &mut report);
        assert_eq!(report.differences.len(), 4);
    }

    #[test]
    fn saved_string_presence_uses_native_length_and_visible_prefix() {
        let mut report = Report::default();
        name_field(&mut report, "empty", b"", None);
        name_field(&mut report, "bytes", &[0xff, b'A'], Some(&[0xff, b'A']));
        name_field(&mut report, "nul", b"A\0suffix", Some(b"A"));
        assert!(report.differences.is_empty());
        assert_eq!(report.limitations.len(), 1);
        name_field(&mut report, "missing", b"A", None);
        name_field(&mut report, "unexpected", b"", Some(b""));
        name_field(&mut report, "changed", b"A\0suffix", Some(b"B"));
        assert_eq!(report.differences.len(), 3);
    }

    #[test]
    fn lexical_writers_keep_exact_bytes_and_scope_unproved_nul_literals() {
        let token_bytes = [b'a', 0xff, 0, b'z'];
        let exact_bytes = [b'a', 0xff];
        let tokens = BTreeMap::from([
            (0, b"".as_slice()),
            (17, exact_bytes.as_slice()),
            (18, token_bytes.as_slice()),
            (0x167, b"0.75".as_slice()),
        ]);
        let mut report = Report::default();
        for writer in [TokenWriter::Token, TokenWriter::String] {
            compare_token(&tokens, 17, &[b'a', 0xff], "exact", writer, &mut report).unwrap();
            compare_token(&tokens, 18, b"unproved output", "nul", writer, &mut report).unwrap();
        }
        compare_token(&tokens, 0, b"", "zero", TokenWriter::Token, &mut report).unwrap();
        assert!(report.differences.is_empty());
        assert_eq!(report.limitations.len(), 1);
        assert!(
            compare_token(
                &tokens,
                0x167,
                b"0.75",
                "numeric",
                TokenWriter::Token,
                &mut report
            )
            .is_err()
        );
        compare_token(
            &tokens,
            0x167,
            b"0.75",
            "upgrade",
            TokenWriter::String,
            &mut report,
        )
        .unwrap();
        compare_token(
            &tokens,
            u32::MAX,
            b"",
            "missing",
            TokenWriter::String,
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty());
        assert!(
            report
                .limitations
                .values()
                .flatten()
                .any(|subject| subject.contains("missing (token 4294967295)"))
        );
        compare_token(
            &tokens,
            17,
            &[b'a', 0xfe],
            "changed",
            TokenWriter::Token,
            &mut report,
        )
        .unwrap();
        assert_eq!(report.differences.len(), 1);
    }

    #[test]
    fn typed_variants_keep_independent_checks_when_lexical_tokens_are_unavailable() {
        let source = br#"equipments={plane={id={id=4294967295 type=70}
            name="A" position="II" creator=GER origin=GER
            modules={slot=module} upgrades={upgrades={{engine 255}}}}}
        "#;
        let saved = production::parse(&parser::parse(source).unwrap().items).unwrap();
        let variant = 0xffff_ffff_0000_0046;
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 0,
            metrics: Default::default(),
            tables: std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index]))
            }),
        };
        frame.tables[TableId::VariantDefinitions.index()] =
            TableView::Complete(TableRows::VariantDefinitions(vec![VariantDefinitionRow {
                id: variant,
                equipment_token: 17,
                creator_link: 11,
                origin_link: 11,
                append_variant_name: 0xff,
                display_name: b"A".to_vec(),
                variant_name: b"II".to_vec(),
                equipment_name: b"unsaved independent bytes".to_vec(),
                classifier_flags: u64::MAX,
                mission_mask: u32::MAX,
                ..Default::default()
            }]));
        frame.tables[TableId::VariantModules.index()] =
            TableView::Complete(TableRows::VariantModules(vec![
                VariantModuleRow {
                    variant,
                    source_ordinal: 0,
                    slot: u32::MAX,
                    definition_token: u32::MAX,
                    definition_active: 0,
                    definition_name: b"omitted inactive module".to_vec(),
                },
                VariantModuleRow {
                    variant,
                    source_ordinal: 1,
                    slot: 18,
                    definition_token: 19,
                    definition_active: 0xff,
                    definition_name: b"unsaved independent bytes".to_vec(),
                },
            ]));
        frame.tables[TableId::VariantUpgrades.index()] =
            TableView::Complete(TableRows::VariantUpgrades(vec![VariantUpgradeRow {
                variant,
                source_ordinal: 0,
                definition_token: 20,
                level: 255,
                definition_name: b"unsaved independent bytes".to_vec(),
            }]));
        frame.tables[TableId::VariantStats.index()] =
            TableView::Complete(TableRows::VariantStats(vec![VariantStatsRow {
                variant,
                values: [i64::MIN; 10],
            }]));
        frame.tables[TableId::LexicalTokens.index()] = TableView::Unavailable {
            reason: "lexical writer closure unproved".into(),
        };
        let country = CountryRow {
            id: 11,
            tag: b"GER".to_vec(),
            ..Default::default()
        };
        let links = BTreeMap::from([(11, &country)]);
        let mut report = Report::default();
        let tokens = lexical_tokens(&frame, &mut report).unwrap();
        compare_variants(&frame, &saved, &links, &tokens, &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["VariantDefinitions"], 1);
        assert_eq!(report.compared_tables["VariantModules"], 1);
        assert_eq!(report.compared_tables["VariantUpgrades"], 1);
        assert_eq!(report.compared_tables["VariantStats"], 0);
        let unverified = report
            .limitations
            .iter()
            .find(|(reason, _)| reason.contains("this field's saved literal is unverified"))
            .unwrap()
            .1;
        assert_eq!(unverified.len(), 4);

        frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![
                LexicalTokenRow {
                    token: 17,
                    literal: b"plane".to_vec(),
                },
                LexicalTokenRow {
                    token: 18,
                    literal: b"slot".to_vec(),
                },
                LexicalTokenRow {
                    token: 19,
                    literal: b"module".to_vec(),
                },
                LexicalTokenRow {
                    token: 20,
                    literal: b"engine".to_vec(),
                },
            ]));
        let mut report = Report::default();
        let tokens = lexical_tokens(&frame, &mut report).unwrap();
        compare_variants(&frame, &saved, &links, &tokens, &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert!(
            !report
                .limitations
                .keys()
                .any(|reason| reason.contains("saved literal is unverified"))
        );
        let TableView::Complete(TableRows::LexicalTokens(rows)) =
            &mut frame.tables[TableId::LexicalTokens.index()]
        else {
            unreachable!()
        };
        for row in rows {
            row.literal.extend_from_slice(&[0, 0xff]);
        }
        let mut report = Report::default();
        let tokens = lexical_tokens(&frame, &mut report).unwrap();
        compare_variants(&frame, &saved, &links, &tokens, &mut report).unwrap();
        assert!(report.differences.is_empty());
        let unverified = report
            .limitations
            .iter()
            .find(|(reason, _)| {
                reason.contains("recorded lexical spelling contains an interior NUL")
            })
            .unwrap()
            .1;
        assert_eq!(unverified.len(), 4);
        let TableView::Complete(TableRows::LexicalTokens(rows)) =
            &mut frame.tables[TableId::LexicalTokens.index()]
        else {
            unreachable!()
        };
        for row in rows {
            if row.token == 17 {
                continue;
            }
            row.literal
                .truncate(row.literal.iter().position(|&byte| byte == 0).unwrap());
            row.literal[0] = b'x';
        }
        let mut report = Report::default();
        let tokens = lexical_tokens(&frame, &mut report).unwrap();
        compare_variants(&frame, &saved, &links, &tokens, &mut report).unwrap();
        assert_eq!(report.differences.len(), 3);
        let unverified = report
            .limitations
            .iter()
            .find(|(reason, _)| {
                reason.contains("recorded lexical spelling contains an interior NUL")
            })
            .unwrap()
            .1;
        assert_eq!(unverified.len(), 1);
        let TableView::Complete(TableRows::LexicalTokens(rows)) =
            &mut frame.tables[TableId::LexicalTokens.index()]
        else {
            unreachable!()
        };
        let row = rows.iter_mut().find(|row| row.token == 17).unwrap();
        row.literal
            .truncate(row.literal.iter().position(|&byte| byte == 0).unwrap());
        row.literal[0] = b'x';
        let mut report = Report::default();
        let tokens = lexical_tokens(&frame, &mut report).unwrap();
        compare_variants(&frame, &saved, &links, &tokens, &mut report).unwrap();
        assert_eq!(report.differences.len(), 4);
        for field in [
            ".equipment_token",
            ".modules[0].slot",
            ".modules[0].definition_token",
            ".upgrades[0].definition_token",
        ] {
            assert!(
                report
                    .differences
                    .iter()
                    .any(|difference| difference.contains(field)),
                "{field}"
            );
        }
        frame.tables[TableId::LexicalTokens.index()] = TableView::Unavailable {
            reason: "lexical writer closure unproved".into(),
        };

        let mut wider_save = saved.clone();
        let mut extra = wider_save.variants.as_ref().unwrap()[&variant].clone();
        extra.id = 70;
        extra.modules = None;
        extra.upgrades = None;
        wider_save
            .variants
            .as_mut()
            .unwrap()
            .insert(extra.id, extra);
        let mut report = Report::default();
        assert!(
            compare_variants(&frame, &wider_save, &links, &BTreeMap::new(), &mut report).is_err()
        );
        assert_eq!(report.differences.len(), 2);
        assert!(
            report
                .differences
                .iter()
                .all(|field| field.contains("variant 70"))
        );

        let TableView::Complete(TableRows::VariantUpgrades(rows)) =
            &mut frame.tables[TableId::VariantUpgrades.index()]
        else {
            unreachable!()
        };
        rows[0].level = 254;
        let mut report = Report::default();
        let tokens = lexical_tokens(&frame, &mut report).unwrap();
        compare_variants(&frame, &saved, &links, &tokens, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
        assert!(report.differences[0].contains(".level"));

        frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
                token: 17,
                literal: b"different_equipment".to_vec(),
            }]));
        let mut report = Report::default();
        let tokens = lexical_tokens(&frame, &mut report).unwrap();
        compare_variants(&frame, &saved, &links, &tokens, &mut report).unwrap();
        assert_eq!(report.differences.len(), 2);
        assert!(
            report
                .differences
                .iter()
                .any(|field| field.contains(".equipment_token"))
        );
        frame.tables[TableId::VariantStats.index()] =
            TableView::Complete(TableRows::VariantStats(Vec::new()));
        frame.tables[TableId::VariantDefinitions.index()] = TableView::Unavailable {
            reason: "variant identity unreadable".into(),
        };
        let mut report = Report::default();
        let tokens = lexical_tokens(&frame, &mut report).unwrap();
        assert!(compare_variants(&frame, &saved, &links, &tokens, &mut report).is_err());
        assert!(report.failed_comparisons.contains_key("VariantDefinitions"));
        assert!(report.failed_comparisons.contains_key("VariantStats"));
        assert_eq!(report.compared_tables["VariantModules"], 1);
        assert_eq!(report.compared_tables["VariantUpgrades"], 1);
        assert!(
            report
                .differences
                .iter()
                .any(|field| field.contains(".level"))
        );
    }

    #[test]
    fn typed_army_upgrades_compare_priority_progress_and_ordered_produced_amounts() {
        let source = br"countries={GER={units={division={id={id=4294967295 type=51}
            requests={upgrades={priority=-1
                request={request=-0.00001 total=92233720368547.75807
                    equipment_variant_index={id=0 type=70}}
                delivery={status=1 progress=-92233720368547.75808 produced={
                    equipment={id={id=0 type=70} amount=-0.00001}
                    allow_zero_entries=no}}
            }}
        }}}}";
        let saved = production::parse(&parser::parse(source).unwrap().items).unwrap();
        let army = 0xffff_ffff_0000_0033;
        let country = CountryRow {
            id: 11,
            tag: b"GER".to_vec(),
            ..Default::default()
        };
        let other_country = CountryRow {
            id: 12,
            tag: b"FRA".to_vec(),
            ..Default::default()
        };
        let links = BTreeMap::from([(11, &country), (12, &other_country)]);
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 0,
            metrics: Default::default(),
            tables: std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index]))
            }),
        };
        frame.tables[TableId::ArmyUpgradeGroups.index()] =
            TableView::Complete(TableRows::ArmyUpgradeGroups(vec![ArmyUpgradeGroupRow {
                country: 11,
                army,
                priority: u32::MAX,
            }]));
        frame.tables[TableId::ArmyUpgradeRequests.index()] =
            TableView::Complete(TableRows::ArmyUpgradeRequests(vec![
                ArmyUpgradeRequestRow {
                    country: 11,
                    army,
                    ordinal: 0,
                    equipment: 70,
                    request_raw: -1,
                    total_raw: i64::MAX,
                },
            ]));
        frame.tables[TableId::ArmyUpgradeDeliveries.index()] =
            TableView::Complete(TableRows::ArmyUpgradeDeliveries(vec![
                ArmyUpgradeDeliveryRow {
                    country: 11,
                    army,
                    ordinal: 0,
                    status: 1,
                    progress_raw: Some(i64::MIN),
                    total_progress_raw: 100_000,
                    allow_zero_entries: 0,
                },
            ]));
        frame.tables[TableId::ArmyUpgradeProduced.index()] =
            TableView::Complete(TableRows::ArmyUpgradeProduced(vec![
                ArmyUpgradeProducedRow {
                    country: 11,
                    army,
                    delivery_ordinal: 0,
                    ordinal: 0,
                    equipment: 70,
                    amount_raw: 0,
                },
                ArmyUpgradeProducedRow {
                    country: 11,
                    army,
                    delivery_ordinal: 0,
                    ordinal: 1,
                    equipment: 70,
                    amount_raw: -1,
                },
            ]));
        let mut report = Report::default();
        compare_army_upgrades(&frame, &saved, &links, &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        for table in [
            "ArmyUpgradeGroups",
            "ArmyUpgradeRequests",
            "ArmyUpgradeDeliveries",
            "ArmyUpgradeProduced",
        ] {
            assert_eq!(report.compared_tables[table], 1);
        }
        assert_eq!(report.scalar_checks, 5);
        let TableView::Complete(TableRows::ArmyUpgradeGroups(rows)) =
            &mut frame.tables[TableId::ArmyUpgradeGroups.index()]
        else {
            unreachable!()
        };
        rows[0].priority = 1;
        let TableView::Complete(TableRows::ArmyUpgradeDeliveries(rows)) =
            &mut frame.tables[TableId::ArmyUpgradeDeliveries.index()]
        else {
            unreachable!()
        };
        rows[0].progress_raw = Some(i64::MIN + 1);
        let TableView::Complete(TableRows::ArmyUpgradeRequests(rows)) =
            &mut frame.tables[TableId::ArmyUpgradeRequests.index()]
        else {
            unreachable!()
        };
        rows[0].total_raw -= 1;
        let TableView::Complete(TableRows::ArmyUpgradeProduced(rows)) =
            &mut frame.tables[TableId::ArmyUpgradeProduced.index()]
        else {
            unreachable!()
        };
        rows[1].amount_raw -= 1;
        let mut report = Report::default();
        compare_army_upgrades(&frame, &saved, &links, &mut report).unwrap();
        assert_eq!(report.differences.len(), 4);
        frame.tables[TableId::ArmyUpgradeRequests.index()] = TableView::Unavailable {
            reason: "request graph unreadable".into(),
        };
        let TableView::Complete(TableRows::ArmyUpgradeGroups(rows)) =
            &mut frame.tables[TableId::ArmyUpgradeGroups.index()]
        else {
            unreachable!()
        };
        rows[0].country = 12;
        let mut report = Report::default();
        assert!(compare_army_upgrades(&frame, &saved, &links, &mut report).is_err());
        assert_eq!(report.differences.len(), 4);
        assert!(
            report
                .differences
                .iter()
                .any(|field| field.contains(".upgrades.country"))
        );
        assert!(
            report
                .failed_comparisons
                .contains_key("ArmyUpgradeRequests")
        );
        assert!(report.failed_comparisons.contains_key("ArmyUpgradeLinks"));
        assert_eq!(report.compared_tables["ArmyUpgradeGroups"], 1);
        assert_eq!(report.compared_tables["ArmyUpgradeDeliveries"], 1);
        assert_eq!(report.compared_tables["ArmyUpgradeProduced"], 1);
    }
}
