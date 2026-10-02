//! CombatLogEquipment against the save's `combat_log` blocks, which
//! CGameState::Serialize@1401f2580 writes from the same NCombatLog managers
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
//! by the prune detour) are listed, not compared. The pool serializer (FFDB00)
//! omits zero amounts
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
        "the local playing country (CGameState+0x520) has no proven save field, and prune observation is recorder state; the header's `player` is not shown to be written from it",
        "CombatLogState",
    );
    Ok(())
}

/// Division totals are saved by CArmy::Serialize@140C80E00. That writer does
/// not persist the +0xc0 parent: COrdersGroup::Serialize@140BE8100 writes
/// member.unit references instead (save::naval_history::OrdersGroup.members).
/// CTheatre::Serialize@140EED430 emits its army/army-group rosters. Grouping
/// assumes every live army is in those theatres; completeness is unproven.
pub(super) fn division_census(
    frame: &DecodedFrame,
    saved: &Save,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut actual: BTreeMap<&[u8], (u32, u32)> = BTreeMap::new();
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
    let expected = saved_division_counts(saved)?;
    let mut compared = 0;
    for (tag, (grouped, ungrouped)) in expected {
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
        if recorded_grouped != grouped {
            report.difference(format!("{label}.grouped"), recorded_grouped, grouped);
        }
        if recorded_ungrouped != ungrouped {
            report.difference(format!("{label}.ungrouped"), recorded_ungrouped, ungrouped);
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
        "CArmy::Serialize@140C80E00 omits +0xc0; COrdersGroup::Serialize@140BE8100 saves member.unit instead. CTheatre::Serialize@140EED430 emits its army/army-group rosters. Grouped counts assume every live army appears in saved land theatres; completeness is unproven, and any missing membership remains a difference",
        "DivisionCensus: total, grouped and ungrouped counts compared through saved army member vectors; absent definitions mean empty membership",
    );
    Ok(())
}

fn saved_division_counts(saved: &Save) -> io::Result<BTreeMap<&[u8], (u32, u32)>> {
    let definitions = saved
        .naval_history
        .theatre_countries
        .iter()
        .flat_map(|countries| countries.values())
        .flat_map(|country| &country.land)
        .flat_map(|theatre| &theatre.definitions)
        // CArmyGroup::Serialize@140BE8070 delegates to 140BE8100's member
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
    let mut expected: BTreeMap<&[u8], (u32, u32)> = saved
        .countries
        .keys()
        .map(|tag| (tag.as_slice(), (0, 0)))
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
    for (tag, country) in &saved.economy.countries {
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
mod tests {
    use super::*;
    use replay::{
        frame::TableView,
        rows::{CombatLogEquipmentRow, LendLeaseEquipmentRow, SliceReadRow, TableId, TableRows},
    };

    const GER: u32 = 4;
    const SOV: u32 = 9;
    /// 1943.10.30.11 in the save's text form.
    const DATE: i32 = 60_827_938;
    const SEPTEMBER: i32 = DATE - 24 * 30;
    const AUGUST: i32 = DATE - 24 * 61;

    #[test]
    fn census_compares_all_saved_divisions_groups_and_expeditionary_owners() {
        use replay::rows::DivisionCensusRow;
        let units = "units={ division={ id={id=1 type=51} logical_country=GER strength=1 organisation=1 } division={ id={id=2 type=51} logical_country=GER expeditionary_owner=SOV strength=1 organisation=1 } }";
        let saved = save_with(
            &[],
            &format!(
                "{units} theatres={{ theatre={{ id={{id=1 type=56}} orders_group={{ id={{id=1 type=53}} name=\"Army\" field_marshal_group=no motorization_level=0 member={{unit={{id=1 type=51}}}} }} }} }}"
            ),
        );
        let mut recorded = frame(vec![], &[]);
        let rows = vec![
            DivisionCensusRow {
                country: GER,
                grouped: 0,
                ungrouped: 0,
            },
            DivisionCensusRow {
                country: SOV,
                grouped: 0,
                ungrouped: 1,
            },
            DivisionCensusRow {
                country: 14,
                grouped: 1,
                ungrouped: 0,
            },
        ];
        *recorded.tables[TableId::DivisionCensus.index()] =
            TableView::Complete(TableRows::DivisionCensus(rows.clone()));
        let countries = links();
        let links = BTreeMap::from([
            (GER, &countries[0]),
            (SOV, &countries[1]),
            (14, &countries[0]),
        ]);
        let mut report = Report::default();
        division_census(&recorded, &saved, &links, &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["DivisionCensus"], 2);
        assert!(
            report
                .limitations
                .values()
                .flatten()
                .any(|text| text.contains("grouped and ungrouped counts compared"))
        );
        let mut wrong = rows.clone();
        wrong[1].grouped = 1;
        wrong[1].ungrouped = 0;
        *recorded.tables[TableId::DivisionCensus.index()] =
            TableView::Complete(TableRows::DivisionCensus(wrong));
        let mut report = Report::default();
        division_census(&recorded, &saved, &links, &mut report).unwrap();
        assert_eq!(report.differences.len(), 2);
        // No definitions mean no grouped divisions, even when totals match.
        let saved = save_with(&[], units);
        assert!(saved.naval_history.theatre_countries.is_some());
        let mut report = Report::default();
        division_census(&recorded, &saved, &links, &mut report).unwrap();
        // Both countries' grouped and ungrouped fields differ, never waived.
        assert_eq!(report.differences.len(), 4, "{:?}", report.differences);
        for tag in ["GER.", "SOV."] {
            assert_eq!(
                report
                    .differences
                    .iter()
                    .filter(|text| text.starts_with(tag))
                    .count(),
                2
            );
        }
        *recorded.tables[TableId::DivisionCensus.index()] =
            TableView::Complete(TableRows::DivisionCensus(rows));
        let mut report = Report::default();
        division_census(&recorded, &saved, &links, &mut report).unwrap();
        assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
        assert!(
            report
                .differences
                .iter()
                .all(|text| text.starts_with("GER."))
        );
        assert!(
            report
                .limitations
                .values()
                .flatten()
                .any(|text| text.contains("absent definitions mean empty membership"))
        );
        assert!(report.limitations.keys().any(|text| {
            text.contains("CArmy::Serialize@140C80E00 omits +0xc0")
                && text.contains("COrdersGroup::Serialize@140BE8100")
        }));
        let TableView::Complete(TableRows::DivisionCensus(rows)) =
            &mut *recorded.tables[TableId::DivisionCensus.index()]
        else {
            unreachable!()
        };
        rows[2].grouped = 0;
        rows[2].ungrouped = 1;
        rows[1].ungrouped += 1;
        let mut report = Report::default();
        division_census(&recorded, &saved, &links, &mut report).unwrap();
        assert_eq!(report.differences.len(), 2);
        assert!(
            report
                .differences
                .iter()
                .any(|text| text.contains(".total"))
        );
        *recorded.tables[TableId::DivisionCensus.index()] = TableView::Unavailable {
            reason: "unread".into(),
        };
        assert!(division_census(&recorded, &saved, &links, &mut Report::default()).is_err());
    }

    #[test]
    fn an_empty_saved_group_still_supplies_membership_data() {
        use replay::rows::DivisionCensusRow;
        let saved = save_with(
            &[],
            "units={ division={ id={id=1 type=51} logical_country=GER strength=1 organisation=1 } } theatres={ theatre={ id={id=1 type=56} orders_group={ id={id=1 type=53} name=\"Empty army\" field_marshal_group=no motorization_level=0 } } }",
        );
        let mut recorded = frame(vec![], &[]);
        *recorded.tables[TableId::DivisionCensus.index()] =
            TableView::Complete(TableRows::DivisionCensus(vec![
                DivisionCensusRow {
                    country: GER,
                    grouped: 1,
                    ungrouped: 0,
                },
                DivisionCensusRow {
                    country: SOV,
                    grouped: 0,
                    ungrouped: 0,
                },
            ]));
        let countries = links();
        let links = BTreeMap::from([(GER, &countries[0]), (SOV, &countries[1])]);
        let mut report = Report::default();
        division_census(&recorded, &saved, &links, &mut report).unwrap();
        assert_eq!(report.differences.len(), 2);
        assert!(
            !report
                .limitations
                .values()
                .flatten()
                .any(|text| text.contains("only total divisions compared"))
        );
    }

    #[test]
    fn army_group_members_do_not_duplicate_child_army_members() {
        let text = |army_group| {
            format!(
                "units={{ division={{ id={{id=1 type=51}} logical_country=GER strength=1 organisation=1 }} }} theatres={{ theatre={{ id={{id=1 type=56}} orders_group={{ id={{id=1 type=53}} name=\"Army\" field_marshal_group=no motorization_level=0 member={{unit={{id=1 type=51}}}} }} orders_group={{ id={{id=2 type=53}} name=\"Parent\" field_marshal_group={army_group} motorization_level=0 member={{unit={{id=1 type=51}}}} }} }} }}"
            )
        };
        let saved = save_with(&[], &text("yes"));
        let counts = saved_division_counts(&saved).unwrap();
        assert_eq!(counts[b"GER".as_slice()], (1, 0));
        assert_eq!(counts[b"SOV".as_slice()], (0, 0));
        let saved = save_with(&[], &text("no"));
        assert_eq!(
            saved_division_counts(&saved).unwrap_err().to_string(),
            "saved division belongs to multiple orders groups"
        );
    }

    /// Read-only check of an external text save with the oracle's parser and
    /// census attribution. Set BATTLE_PLAN_CENSUS_SAVE to its path before running.
    #[test]
    #[ignore]
    fn division_census_from_text_save() {
        let path = std::env::var_os("BATTLE_PLAN_CENSUS_SAVE").or_else(|| std::env::var_os("HOI4_CENSUS_SAVE")).expect("set BATTLE_PLAN_CENSUS_SAVE");
        let bytes = std::fs::read(&path).unwrap();
        let saved = installed_content::save::parse(&bytes).unwrap();
        let mut memberships: BTreeMap<u64, BTreeSet<u64>> = BTreeMap::new();
        let mut army_memberships: BTreeMap<u64, BTreeSet<u64>> = BTreeMap::new();
        let mut groups = [0; 2];
        for group in saved
            .naval_history
            .theatre_countries
            .iter()
            .flat_map(|countries| countries.values())
            .flat_map(|country| &country.land)
            .flat_map(|theatre| &theatre.definitions)
        {
            groups[usize::from(group.is_army_group)] += 1;
            for &member in &group.members {
                if saved.divisions.contains_key(&member) {
                    memberships.entry(member).or_default().insert(group.id);
                    if !group.is_army_group {
                        army_memberships.entry(member).or_default().insert(group.id);
                    }
                }
            }
        }
        println!(
            "save={path:?} countries={} divisions={} armies={} army_groups={}",
            saved.countries.len(),
            saved.divisions.len(),
            groups[0],
            groups[1],
        );
        for (label, memberships) in [
            ("all groups", &memberships),
            ("armies only", &army_memberships),
        ] {
            let duplicates: Vec<_> = memberships
                .iter()
                .filter(|(_, groups)| groups.len() > 1)
                .collect();
            println!(
                "{label}: duplicate divisions={} {duplicates:?}",
                duplicates.len()
            );
        }
        for (tag, (grouped, ungrouped)) in saved_division_counts(&saved).unwrap() {
            println!(
                "{}: grouped={grouped} ungrouped={ungrouped} total={}",
                String::from_utf8_lossy(tag),
                u64::from(grouped) + u64::from(ungrouped),
            );
        }
    }

    fn frame(rows: Vec<CombatLogEquipmentRow>, read: &[u32]) -> DecodedFrame {
        let reads: Vec<SliceReadRow> = read
            .iter()
            .map(|&owner| SliceReadRow {
                table: TableId::CombatLogEquipment.index() as u32,
                owner,
                sample_hour: DATE,
            })
            .collect();
        let tables: [TableView; TableId::ALL.len()] = std::array::from_fn(|index| {
            let table = TableId::ALL[index];
            match table {
                TableId::CombatLogEquipment => {
                    TableView::Complete(TableRows::CombatLogEquipment(rows.clone()))
                }
                TableId::SliceReads => TableView::Complete(TableRows::SliceReads(reads.clone())),
                _ => TableView::Complete(TableRows::empty(table)),
            }
        });
        DecodedFrame {
            branch: 1,
            hour: DATE,
            metrics: Default::default(),
            tables: tables.map(Into::into),
        }
    }

    fn row_at(date: i32, kind: u8, variant: u32, amount_raw: i64) -> CombatLogEquipmentRow {
        CombatLogEquipmentRow {
            country: GER,
            group: 12793,
            kind,
            month: month_bucket(date) as u16,
            variant,
            date,
            amount_raw,
        }
    }

    fn row(kind: u8, variant: u32, amount_raw: i64) -> CombatLogEquipmentRow {
        row_at(DATE, kind, variant, amount_raw)
    }

    /// One saved month entry: its key, reason, date and (variant, amount) pairs.
    fn month_at(key: &str, reason: i32, date: &str, pool: &[(u32, &str)]) -> String {
        let entries: String = pool
            .iter()
            .map(|(variant, amount)| {
                format!("equipment={{ id={{ id={variant} type=70 }} amount={amount} }} ")
            })
            .collect();
        format!(
            "{key}={{ equipment={{ {entries}allow_zero_entries=no }} reason={reason} date=\"{date}\" }} "
        )
    }

    fn month(key: &str, reason: i32, pool: &[(u32, &str)]) -> String {
        month_at(key, reason, "1943.10.30.11", pool)
    }

    fn save_with(months: &[String], extra: &str) -> Save {
        let text = format!(
            "HOI4txt\ndate=\"1943.10.30.11\" game_unique_id=\"campaign\" version=\"build\" game_unique_seed=-42 debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 theater_group_index=6 military_deployment_line_index=7 military_deployment_conveyor_index=8 equipment_variant_index=9 country_leader_index=10 navy_id=11 land_combat_id=12 multiplayer_random_count=-13\ncountries={{ GER={{ {extra} }} SOV={{ }} }}\nstates={{}}\nprovinces={{}}\ncombat_log={{ tag=\"GER\" log={{ group={{ id=12793 type=53 }} {} }} }}\ncombat_log={{ tag=\"SOV\" log={{ group={{ id=500 type=53 }} {} }} }}\n",
            months.concat(),
            month("equipment", 0, &[(9, "1")]),
        );
        installed_content::save::parse(text.as_bytes()).unwrap()
    }

    fn save(months: &[String]) -> Save {
        save_with(months, "")
    }

    fn theatre_groups(save: &mut Save, tag: &[u8], groups: &[u32]) {
        use installed_content::save::naval_history::{LandTheatre, OrdersGroup, TheatreCountry};
        save.naval_history.theatre_countries = Some(BTreeMap::from([(
            tag.to_vec(),
            TheatreCountry {
                land: vec![LandTheatre {
                    id: 1 << 32 | 56,
                    volunteers_country: None,
                    groups: vec![],
                    units: vec![],
                    area_representatives: vec![],
                    definitions: groups
                        .iter()
                        .map(|&group| OrdersGroup {
                            id: u64::from(group) << 32 | u64::from(GROUP_TYPE),
                            ..OrdersGroup::default()
                        })
                        .collect(),
                }],
                air: vec![],
            },
        )]));
    }

    fn links() -> [CountryRow; 2] {
        [
            CountryRow {
                tag: b"GER".to_vec(),
                ..CountryRow::default()
            },
            CountryRow {
                tag: b"SOV".to_vec(),
                ..CountryRow::default()
            },
        ]
    }

    fn run(frame: &DecodedFrame, save: &Save, full: bool) -> io::Result<Report> {
        let countries = links();
        let links = BTreeMap::from([(GER, &countries[0]), (SOV, &countries[1])]);
        let mut report = Report {
            forced_full: full,
            ..Report::default()
        };
        compare(frame, save, &links, &mut report)?;
        Ok(report)
    }

    #[test]
    fn matching_logs_have_no_differences_and_zeros_may_be_omitted() {
        let frame = frame(
            vec![
                row(0, 1326, 300_000),
                row(0, 1327, 0),
                row(2, 1326, 50_000),
                row(RECOVERED_KIND, 1846, 100_000),
                CombatLogEquipmentRow {
                    country: SOV,
                    group: 500,
                    ..row(0, 9, 100_000)
                },
            ],
            &[],
        );
        let save = save(&[
            month("equipment", 0, &[(1326, "3")]),
            month("equipment", 2, &[(1326, "0.5")]),
            month("enemy_equipment", 0, &[(5300, "2")]),
            month("equipment_recovered", 0, &[(1846, "1")]),
        ]);
        let report = run(&frame, &save, true).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["CombatLogEquipment"], 4);
    }

    #[test]
    fn a_missing_log_of_a_still_existing_group_is_a_difference() {
        let frame = frame(
            vec![CombatLogEquipmentRow {
                group: 999,
                ..row(0, 1326, 100_000)
            }],
            &[GER],
        );
        let mut save = save(&[]);
        // Reference ids are global: a transfer to another country is not deletion.
        theatre_groups(&mut save, b"SOV", &[999]);
        let report = run(&frame, &save, false).unwrap();
        assert!(
            report
                .differences
                .iter()
                .any(|difference| difference.contains("group 999")),
            "{:?}",
            report.differences
        );
    }

    #[test]
    fn every_mismatch_is_a_difference() {
        let frame = frame(
            vec![
                row(0, 1326, 300_001),
                row(1, 1326, 100_000),
                row(RECOVERED_KIND, 1846, 5),
            ],
            &[],
        );
        let save = save(&[
            month("equipment", 0, &[(1326, "3"), (1327, "1")]),
            month("equipment", 2, &[(1326, "1")]),
        ]);
        let report = run(&frame, &save, true).unwrap();
        let differences = report.differences.join("\n");
        // GER's four plus SOV's saved month, never recorded.
        assert_eq!(report.differences.len(), 6, "{differences}");
        assert!(differences.contains("[kind 0]") && differences.contains("amount_raw"));
        assert!(differences.contains("[variant 1327]"));
        assert!(differences.contains("[kind 1]") && differences.contains("[kind 2]"));
        assert!(differences.contains("[kind 4]"));
        assert!(differences.contains("SOV.combat_log[group 500]"));
    }

    #[test]
    fn a_sliced_hour_compares_the_countries_read_and_their_last_two_months() {
        // GER was read now: its window (September, October) is recorded; its
        // August entry is not expected. SOV was not read: not compared.
        let frame = frame(
            vec![
                row_at(SEPTEMBER, 0, 1326, 200_000),
                row_at(DATE, 0, 1326, 300_000),
            ],
            &[GER],
        );
        let save = save(&[
            month_at("equipment", 0, "1943.8.30.11", &[(1326, "1")]),
            month_at("equipment", 0, "1943.9.30.11", &[(1326, "2")]),
            month("equipment", 0, &[(1326, "3")]),
        ]);
        let report = run(&frame, &save, false).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["CombatLogEquipment"], 2);
        // The same rows at a full hour miss August and SOV.
        let report = run(&frame, &save, true).unwrap();
        assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
        let _ = AUGUST;
    }

    /// A country whose last complete read is older is read whole outside a
    /// full hour: its older months are compared, never differences for
    /// being there.
    #[test]
    fn a_whole_read_at_a_sliced_hour_compares_its_older_months() {
        let save = save(&[
            month_at("equipment", 0, "1943.8.30.11", &[(1326, "1")]),
            month_at("equipment", 0, "1943.9.30.11", &[(1326, "2")]),
            month("equipment", 0, &[(1326, "3")]),
        ]);
        let whole = |august: i64| {
            frame(
                vec![
                    row_at(AUGUST, 0, 1326, august),
                    row_at(SEPTEMBER, 0, 1326, 200_000),
                    row_at(DATE, 0, 1326, 300_000),
                ],
                &[GER],
            )
        };
        let report = run(&whole(100_000), &save, false).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["CombatLogEquipment"], 3);
        let report = run(&whole(100_001), &save, false).unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        assert!(report.differences[0].contains("amount_raw"));
    }

    #[test]
    fn a_missing_log_without_theatre_definitions_is_a_difference() {
        let frame = frame(
            vec![CombatLogEquipmentRow {
                group: 777,
                ..row(0, 1326, 900_000)
            }],
            &[GER],
        );
        let mut save = save(&[]);
        save.naval_history.theatre_countries = None;
        assert!(save.naval_history.theatre_countries.is_none());
        let report = run(&frame, &save, false).unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        assert!(report.differences[0].contains("group 777"));
    }

    #[test]
    fn a_deleted_logs_final_rows_are_listed_not_compared() {
        let frame = frame(
            vec![
                row(0, 1326, 300_000),
                CombatLogEquipmentRow {
                    group: 777,
                    ..row(0, 1326, 900_000)
                },
            ],
            &[GER],
        );
        let mut save = save(&[month("equipment", 0, &[(1326, "3")])]);
        // The saved census still contains the live group, but not group 777.
        theatre_groups(&mut save, b"GER", &[12793]);
        let report = run(&frame, &save, false).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("daily prune deleted"))
        );
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("completeness is unproven"))
        );
    }

    #[test]
    fn a_saved_reason_outside_the_arrays_is_refused() {
        let save = save(&[month("equipment", 4, &[(1326, "1")])]);
        assert!(run(&frame(vec![], &[]), &save, true).is_err());
    }

    #[test]
    fn lend_lease_compares_every_saved_amount_and_receiver() {
        let exchange = |sunk: &str| {
            format!(
                "resources={{ lend_lease={{ receiver=\"SOV\" id={{ id=97730 type=4713 }} total_delivered={{ equipment={{ id={{ id=1723 type=70 }} amount=18 }} allow_zero_entries=no }} equipment_sunk={{ {sunk} allow_zero_entries=no }} }} produced={{}} transfer_overlord_subject={{}} imported={{}} to_use={{ {{}} {{}} {{}} }} to_export={{}} base_export={{}} exported={{}} }}"
            )
        };
        let rows = vec![
            LendLeaseEquipmentRow {
                giver: GER,
                exchange: 97730,
                kind: DELIVERED_KIND,
                variant: 1723,
                receiver: SOV,
                amount_raw: 1_800_000,
            },
            LendLeaseEquipmentRow {
                giver: GER,
                exchange: 97730,
                kind: SUNK_KIND,
                variant: 5157,
                receiver: SOV,
                amount_raw: 0,
            },
        ];
        let tables: [TableView; TableId::ALL.len()] = std::array::from_fn(|index| {
            let table = TableId::ALL[index];
            if table == TableId::LendLeaseEquipment {
                TableView::Complete(TableRows::LendLeaseEquipment(rows.clone()))
            } else {
                TableView::Complete(TableRows::empty(table))
            }
        });
        let frame = DecodedFrame {
            branch: 1,
            hour: DATE,
            metrics: Default::default(),
            tables: tables.map(Into::into),
        };
        let countries = links();
        let links = BTreeMap::from([(GER, &countries[0]), (SOV, &countries[1])]);
        let save = save_with(&[], &exchange(""));
        let mut report = Report::default();
        lend_lease(&frame, &save, &links, &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["LendLeaseEquipment"], 1);
        let save = save_with(
            &[],
            &exchange("equipment={ id={ id=5157 type=70 } amount=0.5 }"),
        );
        let mut report = Report::default();
        lend_lease(&frame, &save, &links, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    }
}
