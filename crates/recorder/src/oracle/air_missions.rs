//! The flying-aircraft map's mission and sortie tables. The text save holds
//! the bases, mines and reassignment penalties they name, but not the
//! registration order, the country vectors or the other sortie operands;
//! those are checked against the same hour's censuses.

use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use installed_content_save::Save;
use replay::{
    frame::DecodedFrame,
    rows::{AirSortieModifierRow, AirWingRow, Recorded, tables},
};

use super::{CountryContext, Report, complete, invalid};

pub(super) fn compare(
    frame: &DecodedFrame,
    save: &Save,
    countries: &CountryContext<'_>,
    report: &mut Report,
) {
    report.attempt_table(replay::rows::TableId::AirRegionalMissions, |report| {
        report.compared("AirRegionalMissions", registrations(frame, countries)?);
        report.limitation(
            "The save stores each wing's mission, not CStrategicAir's per-region registration vectors. Membership, owner and region are checked against AirWings and the ordinals for contiguity; their native order needs a live comparison.",
            "AirRegionalMissions order",
        );
        Ok(())
    });
    report.attempt_table(replay::rows::TableId::AirCountryRelations, |report| {
        let compared = relations(frame, save, countries, report)?;
        report.compared("AirCountryRelations", compared);
        report.limitation(
            "The save does not serialize the reciprocal-donor, wartime-ally or enemy vectors. Owners, references and contiguous positions are checked, and each enemy set against the saved wars or, where CountryWars read the same vector, exactly against that read; the native order of the other vectors needs a live comparison.",
            "AirCountryRelations order",
        );
        Ok(())
    });
    report.attempt_table(replay::rows::TableId::AirReferenceWitnesses, |report| {
        witnesses(frame, save, report)
    });
    report.attempt_table(replay::rows::TableId::NavalMineLevels, |report| {
        mines(frame, save, countries, report)
    });
    report.attempt_table(replay::rows::TableId::AirMissionInputs, |report| {
        let rows = mission_inputs(frame, save, report)?;
        report.compared("AirMissionInputs", rows);
        for row in complete::<tables::AirMissionInputs>(frame)? {
            if matches!(&row.cas_ratio_raw, Recorded::NotRecorded) {
                report.limitation(
                    "The older recording did not capture the CAS diversion ratio.",
                    format!("AirMissionInputs {} cas_ratio_raw", row.wing),
                );
            }
        }
        report.limitation(
            "The save holds no other sortie operand: mission coverage, last target, reached provinces and pool-definition flags are checked for structure only. The mission's CAS diversion ratio has no saved counterpart; compare its recorded value or read refusal with an independent native read at the same paused hour.",
            "AirMissionInputs",
        );
        Ok(())
    });
    report.attempt("air naval-patrol inputs", |report| {
        let (combats, histories) = naval_patrol(frame, countries, report)?;
        report.compared("AirNavalPatrolCombats", combats);
        report.compared("AirNavalPatrolHistories", histories);
        report.limitation(
            "The save orders no fleets or task forces and holds no air history window 0: owners, contiguous ordinals, availability witnesses and sparse ring indices are checked. Channels 43/3 and 10/17/51/58/44/24/65 require independent native window-zero reads and ordered ring sums at the same paused hour; saved annual air history does not certify them.",
            "AirNavalPatrolCombats / AirNavalPatrolHistories",
        );
        Ok(())
    });
    report.attempt("air sortie operands", |report| {
        sortie_operands(frame, countries, report)?;
        report.limitation(
            "The save holds no base containers, key-0x11 modifiers, host-base operands or modifier metadata: their keys are checked against the registered missions and their bases, and their values need a live comparison.",
            "AirSortieBaseCountries / AirSortieModifiers / AirSortieSpecialBases / AirSortieModifierMetadata",
        );
        Ok(())
    });
    report.attempt_table(replay::rows::TableId::AirWings, |report| {
        let wings = complete::<tables::AirWings>(frame)?;
        for wing in wings.iter().filter(|wing| wing.mission_mask & 8 != 0) {
            let subject = format!("AirWings {} strategic_bombing_raw", wing.id);
            match wing.strategic_bombing_raw {
                Recorded::NotRecorded => report.limitation(
                    "The older recording did not capture the selected strategic bombing statistic.",
                    subject,
                ),
                Recorded::Value(_) => report.limitation(
                    "The selected mission-8 stat 0x32 is not serialized. Its recorded value or read refusal requires an independent native read at the same paused hour.",
                    subject,
                ),
            }
        }
        Ok(())
    });
}

