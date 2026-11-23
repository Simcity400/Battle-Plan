use std::{collections::BTreeMap, io};

use content_reader_save::{
    Save,
    production::{UnitBonus, UnitBonusGrant, UnitBonusTarget},
};
use recorder_oracle_support::support::{eq, serialized_text};
use replay_data::rows::{
    CountryUnitBonusRow, TableId, UnitBonusGrant as RecordedGrant, UnitBonusStat,
    UnitBonusTarget as RecordedTarget, tables,
};
use replay_frame::frame::DecodedFrame;

use super::{
    CountryContext, Report, TokenNames, complete, invalid, required_country_tag, tag_text,
};

const SOURCE_TYPES: [&[u8]; 7] = [
    b"none",
    b"project",
    b"effect",
    b"grand_doctrine",
    b"sub_doctrine",
    b"rewards",
    b"milestones",
];
const NO_SOURCE: &[u8] = b"none";

pub fn compare(
    frame: &DecodedFrame,
    save: &Save,
    countries: &CountryContext<'_>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let rows = complete::<tables::CountryUnitBonuses>(frame)?;
    if !report.refresh_is_current(TableId::CountryUnitBonuses, frame.hour) {
        report.limitation(
            replay_data::cadence::STAGGER_LIMITATION,
            "CountryUnitBonuses",
        );
        return Ok(());
    }
    let mut recorded = BTreeMap::<&[u8], Vec<&CountryUnitBonusRow>>::new();
    for row in rows {
        let list = recorded
            .entry(required_country_tag(row.country, &countries.links)?)
            .or_default();
        if row.ordinal as usize != list.len() {
            return Err(invalid("unit bonus ordinals are not contiguous from zero"));
        }
        list.push(row);
    }
    for (&tag, actual) in &recorded {
        match save.production.unit_bonuses.get(tag) {
            Some(Ok(saved)) => compare_list(tag, actual, saved, tokens, report),
            Some(Err(why)) => report.difference(
                format!("country {}.unit_modifiers", tag_text(Some(tag))),
                format!("{} recorded entries", actual.len()),
                format!("unreadable in save: {why}"),
            ),
            None => report.difference(
                format!("country {}.unit_modifiers", tag_text(Some(tag))),
                format!("{} recorded entries", actual.len()),
                "country absent in save",
            ),
        }
    }
    for (tag, saved) in report.canonical_countries(&save.production.unit_bonuses) {
        if recorded.contains_key(tag.as_slice()) {
            continue;
        }
        match saved {
            Ok(saved) if !saved.is_empty() => compare_list(tag, &[], saved, tokens, report),
            Ok(_) => {}
            Err(why) => report.difference(
                format!("country {}.unit_modifiers", tag_text(Some(tag))),
                "no recorded entries",
                format!("unreadable in save: {why}"),
            ),
        }
    }
    report.compared("CountryUnitBonuses", rows.len());
    report.limitation(
        "each target's requirements, terrain and special adjusters, priorities and sprite are saved but not recorded; the save does not mark a target as exact or category, so targets compare by position and name",
        "CountryUnitBonuses",
    );
    Ok(())
}

fn compare_list(
    tag: &[u8],
    actual: &[&CountryUnitBonusRow],
    saved: &[UnitBonus],
    tokens: &TokenNames<'_>,
    report: &mut Report,
) {
    let label = format!("country {}.unit_modifiers", tag_text(Some(tag)));
    eq(
        report,
        &format!("{label}.entries"),
        actual.len(),
        saved.len(),
    );
    for (index, (row, saved)) in actual.iter().zip(saved).enumerate() {
        compare_entry(&format!("{label}[{index}]"), row, saved, tokens, report);
    }
}

fn name<'a>(
    tokens: &TokenNames<'a>,
    token: u32,
    label: &str,
    report: &mut Report,
) -> Option<&'a [u8]> {
    let name = tokens.get(&token).copied();
    if name.is_none() {
        report.difference(
            label,
            token,
            "unresolved in the recorded lexical dictionary",
        );
    }
    name
}

