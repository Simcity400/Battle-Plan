//! CombatLogEquipment against the save's `combat_log` blocks, which
//! the game state save writer writes from the same combat-log managers
//! the reader walks: each month entry's pool, reason and date, per country
//! and orders group, keyed by (country tag, group id, kind, month, variant).
//!
//! The table is sliced by country, so only the countries read at the compared
//! hour (SliceReads) are compared. A full hour reads every retained month; at
//! any other hour a country is read through each array's last two entries
//! (or whole, when its last complete read is older), so those saved entries
//! are required and older saved entries are compared where recorded. A
//! recorded group log the save no longer holds is accepted as a deleted
//! log only if its group is absent from every saved theatre's definitions.
//! Missing theatre data cannot confirm deletion; a still-existing group or
//! unread theatre census is a difference. Deleted logs' final rows (kept
//! by the prune detour) are listed, not compared. The pool writer omits zero amounts
//! unless `allow_zero_entries`, so a recorded zero with no saved amount is not
//! a difference.
//!
//! LendLeaseEquipment against each giver's saved `lend_lease` exchanges
//! (`total_delivered`, `equipment_sunk`), keyed by (giver tag, exchange id,
//! kind, variant), with the receiver.

use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use engine::{
    clend_lease_exchange::{DELIVERED_KIND, SUNK_KIND},
    ncombat_log_cequipment_loss::month_bucket,
    ncombat_log_cmanager::{RECOVERED_KIND, WINDOW_ENTRIES},
};
use installed_content::save::{Save, production};
use replay::{
    frame::DecodedFrame,
    rows::{CountryRow, TableId, tables},
};

use super::{Report, complete, invalid, required_country_tag};

/// The orders group and variant reference types (the reader checks the same).
const GROUP_TYPE: u32 = engine::corders_group::REFERENCE_TYPE;
const VARIANT_TYPE: u32 = engine::cequipment_variant::REFERENCE_TYPE;

type Key<'a> = (&'a [u8], u32, u8, u16, u32);