/// Naval-patrol rows name live countries; task-force ordinals are
/// contiguous from zero, and history rows retain sparse values and refusals.
fn naval_patrol(
    frame: &DecodedFrame,
    countries: &CountryContext<'_>,
    report: &mut Report,
) -> io::Result<(usize, usize)> {
    let slots: BTreeSet<_> = countries
        .by_tag
        .values()
        .map(|country| country.canonical_slot)
        .collect();
    let combats = complete::<tables::AirNavalPatrolCombats>(frame)?;
    let histories = complete::<tables::AirNavalPatrolHistories>(frame)?;
    let mut ordinals = BTreeMap::<u32, BTreeSet<u32>>::new();
    for row in combats {
        if !slots.contains(&row.country) || row.region == 0 {
            return Err(invalid(format!(
                "naval-patrol combat {}:{} names no live country or region",
                row.country, row.ordinal
            )));
        }
        ordinals.entry(row.country).or_default().insert(row.ordinal);
    }
    contiguous(ordinals.values(), "naval-patrol combat")?;
    histories_structure(histories, &slots)?;
    for row in histories {
        if matches!(row.scenario_channels, Recorded::Value(None)) {
            report.limitation(
                "AirNavalPatrolHistories.scenario_channels unavailable after a current read refusal; original naval-patrol history remains readable",
                format!("air history {}:{}:{}", row.country, row.region, row.index),
            );
        }
    }
    Ok((combats.len(), histories.len()))
}

/// Value and refused rows retain native window-zero positions. Availability witnesses
/// also cover null/reserved native slots, which need not have a saved tag.
fn histories_structure(
    rows: &[replay::rows::AirNavalPatrolHistoryRow],
    slots: &BTreeSet<u32>,
) -> io::Result<()> {
    let mut witnesses = BTreeSet::new();
    let mut values = BTreeSet::new();
    for row in rows {
        if row.is_availability_witness() {
            if row != &replay::rows::AirNavalPatrolHistoryRow::availability_witness(row.country)
                || !witnesses.insert(row.country)
            {
                return Err(invalid(
                    "air history availability witness is malformed or repeated",
                ));
            }
            continue;
        }
        if !slots.contains(&row.country)
            || !row.has_values()
            || row.index == replay::rows::AIR_HISTORY_AVAILABILITY_INDEX
        {
            return Err(invalid(format!(
                "air history {}:{}:{} names no live country, valid ring index or value",
                row.country, row.region, row.index
            )));
        }
        if matches!(&row.scenario_channels, Recorded::Value(_)) {
            values.insert(row.country);
        }
    }
    if !values.is_subset(&witnesses) {
        return Err(invalid(
            "air scenario history values lack their availability witness",
        ));
    }
    Ok(())
}

/// One row per registered mission; reached IDs only for membership masks.
/// The reassignment penalty is the save's `region_change_penalty`.
fn mission_inputs(frame: &DecodedFrame, save: &Save, report: &mut Report) -> io::Result<usize> {
    let wings: BTreeMap<_, _> = complete::<tables::AirWings>(frame)?
        .iter()
        .map(|wing| (wing.id, wing))
        .collect();
    let registered: BTreeSet<_> = complete::<tables::AirRegionalMissions>(frame)?
        .iter()
        .map(|row| row.wing)
        .collect();
    let rows = complete::<tables::AirMissionInputs>(frame)?;
    if rows
        .iter()
        .map(|row| row.wing)
        .ne(registered.iter().copied())
    {
        return Err(invalid(
            "air mission inputs differ from the registered missions",
        ));
    }
    for row in rows {
        let mask = wings
            .get(&row.wing)
            .ok_or_else(|| invalid("air mission input lacks its AirWings row"))?
            .mission_mask;
        let membership = mask & replay::rows::AIR_MISSION_REACHED_IDS != 0;
        if (membership && row.reached_provinces.len() != row.reached_count as usize)
            || (!membership && !row.reached_provinces.is_empty())
        {
            return Err(invalid(format!(
                "air mission {} reached provinces disagree with its mask or count",
                row.wing
            )));
        }
        let saved = save
            .air_wings
            .get(&row.wing)
            .and_then(|wing| wing.region_change_penalty);
        if saved != Some(row.reassignment_penalty_raw) {
            report.difference(
                format!("air mission {} region_change_penalty", row.wing),
                row.reassignment_penalty_raw,
                saved.map_or_else(|| "absent".to_owned(), |raw| raw.to_string()),
            );
        }
    }
    Ok(rows.len())
}

/// The keys the collector must have read for the registered missions: their
/// pool bases (with the witnessed kind and state), the effective countries
/// and mission regions. Only nonzero operands are rows.
struct SortieKeys {
    bases: BTreeMap<u64, (i32, Option<u32>)>,
    countries: BTreeSet<u32>,
    positive: BTreeSet<u32>,
    regions: BTreeSet<u32>,
    country_regions: BTreeSet<(u32, u32)>,
}

