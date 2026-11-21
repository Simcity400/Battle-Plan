use std::{
    cell::LazyCell,
    collections::{BTreeMap, BTreeSet},
    io,
};

use content_reader_save::Save;
use replay_data::{rows::CountryRow, rows::FactionRow, rows::TableId, rows::tables};
use replay_frame::frame::DecodedFrame;

use super::{
    CountryContext, Report, TokenNames, complete, country_tag, invalid, required_country_tag,
    tag_text,
};

#[path = "relations/reverse_visibility.rs"]
mod reverse_visibility;

#[path = "relations/reserved_intel.rs"]
mod reserved_intel;

#[path = "relations/faction_goals.rs"]
mod faction_goals;
pub use faction_goals::compare as compare_faction_goals;

pub fn compare_reserved_reverse_visibility(
    frame: &DecodedFrame,
    report: &mut Report,
) -> io::Result<()> {
    reverse_visibility::compare(frame, report)
}

pub fn compare_reserved_country_intel(
    frame: &DecodedFrame,
    save: &Save,
    report: &mut Report,
) -> io::Result<()> {
    reserved_intel::compare(frame, save, report)
}

pub fn saved_country_slot(
    tag: &[u8],
    save: &Save,
    countries: &CountryContext<'_>,
) -> io::Result<u32> {
    if !save.countries.contains_key(tag) {
        return Err(invalid(format!(
            "unknown saved country {}",
            String::from_utf8_lossy(tag)
        )));
    }
    countries
        .by_tag
        .get(tag)
        .map(|country| country.canonical_slot)
        .ok_or_else(|| {
            invalid(format!(
                "saved country {} has no captured country link",
                String::from_utf8_lossy(tag)
            ))
        })
}

pub fn compare_war_relations(
    frame: &DecodedFrame,
    save: &Save,
    countries: &CountryContext<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let links = &countries.links;
    let mut relations = BTreeMap::new();
    for relation in frame.complete::<tables::WarRelations>()? {
        let first = required_country_tag(relation.first_country, links)?;
        let second = required_country_tag(relation.second_country, links)?;
        if first == second || relations.insert((first, second), relation).is_some() {
            return Err(invalid("self or duplicate resolved ordered war relation"));
        }
        if relation.cancel > 1 || relation.first_was_instigator > 1 {
            return Err(invalid(
                "war relation has a raw flag outside 0/1; yes/no save fields cannot verify these bits",
            ));
        }
    }
    let mut saved_pairs = BTreeSet::new();
    for expected in &save.war_relations {
        let first_slot = saved_country_slot(&expected.first, save, countries)?;
        let second_slot = saved_country_slot(&expected.second, save, countries)?;
        if first_slot == second_slot
            || saved_country_slot(&expected.roster_owner, save, countries)? != first_slot
            || saved_country_slot(&expected.target, save, countries)? != second_slot
        {
            return Err(invalid(
                "saved war relation endpoints disagree with their country/target envelope or resolve to one country",
            ));
        }
        if !saved_pairs.insert((first_slot, second_slot)) {
            return Err(invalid("duplicate resolved ordered war relation in save"));
        }
        let label = format!(
            "war relation {} -> {}",
            String::from_utf8_lossy(&expected.first),
            String::from_utf8_lossy(&expected.second)
        );
        let key = (expected.first.as_slice(), expected.second.as_slice());
        let Some(actual) = relations.remove(&key) else {
            report.difference(&label, "absent", "present");
            continue;
        };
        report.war_relations += 1;
        compare_relation_hours(
            report,
            &label,
            (actual.cancel, actual.start_hour, actual.end_hour),
            (expected.cancel, expected.start_date, expected.end_date),
        )?;
        for (field, captured, saved) in [
            (
                "first_casualties",
                actual.first_casualties,
                expected.first_casualties,
            ),
            (
                "second_casualties",
                actual.second_casualties,
                expected.second_casualties,
            ),
            (
                "first_unknown_casualties",
                actual.first_unknown_casualties,
                expected.first_unknown_casualties,
            ),
            (
                "second_unknown_casualties",
                actual.second_unknown_casualties,
                expected.second_unknown_casualties,
            ),
        ] {
            if captured != saved {
                report.difference(format!("{label}.{field}"), captured, saved);
            }
        }
        if (actual.first_was_instigator != 0) != expected.first_was_instigator {
            report.difference(
                format!("{label}.first_was_instigator"),
                actual.first_was_instigator,
                expected.first_was_instigator,
            );
        }
    }
    for (first, second) in relations.keys() {
        report.difference(
            format!(
                "war relation {} -> {}",
                String::from_utf8_lossy(first),
                String::from_utf8_lossy(second)
            ),
            "present",
            "absent",
        );
    }
    report.limitation("war rows identify ordered country relations; grouped-war identity is an app continuity rule", "war relations");
    report.limitation("capture includes only non-retired war relations; the save writes first-owned relations without their retirement flag, so extra saved relations still fail and are never silently filtered", "war relations");
    Ok(())
}