fn compare_entry(
    label: &str,
    row: &CountryUnitBonusRow,
    saved: &UnitBonus,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) {
    eq(
        report,
        &format!("{label}.type"),
        SOURCE_TYPES.get(usize::from(row.source_kind)).copied(),
        saved.kind.as_deref(),
    );
    eq(
        report,
        &format!("{label}.number"),
        Some(row.source_number),
        saved.number,
    );
    let id = format!("{label}.id");
    if let Some(name) = name(tokens, row.source_id, &id, report) {
        eq(
            report,
            &id,
            (name != NO_SOURCE).then_some(name),
            saved.id.as_deref(),
        );
    }
    let key = format!("{label}.localization_key");
    let text = serialized_text(report, &key, &row.label);
    eq(
        report,
        &key,
        (!row.label.is_empty()).then_some(text),
        saved.label.as_deref(),
    );
    eq(
        report,
        &format!("{label}.targets"),
        row.targets.len(),
        saved.targets.len(),
    );
    for (index, (target, saved)) in row.targets.iter().zip(&saved.targets).enumerate() {
        compare_target(
            &format!("{label}.target[{index}]"),
            target,
            saved,
            tokens,
            report,
        );
    }
}

fn named_stats(stats: &[UnitBonusStat], label: &str, report: &mut Report) -> Vec<(Vec<u8>, i64)> {
    let metadata = content_reader_script::equipment_stats::stat_metadata();
    stats
        .iter()
        .filter(|stat| stat.value_raw != 0)
        .filter_map(|stat| {
            let name = metadata.get(stat.selector as usize);
            if name.is_none() {
                report.difference(label, stat.selector, "outside the executable stat metadata");
            }
            name.map(|(name, _, _)| (name.as_bytes().to_vec(), stat.value_raw))
        })
        .collect()
}

fn saved_stats(stats: &[(Vec<u8>, content_reader_save::Decimal)]) -> Vec<(Vec<u8>, i64)> {
    stats
        .iter()
        .map(|(name, value)| (name.clone(), value.raw))
        .collect()
}

fn compare_target(
    label: &str,
    target: &RecordedTarget,
    saved: &UnitBonusTarget,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) {
    if let Some(name) = name(tokens, target.token, label, report) {
        eq(report, label, name, saved.token.as_slice());
    }
    eq(
        report,
        &format!("{label}.combat_width"),
        (target.combat_width_raw != 0).then_some(target.combat_width_raw),
        saved.combat_width.as_ref().map(|width| width.raw),
    );
    let stats = format!("{label}.stats");
    let mut actual = named_stats(&target.stats, &stats, report);
    let mut expected = saved_stats(&saved.stats);
    actual.sort();
    expected.sort();
    eq(report, &stats, actual, expected);
    eq(
        report,
        &format!("{label}.battalion_mult"),
        target.grants.len(),
        saved.grants.len(),
    );
    for (index, (grant, saved)) in target.grants.iter().zip(&saved.grants).enumerate() {
        compare_grant(
            &format!("{label}.battalion_mult[{index}]"),
            grant,
            saved,
            tokens,
            report,
        );
    }
}

fn compare_grant(
    label: &str,
    grant: &RecordedGrant,
    saved: &UnitBonusGrant,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) {
    let category = format!("{label}.category");
    if let Some(name) = name(tokens, grant.category_token, &category, report) {
        eq(report, &category, Some(name), saved.category.as_deref());
    }
    eq(
        report,
        &format!("{label}.add"),
        Some(grant.add != 0),
        saved.add,
    );
    eq(
        report,
        &format!("{label}.display_as_percentage"),
        Some(grant.display_as_percentage != 0),
        saved.display_as_percentage,
    );
    let stats = format!("{label}.stats");
    let actual = named_stats(&grant.stats, &stats, report);
    eq(report, &stats, actual, saved_stats(&saved.stats));
}

#[cfg(test)]
mod tests;