fn sortie_keys(frame: &DecodedFrame, countries: &CountryContext<'_>) -> io::Result<SortieKeys> {
    let wings: BTreeMap<_, _> = complete::<tables::AirWings>(frame)?
        .iter()
        .map(|wing| (wing.id, wing))
        .collect();
    let manager: BTreeMap<_, _> = complete::<tables::AirBases>(frame)?
        .iter()
        .map(|base| (base.id, (base.kind, base.state)))
        .collect();
    let mut witnessed = BTreeMap::new();
    for row in complete::<tables::AirReferenceWitnesses>(frame)? {
        let base = match row.base_kind {
            Some(kind) => (kind, row.state),
            None => *manager
                .get(&row.handle)
                .ok_or_else(|| invalid("handle-only air reference witness has no AirBases row"))?,
        };
        witnessed.insert(row.handle, base);
    }
    let mut keys = SortieKeys {
        bases: BTreeMap::new(),
        countries: countries
            .by_tag
            .values()
            .map(|country| country.canonical_slot)
            .collect(),
        positive: BTreeSet::new(),
        regions: BTreeSet::new(),
        country_regions: BTreeSet::new(),
    };
    for registration in complete::<tables::AirRegionalMissions>(frame)? {
        let wing: &AirWingRow = wings
            .get(&registration.wing)
            .ok_or_else(|| invalid("registered air mission lacks its AirWings row"))?;
        if let Some(&base) = witnessed.get(&wing.air_base) {
            keys.bases.insert(wing.air_base, base);
        }
        let effective = if wing.country_override > 0 {
            wing.country_override as u32
        } else {
            wing.owner
        };
        let slot = countries
            .links
            .get(&effective)
            .ok_or_else(|| invalid("registered air wing country is not a linked country"))?
            .canonical_slot;
        let region = wing
            .mission_region
            .ok_or_else(|| invalid("registered air mission has no region"))?;
        keys.regions.insert(region);
        if (effective as i32) > 0 {
            keys.positive.insert(slot);
            keys.country_regions.insert((slot, region));
        }
    }
    Ok(keys)
}

fn sortie_operands(
    frame: &DecodedFrame,
    countries: &CountryContext<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let keys = sortie_keys(frame, countries)?;
    let containers = complete::<tables::AirSortieBaseCountries>(frame)?;
    for row in containers {
        if !keys.bases.contains_key(&row.base)
            || !keys.countries.contains(&row.country)
            || row.fuel_demand_raw.is_some() != row.fuel_allocated_raw.is_some()
            || [
                row.capacity_raw,
                row.supply_raw,
                row.fuel_demand_raw.unwrap_or(0),
                row.fuel_allocated_raw.unwrap_or(0),
            ] == [0; 4]
        {
            return Err(invalid(format!(
                "air sortie container {}:{} names no registered base or live country, is zero or has half its fuel",
                row.base, row.country
            )));
        }
    }
    report.compared("AirSortieBaseCountries", containers.len());

    let modifiers = complete::<tables::AirSortieModifiers>(frame)?;
    for row in modifiers {
        let keyed = match row.scope {
            AirSortieModifierRow::WEATHER => row.subject == 0 && keys.regions.contains(&row.region),
            AirSortieModifierRow::COUNTRY => {
                row.region == 0 && keys.positive.contains(&row.subject)
            }
            AirSortieModifierRow::COUNTRY_REGION => {
                keys.country_regions.contains(&(row.subject, row.region))
            }
            // Every console override is a row, zero included.
            AirSortieModifierRow::EFFICIENCY_OVERRIDE => row.region == 0,
            _ => false,
        };
        if !keyed || (row.raw == 0 && row.scope != AirSortieModifierRow::EFFICIENCY_OVERRIDE) {
            return Err(invalid(format!(
                "air sortie modifier {}:{}:{} is neither a registered mission's nonzero term nor an override",
                row.scope, row.subject, row.region
            )));
        }
    }
    report.compared("AirSortieModifiers", modifiers.len());

    let special = complete::<tables::AirSortieSpecialBases>(frame)?;
    for row in special {
        let Some(&(kind, state)) = keys.bases.get(&row.base) else {
            return Err(invalid(format!(
                "air sortie special base {} is not a registered pool base",
                row.base
            )));
        };
        let host = state.is_none() && kind != 2;
        let consistent = (row.rocket_slots.is_none() || (kind == 1 && state.is_some()))
            && (row.carrier_stance.is_none() || (host && kind != 1))
            && (row.leader_modifier_raw == 0 || host)
            && (row.task_force_combats.is_none() || host)
            && (row.task_force_region.is_none() || row.task_force_combats.is_some_and(|n| n > 0))
            && (row.rocket_slots.is_some()
                || row.carrier_stance.is_some()
                || row.leader_modifier_raw != 0
                || row.task_force_combats.is_some());
        if !consistent {
            return Err(invalid(format!(
                "air sortie special base {} operands disagree with its kind {kind} and state",
                row.base
            )));
        }
    }
    report.compared("AirSortieSpecialBases", special.len());

    let [metadata] = complete::<tables::AirSortieModifierMetadata>(frame)? else {
        return Err(invalid("air sortie modifier metadata is not a singleton"));
    };
    if metadata.callback_kind > 1 || (metadata.feature_selector == 0 && metadata.callback_kind != 0)
    {
        return Err(invalid(
            "air sortie modifier metadata names an unknown feature callback",
        ));
    }
    report.compared("AirSortieModifierMetadata", 1);
    Ok(())
}