pub fn current_relation_owners<'a>(
    frame: &DecodedFrame,
    table: TableId,
    links: &BTreeMap<u32, &'a CountryRow>,
    report: &Report,
) -> io::Result<Option<BTreeSet<&'a [u8]>>> {
    if report.full_refresh() {
        return Ok(None);
    }
    let id = table.index() as u32;
    let mut current = BTreeSet::new();
    let mut with_rows = BTreeSet::new();
    for read in frame
        .complete::<tables::SliceReads>()?
        .iter()
        .filter(|read| read.table == id)
    {
        with_rows.insert(read.owner);
        if read.sample_hour == frame.hour
            && let Some(tag) = country_tag(read.owner, links)?
        {
            current.insert(tag);
        }
    }
    let slot = frame.hour.rem_euclid(replay_data::cadence::STAGGER_PERIOD) as u32;
    for country in frame.complete::<tables::Countries>()? {
        if !with_rows.contains(&country.id)
            && replay_data::cadence::slice_of(country.id) == slot
            && let Some(tag) = country_tag(country.id, links)?
        {
            current.insert(tag);
        }
    }
    Ok(Some(current))
}

pub fn slice_limitation(
    frame: &DecodedFrame,
    report: &mut Report,
    table: TableId,
    current: &Option<BTreeSet<&[u8]>>,
) {
    if let Some(current) = current {
        let oldest = complete::<tables::SliceReads>(frame)
            .ok()
            .and_then(|reads| {
                reads
                    .iter()
                    .filter(|row| {
                        row.table == table.index() as u32 && row.sample_hour != frame.hour
                    })
                    .map(|row| row.sample_hour)
                    .min()
            });
        if oldest.is_some() {
            report.carried_owners(table, oldest);
        }
        let table = format!("{table:?}");
        report.limitation(
            "relations are read once a day, one owner slice per hour; only the owners read at the oracle hour are compared, and the others hold earlier reads (their hours are in SliceReads)",
            format!("{table}: {} owners read at the oracle hour", current.len()),
        );
    }
}

pub fn compare_relation_opinions(
    frame: &DecodedFrame,
    save: &Save,
    countries: &CountryContext<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let links = &countries.links;
    let current = current_relation_owners(
        frame,
        replay_data::rows::TableId::RelationOpinions,
        links,
        report,
    )?;
    let is_current = |owner: &[u8]| current.as_ref().is_none_or(|tags| tags.contains(owner));
    slice_limitation(
        frame,
        report,
        replay_data::rows::TableId::RelationOpinions,
        &current,
    );
    let mut opinions = BTreeMap::new();
    for row in frame.complete::<tables::RelationOpinions>()? {
        let owner = required_country_tag(row.owner_country, links)?;
        if !is_current(owner) {
            continue;
        }
        let target = required_country_tag(row.target_country, links)?;
        if row.opinion == 0 {
            return Err(invalid(
                "recorded relation opinion is zero; the native serializer omits a zero cached_sum",
            ));
        }
        if opinions.insert((owner, target), row.opinion).is_some() {
            return Err(invalid("duplicate resolved relation opinion pair"));
        }
    }
    let mut saved_pairs = BTreeSet::new();
    for expected in &save.relation_opinions {
        let owner_slot = saved_country_slot(&expected.owner, save, countries)?;
        let target_slot = saved_country_slot(&expected.target, save, countries)?;
        if !saved_pairs.insert((owner_slot, target_slot)) {
            return Err(invalid("saved relation opinion repeats a resolved pair"));
        }
        if !is_current(&expected.owner) {
            continue;
        }
        let label = format!(
            "relation opinion {} -> {}",
            String::from_utf8_lossy(&expected.owner),
            String::from_utf8_lossy(&expected.target)
        );
        let key = (expected.owner.as_slice(), expected.target.as_slice());
        match opinions.remove(&key) {
            Some(actual) => {
                report.compared("RelationOpinions", 1);
                if actual != expected.cached_sum {
                    report.difference(format!("{label}.cached_sum"), actual, expected.cached_sum);
                }
            }
            None => report.difference(&label, "absent", "present"),
        }
    }
    for (owner, target) in opinions.keys() {
        report.difference(
            format!(
                "relation opinion {} -> {}",
                String::from_utf8_lossy(owner),
                String::from_utf8_lossy(target)
            ),
            "present",
            "absent",
        );
    }
    report.limitation(
        "cached opinion is the engine's summed cache; the per-modifier opinion list is not recorded, so an equal sum does not verify its contributing modifiers",
        "RelationOpinions",
    );
    Ok(())
}