/// (giver tag, exchange id, kind, variant id).
type LendKey<'a> = (&'a [u8], u32, u8, u32);
/// Recorded receiver tag and amount of a lend-lease row.
type LendRows<'a> = BTreeMap<LendKey<'a>, (&'a [u8], i64)>;

/// A saved amount with its entry's date.
struct Saved {
    date: i32,
    amount_raw: i64,
}

pub(super) fn compare(
    frame: &DecodedFrame,
    saved: &Save,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let full = report.full_refresh();
    let reads = complete::<tables::SliceReads>(frame)?;
    let mut actual: BTreeMap<Key<'_>, (i32, i64)> = BTreeMap::new();
    let mut read: BTreeSet<&[u8]> = BTreeSet::new();
    let mut carried = 0;
    for row in complete::<tables::CombatLogEquipment>(frame)? {
        let tag = required_country_tag(row.country, links)?;
        if !full
            && replay::sliced::read_hour(reads, TableId::CombatLogEquipment, row.country)
                != Some(frame.hour)
        {
            carried += 1;
            continue;
        }
        read.insert(tag);
        let key = (tag, row.group, row.kind, row.month, row.variant);
        if actual.insert(key, (row.date, row.amount_raw)).is_some() {
            return Err(invalid("duplicate resolved combat log row"));
        }
    }
    // Every saved entry of a compared country, and whether the read must hold
    // it: at a full hour every entry; otherwise each array's last two, which
    // any read holds (a country read whole also holds the older ones, which
    // are then compared too).
    let mut expected: BTreeMap<Key<'_>, (Saved, bool)> = BTreeMap::new();
    let mut saved_groups: BTreeSet<(&[u8], u32)> = BTreeSet::new();
    // Entries whose pool may omit a zero amount, keyed without the variant.
    let mut omits_zero: BTreeMap<(&[u8], u32, u8, u16), i32> = BTreeMap::new();
    for country in &saved.combat_log.countries {
        let tag = country.tag.as_slice();
        // At a sliced hour only the countries read now describe the save.
        let compared = full || read.contains(tag);
        for group in &country.groups {
            if group.group as u32 != GROUP_TYPE {
                return Err(invalid("saved combat log group is not an orders group"));
            }
            let id = (group.group >> 32) as u32;
            saved_groups.insert((tag, id));
            if !compared {
                continue;
            }
            let mut arrays: [Vec<&installed_content::save::combat_log::Month>; 5] = Default::default();
            for month in &group.losses {
                let kind = u8::try_from(month.reason)
                    .ok()
                    .filter(|&reason| reason < RECOVERED_KIND)
                    .ok_or_else(|| invalid("saved combat log loss reason outside 0 to 3"))?;
                arrays[kind as usize].push(month);
            }
            arrays[RECOVERED_KIND as usize].extend(&group.recovered);
            for (kind, months) in arrays.iter().enumerate() {
                let kind = kind as u8;
                let window = if full {
                    0
                } else {
                    months.len().saturating_sub(WINDOW_ENTRIES as usize)
                };
                for (position, month) in months.iter().enumerate() {
                    let required = position >= window;
                    let bucket = u16::try_from(month_bucket(month.date)).map_err(|_| {
                        invalid("saved combat log month outside the recorded range")
                    })?;
                    if month.pool.allow_zero_entries != Some(true) {
                        omits_zero.insert((tag, id, kind, bucket), month.date);
                    }
                    for entry in &month.pool.equipment {
                        if entry.variant as u32 != VARIANT_TYPE {
                            return Err(invalid("saved combat log pool names a non-variant"));
                        }
                        let key = (tag, id, kind, bucket, (entry.variant >> 32) as u32);
                        let value = Saved {
                            date: month.date,
                            amount_raw: entry.amount.raw,
                        };
                        if expected.insert(key, (value, required)).is_some() {
                            return Err(invalid(
                                "saved combat log repeats a variant within one group, kind and month",
                            ));
                        }
                    }
                }
            }
        }
    }
    let mut count = 0;
    for (key, (saved, required)) in &expected {
        let label = label(key);
        let Some((date, amount_raw)) = actual.remove(key) else {
            if *required {
                report.difference(label, "absent", "saved amount present");
            }
            continue;
        };
        if date != saved.date {
            report.difference(format!("{label}.date"), date, saved.date);
        }
        if amount_raw != saved.amount_raw {
            report.difference(format!("{label}.amount_raw"), amount_raw, saved.amount_raw);
        }
        count += 1;
    }
    let mut zeros = 0;
    let mut deleted = BTreeSet::new();
    // References are global, including a group now assigned to another country.
    let existing_groups: BTreeSet<_> = saved
        .naval_history
        .theatre_countries
        .iter()
        .flat_map(|countries| countries.values())
        .flat_map(|country| &country.land)
        .flat_map(|theatre| &theatre.definitions)
        .map(|group| group.id)
        .collect();
    for (key, (date, amount_raw)) in actual {
        let (tag, group, kind, month, _) = key;
        if !saved_groups.contains(&(tag, group)) {
            if saved.naval_history.theatre_countries.is_none() {
                report.difference(
                    label(&key),
                    "present",
                    "combat log absent; missing theatre definitions cannot confirm deletion",
                );
            } else if existing_groups.contains(&(u64::from(group) << 32 | u64::from(GROUP_TYPE))) {
                report.difference(
                    label(&key),
                    "present",
                    "combat log absent for existing orders group",
                );
            } else {
                deleted.insert((tag, group));
            }
            continue;
        }
        if amount_raw == 0 && omits_zero.get(&(tag, group, kind, month)) == Some(&date) {
            zeros += 1;
            continue;
        }
        report.difference(label(&key), "present", "absent");
    }
    report.compared("CombatLogEquipment", count);
    if zeros != 0 {
        report.limitation(
            "the pool serializer omits zero amounts unless allow_zero_entries is set; a recorded zero amount of a saved month entry is accepted as omitted",
            format!("CombatLogEquipment: {zeros} zero rows"),
        );
    }
    if !deleted.is_empty() {
        report.limitation(
            "a recorded group log absent from both combat_log and all saved orders-group definitions is treated as final rows the daily prune deleted; this assumes every live orders group is listed under a land theatre's definitions, whose completeness is unproven",
            format!("CombatLogEquipment: {} deleted group logs", deleted.len()),
        );
    }
    if carried != 0 {
        report.limitation(
            "sliced by country: rows of countries not read at the compared hour are carried and not compared",
            format!("CombatLogEquipment: {carried} carried rows"),
        );
    }
    if !full {
        report.limitation(
            "outside a full hour a country last read completely this or last month is read through each array's last two month entries, and a country without rows has no read record, so older saved months are compared only where recorded and countries never read are not compared",
            "CombatLogEquipment",
        );
    }
    report.limitation(
        "the Combat Log's enemy losses, manpower, template statistics and combat references, and month entries with no amounts, are not recorded",
        "CombatLogEquipment",
    );
    report.limitation(
        "combat log country tags compare resolved identity and cannot certify equivalent raw country-link aliases",
        "CombatLogEquipment",
    );
    Ok(())
}

/// The save has no proven field for the local playing country or the prune
/// observation.
pub(super) fn state(frame: &DecodedFrame, report: &mut Report) -> io::Result<()> {
    let rows = complete::<tables::CombatLogState>(frame)?;
    if rows.len() != 1 || rows[0].prune_observed > 1 {
        return Err(invalid("CombatLogState is not one row with a 0/1 flag"));
    }
    report.compared("CombatLogState", 0);
    report.limitation(
        "the local playing country has no proven save field, and prune observation is recorder state; the header's `player` is not shown to be written from it",
        "CombatLogState",
    );
    Ok(())
}

/// Division totals are saved by the army save writer. That writer does
/// not persist the parent reference: the formation save writer writes
/// member.unit references instead (save::naval_history::OrdersGroup.members).
/// The theatre save writer emits its army/army-group rosters. Regular
/// member rosters supply the inferred split. Only a proved unsaved slot-zero regular
/// group's reciprocal members qualify the inferred split; totals stay exact.
pub(super) fn division_census(
    frame: &DecodedFrame,
    saved: &Save,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut actual: BTreeMap<&[u8], (u32, u32)> = BTreeMap::new();
    report.record_country_emissions(saved);
    for row in complete::<tables::DivisionCensus>(frame)? {
        let tag = required_country_tag(row.country, links)?;
        let counts = actual.entry(tag).or_default();
        counts.0 = counts
            .0
            .checked_add(row.grouped)
            .ok_or_else(|| invalid("division grouped count overflow"))?;
        counts.1 = counts
            .1
            .checked_add(row.ungrouped)
            .ok_or_else(|| invalid("division ungrouped count overflow"))?;
    }
    let mut omitted = BTreeSet::new();
    report.attempt("DivisionCensus omitted-group qualification", |_| {
        omitted = super::naval_history::omitted_regular_group_units(
            frame,
            &saved.naval_history,
            saved.country_emissions.as_deref(),
        )?;
        Ok(())
    });
    let expected = saved_division_counts(saved, &omitted)?;
    let mut compared = 0;
    for (tag, (grouped, ungrouped, qualified)) in expected {
        if saved.country_emissions.as_ref().is_some_and(|emissions| {
            emissions.iter().any(|emission| {
                emission.identity.actual == installed_content::save::ActualCountry::Reserved
                    && emission.identity.tag.as_slice() == tag
            })
        }) {
            // This object has no canonical DivisionCensus row. Its saved
            // operands stay in the full save and receive a named refusal.
            report.failed_comparisons.insert(
                format!(
                    "reserved country {} DivisionCensus",
                    String::from_utf8_lossy(tag)
                ),
                "admitted reserved country division totals have no canonical capture".into(),
            );
            continue;
        }
        let label = format!("{}.division_census", String::from_utf8_lossy(tag));
        let Some((recorded_grouped, recorded_ungrouped)) = actual.remove(tag) else {
            report.difference(label, "absent country row", "saved country present");
            continue;
        };
        let total = u64::from(recorded_grouped) + u64::from(recorded_ungrouped);
        if total != u64::from(grouped) + u64::from(ungrouped) {
            report.difference(
                format!("{label}.total"),
                total,
                u64::from(grouped) + u64::from(ungrouped),
            );
        }
        // These exact members are grouped in the reciprocal current tables,
        // but their omitted definition would otherwise infer saved ungrouped.
        // Compare every remaining member instead of waiving the country's split.
        match recorded_grouped.checked_sub(qualified) {
            Some(remaining) if remaining == grouped => {}
            Some(remaining) => report.difference(format!("{label}.grouped"), remaining, grouped),
            None => report.difference(
                format!("{label}.grouped"),
                recorded_grouped,
                format!("at least {qualified} certified omitted-group members"),
            ),
        }
        let inferred_ungrouped = ungrouped - qualified;
        if recorded_ungrouped != inferred_ungrouped {
            report.difference(
                format!("{label}.ungrouped"),
                recorded_ungrouped,
                inferred_ungrouped,
            );
        }
        if qualified != 0 {
            report.limitation("certified omitted slot-zero regular groups have no serialized parent membership; only their reciprocally joined saved divisions are excluded from the inferred grouping split, while totals and all remaining memberships stay exact", format!("{label}: {qualified} certified omitted-group members"));
        }
        compared += 1;
    }
    for (tag, _) in actual {
        report.difference(
            format!("{}.division_census", String::from_utf8_lossy(tag)),
            "recorded country present",
            "absent saved country",
        );
    }
    report.compared("DivisionCensus", compared);
    report.limitation(
        "the army save writer omits the parent reference; the formation save writer saves member.unit instead. Only certified unsaved slot-zero regular-group members qualify that inference; any other missing membership remains a difference",
        "DivisionCensus: total, grouped and ungrouped counts compared through saved army member vectors; only the named certified omitted members qualify the inferred split",
    );
    Ok(())
}

/// Grouped, inferred ungrouped, and the exact qualified subset of ungrouped.
type SavedDivisionCounts<'a> = BTreeMap<&'a [u8], (u32, u32, u32)>;

fn saved_division_counts<'a>(
    saved: &'a Save,
    omitted: &BTreeSet<u64>,
) -> io::Result<SavedDivisionCounts<'a>> {
    let definitions = saved
        .naval_history
        .theatre_countries
        .iter()
        .flat_map(|countries| countries.values())
        .flat_map(|country| &country.land)
        .flat_map(|theatre| &theatre.definitions)
        // The army group save writer delegates to the game's member
        // writer too; parent membership must not duplicate a child army's.
        .filter(|group| !group.is_army_group);
    let mut members = BTreeSet::new();
    for group in definitions {
        for &member in &group.members {
            if saved.divisions.contains_key(&member) && !members.insert(member) {
                return Err(invalid("saved division belongs to multiple orders groups"));
            }
        }
    }
    let mut expected: SavedDivisionCounts<'_> = saved
        .countries
        .keys()
        .map(|tag| (tag.as_slice(), (0, 0, 0)))
        .collect();
    for division in saved.divisions.values() {
        let owner = division
            .expeditionary_owner
            .as_deref()
            .unwrap_or(&division.roster_owner);
        let counts = expected
            .get_mut(owner)
            .ok_or_else(|| invalid("division census names an absent saved country"))?;
        if members.contains(&division.id) {
            counts.0 += 1;
        } else {
            counts.1 += 1;
            if omitted.contains(&division.id) {
                counts.2 += 1;
            }
        }
    }
    Ok(expected)
}