fn registrations(frame: &DecodedFrame, countries: &CountryContext<'_>) -> io::Result<usize> {
    let wings: BTreeMap<_, _> = complete::<tables::AirWings>(frame)?
        .iter()
        .map(|wing| (wing.id, wing))
        .collect();
    let rows = complete::<tables::AirRegionalMissions>(frame)?;
    let mut ordinals = BTreeMap::<(u32, u32), BTreeSet<u32>>::new();
    for row in rows {
        let wing = wings
            .get(&row.wing)
            .ok_or_else(|| invalid("registered air mission lacks its AirWings row"))?;
        let owner = countries
            .links
            .get(&wing.owner)
            .ok_or_else(|| invalid("registered air mission owner is not a linked country"))?;
        let region = wing
            .mission_region
            .ok_or_else(|| invalid("registered air mission has no region"))?;
        if !ordinals
            .entry((owner.canonical_slot, region))
            .or_default()
            .insert(row.ordinal)
        {
            return Err(invalid("regional air mission ordinal repeats"));
        }
    }
    contiguous(ordinals.values(), "regional air mission")?;
    Ok(rows.len())
}

/// Structure, then each country's enemy set: as a set, wartime-ally rebuilding
/// fills the enemy-country set with the other endpoint of each of the
/// country's wars, which the save lists as `war_relations`.
///
/// That fails where the vector can lag its wars, the rows the same hour's
/// CountryWars marks `enemies_read`: a civil-war revolter, whose transferred
/// wars the game creates without rebuilding its vector and which
/// civil-war creation leaves unrebuilt when the revolter's declaration
/// fails the war-declaration eligibility test and returns before the
/// declaration would rebuild it, and a country
/// owning a retired war relation. CountryWars read the same stored vector,
/// so there it must equal the recorded one exactly, order included, and the
/// save comparison is a named limitation.
fn relations(
    frame: &DecodedFrame,
    save: &Save,
    countries: &CountryContext<'_>,
    report: &mut Report,
) -> io::Result<usize> {
    let slots: BTreeSet<_> = countries
        .by_tag
        .values()
        .map(|country| country.canonical_slot)
        .collect();
    let mut read = BTreeMap::<u32, &[u32]>::new();
    for row in complete::<tables::CountryWars>(frame)? {
        if row.enemies_read == 0 {
            continue;
        }
        let country = countries
            .links
            .get(&row.country)
            .ok_or_else(|| invalid("CountryWars row names no linked country"))?;
        if read
            .insert(country.canonical_slot, row.enemies.as_slice())
            .is_some()
        {
            return Err(invalid("CountryWars reads one country's enemies twice"));
        }
    }
    let rows = complete::<tables::AirCountryRelations>(frame)?;
    let mut ordinals = BTreeMap::<(u32, u8), BTreeSet<u32>>::new();
    let mut enemies = BTreeMap::<u32, BTreeSet<u32>>::new();
    let mut vectors = BTreeMap::<u32, BTreeMap<u32, u32>>::new();
    for row in rows {
        if !slots.contains(&row.country)
            || row.kind > replay::rows::AirCountryRelationRow::ENEMY
            || !countries.links.contains_key(&row.other)
        {
            return Err(invalid(format!(
                "air country relation {}:{}:{} names no live country or kind",
                row.country, row.kind, row.ordinal
            )));
        }
        ordinals
            .entry((row.country, row.kind))
            .or_default()
            .insert(row.ordinal);
        if row.kind == replay::rows::AirCountryRelationRow::ENEMY {
            enemies
                .entry(row.country)
                .or_default()
                .insert(countries.links[&row.other].canonical_slot);
            vectors
                .entry(row.country)
                .or_default()
                .insert(row.ordinal, row.other);
        }
    }
    contiguous(ordinals.values(), "air country relation")?;
    let mut expected = BTreeMap::<u32, BTreeSet<u32>>::new();
    let mut cancelled = BTreeSet::new();
    for war in &save.war_relations {
        let first = super::saved_country_slot(&war.first, save, countries)?;
        let second = super::saved_country_slot(&war.second, save, countries)?;
        if war.cancel {
            cancelled.extend([first, second]);
            continue;
        }
        expected.entry(first).or_default().insert(second);
        expected.entry(second).or_default().insert(first);
    }
    let empty = BTreeSet::new();
    for (tag, country) in &countries.by_tag {
        let slot = country.canonical_slot;
        let tag = String::from_utf8_lossy(tag);
        if let Some(&read) = read.get(&slot) {
            let recorded: Vec<u32> = vectors
                .get(&slot)
                .map(|vector| vector.values().copied().collect())
                .unwrap_or_default();
            if recorded != read {
                report.difference(
                    format!("{tag} air enemies against CountryWars"),
                    recorded,
                    read,
                );
            }
            report.limitation(
                "CountryWars read this country's enemy vector because it can lag the saved wars (a civil-war revolter whose transferred wars were never rebuilt into it, or the owner of a retired war relation), so it is compared exactly with that read, not with the saved wars",
                tag,
            );
            continue;
        }
        if cancelled.contains(&slot) {
            report.limitation(
                "A cancelled saved war may still hold its status, so the enemy set of a country in one is not compared",
                tag,
            );
            continue;
        }
        let actual = enemies.get(&slot).unwrap_or(&empty);
        let wars = expected.get(&slot).unwrap_or(&empty);
        if actual != wars {
            report.difference(format!("{tag} air enemies"), actual, wars);
        }
    }
    Ok(rows.len())
}