pub fn compare_relation_tuples(
    frame: &DecodedFrame,
    save: &Save,
    countries: &CountryContext<'_>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let links = &countries.links;
    let current = current_relation_owners(
        frame,
        replay_data::rows::TableId::RelationTuples,
        links,
        report,
    )?;
    let is_current = |owner: &[u8]| current.as_ref().is_none_or(|tags| tags.contains(owner));
    slice_limitation(
        frame,
        report,
        replay_data::rows::TableId::RelationTuples,
        &current,
    );
    let mut tuples = BTreeMap::new();
    for row in frame.complete::<tables::RelationTuples>()? {
        let owner = required_country_tag(row.owner_country, links)?;
        if !is_current(owner) {
            continue;
        }
        let target = required_country_tag(row.target_country, links)?;
        if owner == target {
            return Err(invalid(
                "recorded relation tuple endpoints resolve to one country",
            ));
        }
        if tuples.insert((owner, target, row.ordinal), row).is_some() {
            return Err(invalid("duplicate resolved relation tuple ordinal"));
        }
        if row.cancel > 1 {
            return Err(invalid(
                "relation tuple has a raw cancel byte outside 0/1; a yes/no save field cannot verify these bits",
            ));
        }
    }
    let mut unnamed = 0;
    for expected in &save.relation_tuples {
        saved_country_slot(&expected.owner, save, countries)?;
        saved_country_slot(&expected.target, save, countries)?;
        if !is_current(&expected.owner) {
            continue;
        }
        let ordinal = u16::try_from(expected.ordinal).map_err(|_| {
            invalid("saved relation ordinal exceeds the recorded ordinal's wire width")
        })?;
        let label = format!(
            "relation {} {} -> {} ordinal {}",
            String::from_utf8_lossy(&expected.kind),
            String::from_utf8_lossy(&expected.owner),
            String::from_utf8_lossy(&expected.target),
            expected.ordinal
        );
        let key = (
            expected.owner.as_slice(),
            expected.target.as_slice(),
            ordinal,
        );
        let Some(actual) = tuples.remove(&key) else {
            report.difference(&label, "absent", "present");
            continue;
        };
        report.compared("RelationTuples", 1);
        match tokens.get(&actual.type_id) {
            Some(&spelling) => {
                if spelling != expected.kind.as_slice() {
                    report.difference(
                        format!("{label}.kind"),
                        String::from_utf8_lossy(spelling).into_owned(),
                        String::from_utf8_lossy(&expected.kind).into_owned(),
                    );
                }
            }
            None => unnamed += 1,
        }
        for (field, captured, saved) in [
            (
                "first",
                required_country_tag(actual.first_country, links)?,
                expected.first.as_slice(),
            ),
            (
                "second",
                required_country_tag(actual.second_country, links)?,
                expected.second.as_slice(),
            ),
        ] {
            if captured != saved {
                report.difference(
                    format!("{label}.{field}"),
                    String::from_utf8_lossy(captured).into_owned(),
                    String::from_utf8_lossy(saved).into_owned(),
                );
            }
        }
        compare_relation_hours(
            report,
            &label,
            (actual.cancel, actual.start_hour, actual.end_hour),
            (expected.cancel, expected.start_date, expected.end_date),
        )?;
    }
    for ((owner, target, ordinal), row) in &tuples {
        report.difference(
            format!(
                "relation type {} {} -> {} ordinal {ordinal}",
                row.type_id,
                String::from_utf8_lossy(owner),
                String::from_utf8_lossy(target)
            ),
            "present",
            "absent",
        );
    }
    if unnamed != 0 {
        report.limitation(
            "the recorded lexical dictionary has no spelling for some relation type tokens, so those rows compare their endpoints, hours and flags but not their class name",
            format!("{unnamed} relation tuples"),
        );
    }
    report.limitation(
        "the retired byte is not serialized, and CWarRelation and CSubjectRelation rows belong to the WarRelations and Subjects tables while still occupying a native ordinal",
        "RelationTuples",
    );
    Ok(())
}

