
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use engine_land_inputs::{
    ncombat_log_cequipment_loss::month_bucket, ncombat_log_cmanager::RECOVERED_KIND,
};
use engine_resources::{clend_lease_exchange::DELIVERED_KIND, clend_lease_exchange::SUNK_KIND};
use content_reader_save::{Save, production};
use replay_data::{rows::CountryRow, rows::TableId, rows::tables};
use replay_frame::frame::DecodedFrame;

use super::{Report, complete, invalid, required_country_tag};

const GROUP_TYPE: u32 = engine_native::corders_group::REFERENCE_TYPE;
const VARIANT_TYPE: u32 = engine_native::cequipment_variant::REFERENCE_TYPE;

type Key<'a> = (&'a [u8], u32, u8, u16, u32);

type LendKey<'a> = (&'a [u8], u32, u8, u32);
type LendRows<'a> = BTreeMap<LendKey<'a>, (&'a [u8], i64)>;

struct Saved {
    date: i32,
    amount_raw: i64,
}

pub fn compare(
    frame: &DecodedFrame,
    saved: &Save,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let full = report.full_refresh();
    let reads = complete::<tables::SliceReads>(frame)?;
    let mut actual: BTreeMap<Key<'_>, (i32, i64)> = BTreeMap::new();
    let mut read: BTreeSet<&[u8]> = BTreeSet::new();
    for slice in reads.iter().filter(|slice| {
        slice.table == TableId::CombatLogEquipment.index() as u32
            && slice.sample_hour == frame.hour
    }) {
        read.insert(required_country_tag(slice.owner, links)?);
    }
    let mut carried = 0;
    for row in complete::<tables::CombatLogEquipment>(frame)? {
        let tag = required_country_tag(row.country, links)?;
        if !full
            && replay_data::sliced::read_hour(reads, TableId::CombatLogEquipment, row.country)
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
    let mut expected: BTreeMap<Key<'_>, Saved> = BTreeMap::new();
    let mut saved_groups: BTreeSet<(&[u8], u32)> = BTreeSet::new();
    let mut omits_zero: BTreeMap<(&[u8], u32, u8, u16), i32> = BTreeMap::new();
    for country in &saved.combat_log.countries {
        let tag = country.tag.as_slice();
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
            for (months, recovered) in [(&group.losses, false), (&group.recovered, true)] {
                for month in months {
                    let kind = if recovered {
                        RECOVERED_KIND
                    } else {
                        u8::try_from(month.reason)
                            .ok()
                            .filter(|&reason| reason < RECOVERED_KIND)
                            .ok_or_else(|| invalid("saved combat log loss reason outside 0 to 3"))?
                    };
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
                        if expected.insert(key, value).is_some() {
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
    for (key, saved) in &expected {
        let label = label(key);
        let Some((date, amount_raw)) = actual.remove(key) else {
            report.difference(label, "absent", "saved amount present");
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
            "outside a full refresh the comparison covers every saved equipment month for countries with a current read",
            "CombatLogEquipment",
        );
    }
    report.limitation(
        "combat log country tags compare resolved identity and cannot certify equivalent raw country-link aliases",
        "CombatLogEquipment",
    );
    Ok(())
}

pub fn state(frame: &DecodedFrame, report: &mut Report) -> io::Result<()> {
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

pub fn division_census(
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
                emission.identity.actual == content_reader_save::ActualCountry::Reserved
                    && emission.identity.tag.as_slice() == tag
            })
        }) {
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

pub fn lend_lease(
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
    let mut receivers: BTreeMap<(&[u8], u32), &[u8]> = BTreeMap::new();
    for (tag, country) in report.canonical_countries(&saved.economy.countries) {
        for exchange in country.lend_lease.iter().flatten() {
            let id = (exchange.id >> 32) as u32;
            receivers.insert((tag.as_slice(), id), exchange.receiver.as_slice());
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
    for ((giver, exchange, kind, variant), (receiver, amount)) in actual {
        let label = format!(
            "{}.lend_lease[{exchange}][kind {kind}][variant {variant}]",
            String::from_utf8_lossy(giver)
        );
        if amount == 0 && omits_zero.contains(&(giver, exchange, kind)) {
            if let Some(&saved_receiver) = receivers.get(&(giver, exchange))
                && saved_receiver != receiver
            {
                report.difference(
                    format!("{label}.receiver"),
                    String::from_utf8_lossy(receiver),
                    String::from_utf8_lossy(saved_receiver),
                );
            }
            zeros += 1;
            continue;
        }
        report.difference(label, "present", "absent");
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