fn contiguous<'a>(groups: impl Iterator<Item = &'a BTreeSet<u32>>, label: &str) -> io::Result<()> {
    for positions in groups {
        if positions.iter().copied().ne(0..positions.len() as u32) {
            return Err(invalid(format!("{label} positions are not contiguous")));
        }
    }
    Ok(())
}

/// A resolved base must be a saved base with the recorded fields; a probed
/// handle with no witness must be unsaved. Match capture_references: registered
/// active pools, idle pools even with a retained region,
/// unregistered training pools without a region, and every transfer base.
fn witnesses(frame: &DecodedFrame, save: &Save, report: &mut Report) -> io::Result<()> {
    let saved: BTreeMap<_, _> = save.air.bases.iter().map(|base| (base.id, base)).collect();
    let manager: BTreeMap<_, _> = complete::<tables::AirBases>(frame)?
        .iter()
        .map(|base| (base.id, base))
        .collect();
    let wings = complete::<tables::AirWings>(frame)?;
    let by_wing: BTreeMap<_, _> = wings.iter().map(|wing| (wing.id, wing)).collect();
    let mut probed: BTreeSet<u64> = wings
        .iter()
        .map(|wing| wing.base_override)
        .chain(
            wings
                .iter()
                .filter(|wing| {
                    wing.mission_mask == 0
                        || (wing.mission_region.is_none()
                            && wing.mission_mask & replay::rows::AIR_MISSION_TRAINING != 0)
                })
                .map(|wing| wing.air_base),
        )
        .filter(|&handle| handle != 0)
        .collect();
    for registration in complete::<tables::AirRegionalMissions>(frame)? {
        let wing = by_wing
            .get(&registration.wing)
            .ok_or_else(|| invalid("registered air mission lacks its AirWings row"))?;
        probed.insert(wing.air_base);
    }
    let mut compared = 0;
    for row in complete::<tables::AirReferenceWitnesses>(frame)? {
        let label = format!("air reference witness {}", row.handle);
        if !probed.remove(&row.handle) {
            report.difference(&label, "present", "unprobed");
            continue;
        }
        let Some(expected) = saved.get(&row.handle) else {
            report.difference(&label, "resolved", "unsaved");
            continue;
        };
        let (kind, state, host) = match row.base_kind {
            Some(kind) => (kind, row.state, row.host.unwrap_or(0)),
            None => {
                let base = manager.get(&row.handle).ok_or_else(|| {
                    invalid("handle-only air reference witness has no AirBases row")
                })?;
                (base.kind, base.state, base.host)
            }
        };
        if row.host_without_task_force && (row.host_owner.is_some() || state.is_some() || host == 0)
        {
            return Err(invalid(format!(
                "{label} marks a null host task force on a base without a resolved host"
            )));
        }
        compared += 1;
        if (kind, state) != (expected.kind, expected.state) {
            report.difference(
                format!("{label}.kind/state"),
                format!("{kind}/{state:?}"),
                format!("{}/{:?}", expected.kind, expected.state),
            );
        }
        if let Some(expected) = expected.host
            && host != expected
        {
            report.difference(format!("{label}.host"), host, expected);
        }
    }
    for handle in probed {
        if saved.contains_key(&handle) {
            report.difference(
                format!("air reference witness {handle}"),
                "unresolved",
                "saved",
            );
        }
    }
    report.compared("AirReferenceWitnesses", compared);
    Ok(())
}