fn compare_relation_hours(
    report: &mut Report,
    label: &str,
    (cancel, start_hour, end_hour): (u8, i32, i32),
    (saved_cancel, start_date, end_date): (
        bool,
        Option<content_reader_save::GameDate>,
        Option<content_reader_save::GameDate>,
    ),
) -> io::Result<()> {
    if (cancel != 0) != saved_cancel {
        report.difference(format!("{label}.cancel"), cancel, saved_cancel);
    }
    if let Some(start) = start_date {
        let start = start.engine_hour()?;
        if start_hour != start {
            report.difference(format!("{label}.start_hour"), start_hour, start);
        }
    } else {
        report.limitation(
            "cancelled relation serializer omits its raw start hour",
            label.to_owned(),
        );
    }
    let emits_end = start_hour < end_hour;
    if emits_end != end_date.is_some() {
        report.difference(
            format!("{label}.end_date presence (start < end)"),
            emits_end,
            end_date.is_some(),
        );
    }
    if let Some(end) = end_date {
        let end = end.engine_hour()?;
        if end_hour != end {
            report.difference(format!("{label}.end_hour"), end_hour, end);
        }
    } else {
        report.limitation(
            "omitted relation end_date proves end_hour <= start_hour, not one exact raw end hour; the inequality is checked",
            label.to_owned(),
        );
    }
    Ok(())
}

pub fn compare_country_intel(
    frame: &DecodedFrame,
    save: &Save,
    countries: &CountryContext<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let links = &countries.links;
    let rows = frame.complete::<tables::CountryIntel>()?;
    let counts = replay_data::rows::CountryIntelRow::native_counts(rows);
    let mut recorded: BTreeMap<&[u8], (u32, BTreeMap<u32, &replay_data::rows::CountryIntelRow>)> =
        BTreeMap::new();
    for row in rows.iter().filter(|row| row.owner_country != 0) {
        let owner = required_country_tag(row.owner_country, links)?;
        let count = counts[&row.owner_country];
        let (known, records) = recorded.entry(owner).or_insert((count, BTreeMap::new()));
        if *known != count {
            return Err(invalid(
                "country intel owner aliases resolve to different record counts",
            ));
        }
        if records.insert(row.target_index, row).is_some() {
            return Err(invalid("duplicate resolved country intel record index"));
        }
    }
    let zero = replay_data::rows::CountryIntelRow::default();
    for (tag, records) in &save.country_intel {
        if reserved_intel::is_serialized_reserved_tag(save, tag) {
            continue;
        }
        saved_country_slot(tag, save, countries)?;
        let owner = String::from_utf8_lossy(tag).into_owned();
        let (count, captured) = recorded.remove(tag.as_slice()).unwrap_or_default();
        let saved = u32::try_from(records.len())
            .map_err(|_| invalid("saved country intel index exceeds its wire width"))?;
        for (index, expected) in records.iter().enumerate() {
            let index = index as u32;
            let label = format!("{owner}.intel[{index}]");
            if index >= count {
                report.difference(&label, "absent", "present");
                continue;
            }
            let actual = captured.get(&index).copied().unwrap_or(&zero);
            report.compared("CountryIntel", 1);
            for (field, captured, saved) in [
                ("army", actual.army, expected.army),
                ("airforce", actual.air, expected.air),
                ("civilian", actual.civilian, expected.civilian),
                ("navy", actual.navy, expected.navy),
            ] {
                if captured != saved {
                    report.difference(format!("{label}.{field}"), captured, saved);
                }
            }
        }
        if count > saved {
            report.difference(
                format!("{owner}.intel[{saved}..{count}]"),
                "present",
                "absent",
            );
        }
    }
    for (tag, (count, _)) in &recorded {
        report.difference(
            format!("{}.intel ({count} records)", String::from_utf8_lossy(tag),),
            "present",
            "absent",
        );
    }
    report.limitation(
        "the native list writer emits nothing for a country whose record count is not positive, so an absent saved list means no records at all, which is a different fact from a list of all-zero records",
        "CountryIntel",
    );
    Ok(())
}