/// LendLeaseEquipment against each giver's saved exchanges.
pub(super) fn lend_lease(
    frame: &DecodedFrame,
    saved: &Save,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut actual: LendRows<'_> = BTreeMap::new();
    for row in complete::<tables::LendLeaseEquipment>(frame)? {
        let giver = required_country_tag(row.giver, links)?;
        let receiver = required_country_tag(row.receiver, links)?;
        let key = (giver, row.exchange, row.kind, row.variant);
        if actual.insert(key, (receiver, row.amount_raw)).is_some() {
            return Err(invalid("duplicate resolved lend-lease row"));
        }
    }
    let mut count = 0;
    let mut zeros = 0;
    let mut omits_zero: BTreeSet<(&[u8], u32, u8)> = BTreeSet::new();
    for (tag, country) in report.canonical_countries(&saved.economy.countries) {
        for exchange in country.lend_lease.iter().flatten() {
            let id = (exchange.id >> 32) as u32;
            for (kind, pool) in [
                (DELIVERED_KIND, &exchange.delivered),
                (SUNK_KIND, &exchange.sunk),
            ] {
                if pool.allow_zero_entries != Some(true) {
                    omits_zero.insert((tag.as_slice(), id, kind));
                }
                count += compare_pool(
                    report,
                    &mut actual,
                    (tag.as_slice(), id, kind),
                    &exchange.receiver,
                    pool,
                )?;
            }
        }
    }
    for ((giver, exchange, kind, variant), (_, amount)) in actual {
        if amount == 0 && omits_zero.contains(&(giver, exchange, kind)) {
            zeros += 1;
            continue;
        }
        report.difference(
            format!(
                "{}.lend_lease[{exchange}][kind {kind}][variant {variant}]",
                String::from_utf8_lossy(giver)
            ),
            "present",
            "absent",
        );
    }
    report.compared("LendLeaseEquipment", count);
    if zeros != 0 {
        report.limitation(
            "the pool serializer omits zero amounts unless allow_zero_entries is set",
            format!("LendLeaseEquipment: {zeros} zero rows"),
        );
    }
    Ok(())
}