/// The recorded pairs are those of the registered mine-laying wings, and each
/// equals the owner's saved `per_region_mines` entry.
fn mines(
    frame: &DecodedFrame,
    save: &Save,
    countries: &CountryContext<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let tags: BTreeMap<_, _> = countries
        .by_tag
        .iter()
        .map(|(tag, country)| (country.canonical_slot, *tag))
        .collect();
    let wings: BTreeMap<_, _> = complete::<tables::AirWings>(frame)?
        .iter()
        .map(|wing| (wing.id, wing))
        .collect();
    let mut required = BTreeSet::new();
    for registration in complete::<tables::AirRegionalMissions>(frame)? {
        let wing = wings
            .get(&registration.wing)
            .ok_or_else(|| invalid("registered air mission lacks its AirWings row"))?;
        if wing.mission_mask & replay::rows::AIR_MISSION_LAY_MINES == 0 {
            continue;
        }
        let effective = if wing.country_override > 0 {
            wing.country_override as u32
        } else {
            wing.owner
        };
        let country = countries
            .links
            .get(&effective)
            .ok_or_else(|| invalid("mine-laying wing country is not a linked country"))?;
        if let Some(region) = wing.mission_region {
            required.insert((country.canonical_slot, region));
        }
    }
    let rows = complete::<tables::NavalMineLevels>(frame)?;
    let recorded: BTreeSet<_> = rows.iter().map(|row| (row.country, row.region)).collect();
    if recorded != required {
        report.difference(
            "naval mine level pairs",
            recorded.len(),
            format!("{} registered mine-laying pairs", required.len()),
        );
    }
    for row in rows {
        let tag = tags
            .get(&row.country)
            .ok_or_else(|| invalid("naval mine level names no live country"))?;
        let field = format!(
            "naval mines {} region {}",
            String::from_utf8_lossy(tag),
            row.region
        );
        let saved = save
            .countries
            .get(*tag)
            .map(|country| country.naval_mines.as_ref());
        match saved_mines(saved, row.region) {
            Ok(saved) if saved == row.raw_mines => {}
            Ok(saved) => report.difference(field, row.raw_mines, saved),
            Err(missing) => report.difference(field, row.raw_mines, missing),
        }
    }
    report.compared("NavalMineLevels", rows.len());
    Ok(())
}

/// The saved mines of `region`. A recorded row read an element of the nonempty
/// navy mine-region vector (the collector refuses a region outside it),
/// and the writer emits `per_region_mines` whenever that vector's
/// count is nonzero, listing only positive entries. So only a region
/// missing from a present block is zero; a missing country or block
/// contradicts the recording and is reported, never read as zero.
/// `saved` is the country's parsed block, `None` when the country is absent.
fn saved_mines(
    saved: Option<Option<&BTreeMap<u32, installed_content_save::Decimal>>>,
    region: u32,
) -> Result<i64, &'static str> {
    let mines = saved
        .ok_or("country absent from the save")?
        .ok_or("per_region_mines absent though the vector is nonempty")?;
    Ok(mines.get(&region).map_or(0, |amount| amount.raw))
}