pub fn compare_factions(
    frame: &DecodedFrame,
    save: &Save,
    countries: &CountryContext<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let links = &countries.links;
    let zero_tag = LazyCell::new(|| -> io::Result<&[u8]> {
        let [reserved] = frame.complete::<tables::ReservedCountryNaming>()? else {
            return Err(invalid(
                "reserved faction-member tag requires one naming row",
            ));
        };
        let tag = reserved.tag.as_slice();
        if countries.by_tag.contains_key(tag)
            || (save.countries.contains_key(tag)
                && !reserved_intel::is_serialized_reserved_tag(save, tag))
        {
            return Err(invalid(
                "reserved faction-member tag overlaps a positive country",
            ));
        }
        Ok(tag)
    });
    let mut factions = BTreeMap::new();
    for faction in frame.complete::<tables::Factions>()? {
        if faction.id == 0 || factions.insert(faction.id, faction).is_some() {
            return Err(invalid("null or duplicate full faction handle in replay"));
        }
    }
    let mut members: BTreeMap<u64, BTreeMap<u32, &[u8]>> = BTreeMap::new();
    let mut member_countries = BTreeSet::new();
    for member in frame.complete::<tables::FactionMembers>()? {
        let tag = if member.country == 0 {
            *zero_tag
                .as_ref()
                .map_err(|error| invalid(error.to_string()))?
        } else {
            required_country_tag(member.country, links)?
        };
        if !factions.contains_key(&member.faction) {
            return Err(invalid("faction member references an absent faction"));
        }
        if !member_countries.insert(tag)
            || members
                .entry(member.faction)
                .or_default()
                .insert(member.ordinal, tag)
                .is_some()
        {
            return Err(invalid(
                "duplicate resolved country or ordinal in faction members",
            ));
        }
    }
    for ordered in members.values() {
        if ordered
            .keys()
            .enumerate()
            .any(|(index, &ordinal)| index != ordinal as usize)
        {
            return Err(invalid(
                "faction member ordinals must be contiguous from zero",
            ));
        }
    }
    let tokens: Option<BTreeMap<u32, &[u8]>> =
        frame.complete::<tables::LexicalTokens>().ok().map(|rows| {
            rows.iter()
                .map(|row| (row.token, row.literal.as_slice()))
                .collect()
        });
    let mut saved_members = BTreeSet::new();
    for (&id, expected) in &save.factions {
        if id == 0 || id != expected.id {
            return Err(invalid("invalid saved full faction identity"));
        }
        for tag in &expected.members {
            let slot = if countries.by_tag.contains_key(tag.as_slice()) {
                saved_country_slot(tag, save, countries)?
            } else {
                let reserved = *zero_tag
                    .as_ref()
                    .map_err(|error| invalid(error.to_string()))?;
                if tag.as_slice() != reserved {
                    return Err(invalid(format!(
                        "unknown saved country {}",
                        String::from_utf8_lossy(tag)
                    )));
                }
                0
            };
            if !saved_members.insert(slot) {
                return Err(invalid(
                    "duplicate resolved country in saved faction membership",
                ));
            }
        }
        let label = format!("faction type={} id={}", id as u32, id >> 32);
        let Some(actual) = factions.remove(&id) else {
            report.difference(&label, "absent", "present");
            continue;
        };
        report.factions += 1;
        let name_label = format!("{label}.literal_name");
        let name = (!actual.name.is_empty())
            .then(|| super::serialized_text(report, &name_label, &actual.name));
        if name != expected.name.as_deref() {
            report.difference(name_label, name, expected.name.as_deref());
        }
        compare_faction_presentation(&label, actual, expected, tokens.as_ref(), report);
        let actual_members = members.remove(&id).unwrap_or_default();
        for (ordinal, expected_tag) in expected.members.iter().enumerate() {
            let ordinal_key = u32::try_from(ordinal)
                .map_err(|_| invalid("saved faction membership exceeds u32 ordinals"))?;
            let actual_tag = actual_members.get(&ordinal_key).copied();
            if actual_tag.is_some() {
                report.faction_members += 1;
            }
            if actual_tag != Some(expected_tag.as_slice()) {
                report.difference(
                    format!("{label}.members[{ordinal}]"),
                    tag_text(actual_tag),
                    tag_text(Some(expected_tag)),
                );
            }
        }
        for (&ordinal, &tag) in &actual_members {
            if ordinal as usize >= expected.members.len() {
                report.difference(
                    format!("{label}.members[{ordinal}]"),
                    tag_text(Some(tag)),
                    "absent",
                );
            }
        }
    }
    for id in factions.keys() {
        report.difference(
            format!("faction type={} id={}", *id as u32, id >> 32),
            "present",
            "absent",
        );
    }
    report.limitation("faction names compare only literal bytes; the template's localised name and the rendered friendly name are not recorded (the template is, as its token)", "factions");
    Ok(())
}