fn compare_pool<'a>(
    report: &mut Report,
    actual: &mut LendRows<'a>,
    (giver, exchange, kind): (&'a [u8], u32, u8),
    receiver: &[u8],
    pool: &production::Pool,
) -> io::Result<usize> {
    let mut count = 0;
    for entry in &pool.equipment {
        if entry.variant as u32 != VARIANT_TYPE {
            return Err(invalid("saved lend-lease pool names a non-variant"));
        }
        let variant = (entry.variant >> 32) as u32;
        let label = format!(
            "{}.lend_lease[{exchange}][kind {kind}][variant {variant}]",
            String::from_utf8_lossy(giver)
        );
        let Some((recorded_receiver, amount)) = actual.remove(&(giver, exchange, kind, variant))
        else {
            report.difference(label, "absent", "saved amount present");
            continue;
        };
        if recorded_receiver != receiver {
            report.difference(
                format!("{label}.receiver"),
                String::from_utf8_lossy(recorded_receiver),
                String::from_utf8_lossy(receiver),
            );
        }
        if amount != entry.amount.raw {
            report.difference(format!("{label}.amount_raw"), amount, entry.amount.raw);
        }
        count += 1;
    }
    Ok(count)
}

fn label(key: &Key<'_>) -> String {
    let (tag, group, kind, month, variant) = key;
    format!(
        "{}.combat_log[group {group}][kind {kind}][month {month}][variant {variant}]",
        String::from_utf8_lossy(tag)
    )
}

#[cfg(test)]
mod tests;