#[cfg(test)]
#[path = "air_missions/witness_tests.rs"]
mod witness_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use installed_content_save::Decimal;
    use replay::{
        frame::TableView,
        rows::{
            AirCountryRelationRow, AirNavalPatrolHistoryRow, CountryLinkRow, CountryWarRow,
            TableId, TableRows,
        },
    };

    #[test]
    fn naval_patrol_histories_are_sparse_nonzero_entries() {
        let slots = BTreeSet::from([4]);
        let row = |index, sunk, lost| AirNavalPatrolHistoryRow {
            country: 4,
            region: 9,
            index,
            enemy_ships_sunk_raw: sunk,
            our_fighters_lost_raw: lost,
            scenario_channels: Recorded::NotRecorded,
        };
        // Indices 0 and 7 with the zero entries between them dropped.
        assert!(histories_structure(&[row(0, 5, 0), row(7, 0, -2)], &slots).is_ok());
        assert!(histories_structure(&[row(3, 0, 0)], &slots).is_err());
        let foreign = AirNavalPatrolHistoryRow {
            country: 5,
            ..row(0, 1, 1)
        };
        assert!(histories_structure(&[foreign], &slots).is_err());
    }

    #[test]
    fn scenario_histories_validate_empty_witnesses_and_new_channel_only_records() {
        let slots = BTreeSet::from([4]);
        let witness = replay::rows::AirNavalPatrolHistoryRow::availability_witness;
        let row = AirNavalPatrolHistoryRow {
            country: 4,
            region: 9,
            index: 7,
            scenario_channels: Recorded::Value(Some([0, 0, 0, 0, 0, -2, 0])),
            ..Default::default()
        };
        // Native null/reserved country-air slots have no save country.
        assert!(histories_structure(&[witness(0), witness(4)], &slots).is_ok());
        assert!(histories_structure(&[row.clone(), witness(4)], &slots).is_ok());
        assert!(histories_structure(&[row], &slots).is_err());
        assert!(histories_structure(&[witness(4), witness(4)], &slots).is_err());
        let mut malformed = witness(4);
        malformed.scenario_channels = Recorded::NotRecorded;
        assert!(histories_structure(&[malformed], &slots).is_err());
        let mut malformed = witness(4);
        malformed.enemy_ships_sunk_raw = 1;
        assert!(histories_structure(&[malformed], &slots).is_err());
        let mut malformed = witness(4);
        malformed.region = 9;
        assert!(histories_structure(&[malformed], &slots).is_err());
    }

    #[test]
    fn current_scenario_refusals_remain_independent_named_history_rows() {
        let (mut frame, _, _, _) = super::super::tests::fixture();
        let rows = vec![
            AirNavalPatrolHistoryRow {
                country: 7,
                region: 9,
                index: 0,
                scenario_channels: Recorded::Value(None),
                ..Default::default()
            },
            AirNavalPatrolHistoryRow {
                country: 7,
                region: 9,
                index: 1,
                enemy_ships_sunk_raw: 300_000,
                our_fighters_lost_raw: 200_000,
                scenario_channels: Recorded::Value(None),
            },
            AirNavalPatrolHistoryRow {
                country: 7,
                region: 8,
                index: 0,
                scenario_channels: Recorded::Value(Some([1; 7])),
                ..Default::default()
            },
            AirNavalPatrolHistoryRow::availability_witness(7),
        ];
        assert!(histories_structure(&rows, &BTreeSet::from([7])).is_ok());
        *frame.tables[TableId::AirNavalPatrolHistories.index()] =
            TableView::Complete(TableRows::AirNavalPatrolHistories(rows));
        let context = super::super::country_context(&frame).unwrap();
        let mut report = Report::default();
        assert_eq!(naval_patrol(&frame, &context, &mut report).unwrap(), (0, 4));
        assert_eq!(
            report.limitations["AirNavalPatrolHistories.scenario_channels unavailable after a current read refusal; original naval-patrol history remains readable"],
            ["air history 7:9:0", "air history 7:9:1"]
        );
    }

    fn add_country(frame: &mut DecodedFrame, saved: &mut Save, id: u32, slot: u32, tag: &[u8]) {
        let mut country = saved.countries[b"GER".as_slice()].clone();
        country.tag = tag.to_vec();
        saved.countries.insert(tag.to_vec(), country);
        let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frame.tables[TableId::Countries.index()]
        else {
            unreachable!()
        };
        let mut row = rows[0].clone();
        row.id = id;
        row.canonical_slot = slot;
        row.tag = tag.to_vec();
        rows.push(row);
        let TableView::Complete(TableRows::CountryLinks(rows)) =
            &mut *frame.tables[TableId::CountryLinks.index()]
        else {
            unreachable!()
        };
        rows.push(CountryLinkRow {
            id,
            canonical_slot: slot,
        });
    }

    fn enemies(frame: &mut DecodedFrame, rows: &[(u32, &[u32])]) {
        *frame.tables[TableId::AirCountryRelations.index()] =
            TableView::Complete(TableRows::AirCountryRelations(
                rows.iter()
                    .flat_map(|&(country, others)| {
                        others.iter().enumerate().map(move |(ordinal, &other)| {
                            AirCountryRelationRow {
                                country,
                                kind: AirCountryRelationRow::ENEMY,
                                ordinal: ordinal as u32,
                                other,
                            }
                        })
                    })
                    .collect(),
            ));
    }

    fn wars(frame: &mut DecodedFrame, rows: Vec<CountryWarRow>) {
        *frame.tables[TableId::CountryWars.index()] =
            TableView::Complete(TableRows::CountryWars(rows));
    }

    fn run(frame: &DecodedFrame, saved: &Save) -> Report {
        let context = super::super::country_context(frame).unwrap();
        let mut report = Report::default();
        compare(frame, saved, &context, &mut report);
        assert!(
            report.failed_comparisons.is_empty(),
            "{:?}",
            report.failed_comparisons
        );
        report
    }

    /// A civil war: REB revolted from GER, which was at war with SOV.
    /// The game created the transferred REB-SOV war and ended GER-SOV
    /// (rebuilding GER's and SOV's vectors), but the declaration on REB
    /// failed, so REB's vector was never rebuilt and stays empty while the
    /// save lists REB at war with SOV. CountryWars read that vector; the
    /// oracle holds REB's air enemies to it, not to the saved war.
    #[test]
    fn an_unrebuilt_revolter_vector_is_held_to_the_country_wars_read() {
        let (mut frame, mut saved, _, _) = super::super::tests::fixture();
        // The sortie comparison needs its Static singleton.
        *frame.tables[TableId::AirSortieModifierMetadata.index()] =
            TableView::Complete(TableRows::AirSortieModifierMetadata(vec![
                Default::default(),
            ]));
        // GER is 101 (alias 102) at slot 7.
        add_country(&mut frame, &mut saved, 201, 8, b"REB");
        add_country(&mut frame, &mut saved, 301, 9, b"SOV");
        saved.war_relations.push(installed_content_save::WarRelation {
            first_goals: Vec::new(),
            second_goals: Vec::new(),
            roster_owner: b"REB".to_vec(),
            target: b"SOV".to_vec(),
            first: b"REB".to_vec(),
            second: b"SOV".to_vec(),
            start_date: Some(saved.date),
            end_date: None,
            cancel: false,
            first_casualties: 0,
            second_casualties: 0,
            first_unknown_casualties: 0,
            second_unknown_casualties: 0,
            first_was_instigator: false,
        });
        enemies(&mut frame, &[(9, &[201])]);
        let revolter = |enemies: Vec<u32>| CountryWarRow {
            country: 201,
            can_occupy_non_war: 0,
            enemies_read: 1,
            enemies,
            ordered: Default::default(),
        };
        let limited = |report: &Report| {
            report
                .limitations
                .iter()
                .filter(|(reason, _)| reason.starts_with("CountryWars read"))
                .flat_map(|(_, subjects)| subjects.clone())
                .collect::<Vec<_>>()
        };

        // Without the read, the unrebuilt vector is compared with the saved
        // war and differs.
        wars(&mut frame, Vec::new());
        let report = run(&frame, &saved);
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        assert!(report.differences[0].starts_with("REB air enemies"));

        // With it, the save comparison is a named limitation and the vector
        // equals the read.
        wars(&mut frame, vec![revolter(Vec::new())]);
        let report = run(&frame, &saved);
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(limited(&report), ["REB"]);

        // The read is the same stored vector: a different one, or the same
        // countries in another order, is a difference.
        wars(&mut frame, vec![revolter(vec![301])]);
        let report = run(&frame, &saved);
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        assert!(report.differences[0].starts_with("REB air enemies against CountryWars"));
        enemies(&mut frame, &[(9, &[201]), (8, &[301, 101])]);
        wars(&mut frame, vec![revolter(vec![101, 301])]);
        let report = run(&frame, &saved);
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        wars(&mut frame, vec![revolter(vec![301, 101])]);
        let report = run(&frame, &saved);
        assert!(report.differences.is_empty(), "{:?}", report.differences);

        // Only read vectors are exempt: SOV's still meets the saved war, and
        // a row with the vector not read exempts nothing.
        enemies(&mut frame, &[(8, &[301, 101])]);
        wars(
            &mut frame,
            vec![
                revolter(vec![301, 101]),
                CountryWarRow {
                    country: 301,
                    can_occupy_non_war: 1,
                    enemies_read: 0,
                    enemies: Vec::new(),
                    ordered: Default::default(),
                },
            ],
        );
        let report = run(&frame, &saved);
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        assert!(report.differences[0].starts_with("SOV air enemies"));
    }

    #[test]
    fn only_a_region_missing_from_a_saved_block_is_zero() {
        assert_eq!(saved_mines(None, 4), Err("country absent from the save"));
        assert_eq!(
            saved_mines(Some(None), 4),
            Err("per_region_mines absent though the vector is nonempty")
        );
        let block = BTreeMap::from([(
            4,
            Decimal {
                raw: 250_000,
                fractional_digits: 5,
                lexeme: b"2.5".to_vec(),
            },
        )]);
        assert_eq!(saved_mines(Some(Some(&block)), 4), Ok(250_000));
        assert_eq!(saved_mines(Some(Some(&block)), 9), Ok(0));
    }
}