fn compare_faction_presentation(
    label: &str,
    actual: &FactionRow,
    expected: &content_reader_save::Faction,
    tokens: Option<&BTreeMap<u32, &[u8]>>,
    report: &mut Report,
) {
    match (actual.template_token, expected.template.as_deref()) {
        (None, None) => {}
        (Some(token), None) => report.difference(
            format!("{label}.template"),
            format!("token {token}"),
            "absent",
        ),
        (None, Some(saved)) => report.difference(
            format!("{label}.template"),
            "absent",
            String::from_utf8_lossy(saved),
        ),
        (Some(token), Some(saved)) => match tokens {
            None => report.limitation(
                "faction template tokens cannot be spelled without a complete LexicalTokens table",
                label,
            ),
            Some(tokens) => match tokens.get(&token).copied() {
                None => report.difference(
                    format!("{label}.template"),
                    format!("token {token} not in LexicalTokens"),
                    String::from_utf8_lossy(saved),
                ),
                Some(spelling) if spelling != saved => report.difference(
                    format!("{label}.template"),
                    String::from_utf8_lossy(spelling),
                    String::from_utf8_lossy(saved),
                ),
                Some(_) => {}
            },
        },
    }
    let written = saved_colour([
        actual.color_red,
        actual.color_green,
        actual.color_blue,
        actual.color_alpha,
    ]);
    if expected.color.as_deref() != Some(written.as_slice()) {
        report.difference(format!("{label}.color"), &written, &expected.color);
    }
    if actual.color_overridden > 1 {
        report.limitation("yes/no is_color_overridden preserves truth but cannot certify noncanonical raw flag bits", label);
    }
    if expected.color_overridden != Some(actual.color_overridden != 0) {
        report.difference(
            format!("{label}.is_color_overridden"),
            actual.color_overridden,
            expected.color_overridden,
        );
    }
}

pub fn saved_colour(components: [u32; 4]) -> Vec<i64> {
    let scale = |bits: u32| {
        let product = f32::from_bits(bits) * 255.0_f32;
        if product.is_nan() || product <= i32::MIN as f32 - 1.0 || product >= 2_147_483_648.0 {
            i64::from(i32::MIN)
        } else {
            i64::from(product as i32)
        }
    };
    let mut written: Vec<i64> = components[..3].iter().map(|&bits| scale(bits)).collect();
    if f32::from_bits(components[3]) != 1.0 {
        written.push(scale(components[3]));
    }
    written
}

#[cfg(test)]
#[path = "relations/zero_member_tests.rs"]
mod zero_member_tests;

#[path = "relations/faction_companions.rs"]
pub mod faction_companions;

#[path = "relations/faction_accounts.rs"]
pub mod faction_accounts;
#[path = "relations/faction_pools.rs"]
pub mod faction_pools;

#[path = "relations/faction_advisors.rs"]
pub mod faction_advisors;
