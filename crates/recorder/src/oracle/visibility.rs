//! Fog-rebuild inputs (L2b): RelationLinks against the saved relations that set
//! its slots, LocalPlayer against the saved `player` tag, and named limitations
//! for the inputs no save serializer writes.

use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use engine::{
    cair_base_access_relation, cattache_relation, cdocking_rights_relation,
    cmilitary_access_relation, coffer_air_base_access_relation, coffer_docking_rights_relation,
    coffer_military_access_relation,
};
use installed_content::save::Save;
use replay::{
    frame::DecodedFrame,
    rows::{RelationLinkRow, tables},
};

use super::{CountryContext, Report, TokenNames, complete, invalid, required_country_tag};

pub(super) fn compare(
    frame: &DecodedFrame,
    save: &Save,
    countries: &CountryContext<'_>,
    tokens: &TokenNames<'_>,
    essential_caps: Result<&BTreeMap<Vec<u8>, i32>, String>,
    report: &mut Report,
) -> io::Result<()> {
    let mut complete_comparison = report
        .attempt_table(replay::rows::TableId::LocalPlayer, |report| {
            compare_local_player(frame, save, countries, report)
        });
    complete_comparison &= report.attempt("visibility RelationLinks", |report| {
        compare_links(frame, save, countries, tokens, report)
    });
    complete_comparison &= report.attempt("visibility NavalRegionAccess", |report| {
        compare_naval_access(frame, save, countries, report)
    });
    complete_comparison &= report.attempt("visibility AceRangeInputs", |report| {
        compare_ace_inputs(frame, save, countries, report)
    });
    complete_comparison &= report.attempt("visibility StockpileArchetypes", |report| {
        compare_stockpile_archetypes(frame, save, countries, tokens, essential_caps, report)
    });
    complete_comparison &=
        report.attempt("visibility inputs", |report| record_inputs(frame, report));
    if complete_comparison {
        Ok(())
    } else {
        Err(invalid(
            "fog-rebuild input comparison incomplete; see per-table errors",
        ))
    }
}

/// The attache relation sets the second country's status toward the first.
/// Air-base access and its offer set the first country's status toward the
/// second. Military access and docking rights set the first country's status
/// toward the second, while their offers set the second's toward the first.
/// Removing a link clears its status, so
/// each saved relation of those types is one set slot. A relation is saved
/// under both endpoints; the slot is counted once. The co-belligerent byte
/// is rebuilt at load by wartime-ally rebuilding and never
/// saved, so it is excluded here and named as a limitation.
pub(super) fn compare_links(
    frame: &DecodedFrame,
    save: &Save,
    countries: &CountryContext<'_>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let rows = complete::<tables::RelationLinks>(frame)?;
    compare_transport_subject_access(frame, rows, report);
    let spelling = |token: u32| {
        tokens.get(&token).copied().ok_or_else(|| {
            invalid(format!(
                "the recorded lexical dictionary has no spelling for relation type {token:#x}"
            ))
        })
    };
    let attache = spelling(cattache_relation::TYPE)?;
    let air = [
        spelling(cair_base_access_relation::TYPE)?,
        spelling(coffer_air_base_access_relation::TYPE)?,
    ];
    let access = spelling(cmilitary_access_relation::TYPE)?;
    let offered_access = spelling(coffer_military_access_relation::TYPE)?;
    let docking = spelling(cdocking_rights_relation::TYPE)?;
    let offered_docking = spelling(coffer_docking_rights_relation::TYPE)?;
    let mut expected: BTreeMap<(&[u8], &[u8]), u8> = BTreeMap::new();
    for relation in &save.relation_tuples {
        let kind = relation.kind.as_slice();
        let forward = (relation.first.as_slice(), relation.second.as_slice());
        let reverse = (relation.second.as_slice(), relation.first.as_slice());
        let (key, bit) = if kind == attache {
            (reverse, RelationLinkRow::ATTACHE)
        } else if air.contains(&kind) {
            (forward, RelationLinkRow::AIR_BASE_ACCESS)
        } else if kind == access {
            (forward, RelationLinkRow::MILITARY_ACCESS)
        } else if kind == offered_access {
            (reverse, RelationLinkRow::MILITARY_ACCESS)
        } else if kind == docking {
            (forward, RelationLinkRow::DOCKING_RIGHTS)
        } else if kind == offered_docking {
            (reverse, RelationLinkRow::DOCKING_RIGHTS)
        } else {
            continue;
        };
        *expected.entry(key).or_default() |= bit;
    }
    let by_slot: BTreeMap<u32, &[u8]> = countries
        .by_tag
        .iter()
        .map(|(&tag, country)| (country.canonical_slot, tag))
        .collect();
    let mut actual: BTreeMap<(&[u8], &[u8]), u8> = BTreeMap::new();
    for row in rows {
        let country = required_country_tag(row.country, &countries.links)?;
        let target = by_slot.get(&row.target_index).copied().ok_or_else(|| {
            invalid(format!(
                "RelationLinks target slot {} has no captured country",
                row.target_index
            ))
        })?;
        if row.links != 0 {
            *actual.entry((country, target)).or_default() |= row.links;
        }
    }
    let mut compared = 0;
    for (&(country, target), &saved) in &expected {
        let captured = actual.remove(&(country, target)).unwrap_or(0);
        compared += 1;
        if captured != saved {
            report.difference(
                format!(
                    "RelationLinks {} -> {}",
                    String::from_utf8_lossy(country),
                    String::from_utf8_lossy(target)
                ),
                captured,
                saved,
            );
        }
    }
    for ((country, target), links) in actual {
        report.difference(
            format!(
                "RelationLinks {} -> {}",
                String::from_utf8_lossy(country),
                String::from_utf8_lossy(target)
            ),
            links,
            "no saved relation",
        );
    }
    report.compared("RelationLinks", compared);
    Ok(())
}

/// The save serializes relation objects, never the current association
/// or the sparse owner-coverage certificate. Check current suffix admission
/// independently while keeping the saved link-byte comparison unchanged.
fn compare_transport_subject_access(
    frame: &DecodedFrame,
    rows: &[RelationLinkRow],
    report: &mut Report,
) {
    let mut admitted = 0;
    for row in rows {
        let label = format!(
            "RelationLinks {} -> {}.transport_subject",
            row.country, row.target_index
        );
        let Some(packet) = report.recorded(
            &row.transport_subject,
            "RelationLinks.transport_subject",
            &label,
        ) else {
            continue;
        };
        let Some(packet) = packet else {
            report.difference(&label, "capture refused", "current subject status inputs");
            continue;
        };
        report.packet_fresh(&label, packet.sample_hour, frame.hour);
        if packet.complete_owner {
            super::pass_inputs::eq(
                report,
                &format!("{label}.certificate status inputs"),
                (packet.target_raw, packet.subject),
                (None, None),
            );
            super::pass_inputs::eq(
                report,
                &format!("{label}.certificate link flags"),
                row.links,
                0,
            );
        } else if packet.target_raw.is_none() || packet.subject.is_none() {
            report.difference(
                &label,
                "incomplete current status operands",
                "target and subject inputs",
            );
        }
        admitted += 1;
        report.limitation(
            "current subject status pointer/type, ordered endpoint admission and sparse owner coverage are unsaved runtime witnesses; historical Subjects dates are not independent acceptance",
            label,
        );
    }
    if admitted != 0 {
        report.compared(
            "RelationLinks current subject admission only; no saved counterpart",
            admitted,
        );
    }
}

#[cfg(test)]
mod transport_subject_tests {
    use super::*;
    use replay::{
        frame::{Metrics, TableView},
        rows::{Recorded, TransportSubjectAccess},
    };
    fn frame() -> DecodedFrame {
        DecodedFrame {
            branch: 1,
            hour: 7,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|_| {
                TableView::Unavailable {
                    reason: "undemanded".into(),
                }
                .into()
            }),
        }
    }
    #[test]
    fn transport_subject_oracle_distinguishes_runtime_admission_legacy_and_refusal() {
        let certificate = RelationLinkRow {
            country: 11,
            target_index: 1,
            transport_subject: Recorded::Value(Some(TransportSubjectAccess {
                sample_hour: replay::rows::PacketHour::Stamped(7),
                complete_owner: true,
                ..Default::default()
            })),
            ..Default::default()
        };
        let frame = frame();
        let mut report = Report::default();
        compare_transport_subject_access(&frame, std::slice::from_ref(&certificate), &mut report);
        assert!(report.differences.is_empty());
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("unsaved runtime witnesses"))
        );
        assert_eq!(
            report.compared_tables["RelationLinks current subject admission only; no saved counterpart"],
            1
        );
        let mut historical = certificate.clone();
        historical.transport_subject = Recorded::NotRecorded;
        let mut report = Report::default();
        compare_transport_subject_access(&frame, &[historical], &mut report);
        assert!(report.differences.is_empty());
        assert!(
            report
                .limitations
                .contains_key("RelationLinks.transport_subject not recorded")
        );
        let mut refused = certificate;
        refused.transport_subject = Recorded::Value(None);
        let mut report = Report::default();
        compare_transport_subject_access(&frame, &[refused], &mut report);
        assert_eq!(report.differences.len(), 1);
    }
}

/// The access-mode writer's secondary+A8 (=primary+B0) emits a pair only for CHAR > 0.
/// A fresh native save cannot explicitly contain zero/high-bit modes. Keep
/// duplicates and unsupported saved forms visible instead of normalizing them.
pub(super) fn saved_access_modes(pairs: &[(u32, u8)]) -> io::Result<BTreeMap<u32, u8>> {
    let mut expected = BTreeMap::new();
    for &(region, mode) in pairs {
        if (mode as i8) <= 0 {
            return Err(invalid(format!(
                "saved navy access region {region} mode {mode} cannot be emitted by native signed-positive gate"
            )));
        }
        if expected.insert(region, mode).is_some() {
            return Err(invalid(format!(
                "saved navy access has repeated signed-positive region {region}"
            )));
        }
    }
    Ok(expected)
}

/// The access-mode writer saves only signed-positive CHAR access modes. The table retains
/// every nonzero raw byte; omitted high-bit values have no exact save witness.
pub(super) fn compare_naval_access(
    frame: &DecodedFrame,
    save: &Save,
    countries: &CountryContext<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let rows = complete::<tables::NavalRegionAccess>(frame)?;
    let mut expected: BTreeMap<(&[u8], u32), u8> = BTreeMap::new();
    for (tag, country) in report.canonical_countries(&save.countries) {
        for (region, mode) in
            saved_access_modes(country.naval_region_access.as_deref().unwrap_or(&[]))?
        {
            expected.insert((tag.as_slice(), region), mode);
        }
    }
    let mut compared = 0;
    for row in rows {
        let tag = required_country_tag(row.country, &countries.links)?;
        let label = format!(
            "NavalRegionAccess {} region {}",
            String::from_utf8_lossy(tag),
            row.region
        );
        compared += 1;
        if row.mode == 0 {
            return Err(invalid(
                "NavalRegionAccess has a zero row outside its original nonzero projection",
            ));
        }
        match expected.remove(&(tag, row.region)) {
            Some(saved) if saved == row.mode => {}
            Some(saved) => report.difference(label, row.mode, saved),
            None if (row.mode as i8) < 0 => report.limitation(
                "navy access writer omission proves signed nonpositive; exact high-bit raw mode is not saved",
                label),
            None => report.difference(label, row.mode, "no saved mode"),
        }
    }
    for ((tag, region), mode) in expected {
        report.difference(
            format!(
                "NavalRegionAccess {} region {region}",
                String::from_utf8_lossy(tag)
            ),
            "absent",
            mode,
        );
    }
    report.compared("NavalRegionAccess", compared);
    Ok(())
}

/// `AceRangeInputs` against the saved ace rosters (`countries.<tag>.ace`, in
/// roster order) and the saved wings' `ace` references. The ace block carries
/// `alive` but not its assigned wing `ace+0xE8`; the save names that wing
/// only from the wing side, the wing's `+0x2E8` reference, which ace assignment
/// keeps paired with it (assignment, deployment, deletion
/// and disband; marking an ace dead clears neither). The table is read
/// for every country each hour.
pub(super) fn compare_ace_inputs(
    frame: &DecodedFrame,
    save: &Save,
    countries: &CountryContext<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let rows = complete::<tables::AceRangeInputs>(frame)?;
    let mut wing_of = BTreeMap::new();
    for wing in &save.air.wings {
        if let Some(ace) = wing.ace
            && wing_of.insert(ace, wing.id).is_some()
        {
            return Err(invalid(format!(
                "two saved wings name ace {ace}; its assigned wing is ambiguous"
            )));
        }
    }
    let wing_ids: BTreeSet<u64> = save.air.wings.iter().map(|wing| wing.id).collect();
    let mut expected = BTreeMap::new();
    for ace in &save.air.aces {
        let Some(country) = countries.by_tag.get(ace.country.as_slice()) else {
            report.difference(
                format!(
                    "AceRangeInputs {} ace {}",
                    String::from_utf8_lossy(&ace.country),
                    ace.id
                ),
                "no captured country",
                "present",
            );
            continue;
        };
        let slot = country.canonical_slot;
        if expected.insert((slot, ace.id), ace).is_some() {
            return Err(invalid(format!(
                "{} saves ace {} twice",
                String::from_utf8_lossy(&ace.country),
                ace.id
            )));
        }
    }
    let mut compared = 0;
    for row in rows {
        let label = format!("AceRangeInputs slot {} ace {}", row.canonical_slot, row.ace);
        let Some(saved) = expected.remove(&(row.canonical_slot, row.ace)) else {
            report.difference(label, "present", "absent");
            continue;
        };
        compared += 1;
        if row.alive != u8::from(saved.alive) {
            report.difference(format!("{label}.alive"), row.alive, saved.alive);
        }
        match wing_of.get(&row.ace) {
            Some(&wing) if row.wing != wing => {
                report.difference(format!("{label}.wing"), row.wing, wing);
            }
            Some(_) => {}
            None if row.wing == 0 => {}
            None if wing_ids.contains(&row.wing) => report.difference(
                format!("{label}.wing"),
                row.wing,
                "that saved wing does not name this ace",
            ),
            None => report.limitation(
                "no saved wing names this ace, and its wing reference names no saved wing: the ace block does not carry the ace wing reference, so an unassigned reference's bits (the runtime invalid reference the setter stores) are not saved",
                format!("{label}.wing"),
            ),
        }
    }
    for (slot, ace) in expected.keys() {
        report.difference(
            format!("AceRangeInputs slot {slot} ace {ace}"),
            "absent",
            "present",
        );
    }
    report.compared("AceRangeInputs", compared);
    report.limitation(
        "the ace modifier getter results (key 5 on the ace's own and its type's modifiers) are computed from ace types, not saved",
        "AceRangeInputs.range, AceRangeInputs.type_range",
    );
    Ok(())
}

/// `StockpileArchetypes` against the saved stockpile. The table is the first
/// array of each country's `production status+0x200` pool, kept for the
/// archetypes a raid type lists as essential, in whole units capped at the
/// largest amount any raid type requires (`essential_caps`, by archetype
/// name); the save writes only the pool's variant rows (`equipments`,
/// the variant quantity writer). Every quantity reaches both through
/// variant quantity insertion, which adds it to
/// the variant and to its archetype's total (the definition's `+0x4D8`), so
/// each total is the sum of its variants' saved amounts. A variant's
/// archetype is its `VariantDefinitions.archetype_token`. Every country is
/// read each hour.
pub(super) fn compare_stockpile_archetypes(
    frame: &DecodedFrame,
    save: &Save,
    countries: &CountryContext<'_>,
    tokens: &TokenNames<'_>,
    essential_caps: Result<&BTreeMap<Vec<u8>, i32>, String>,
    report: &mut Report,
) -> io::Result<()> {
    /// `variant addition` clamps each running sum to this bound.
    const LIMIT: i128 = 0x53e2_d620_fb40;
    let rows = complete::<tables::StockpileArchetypes>(frame)?;
    let caps = match essential_caps {
        Ok(caps) => caps,
        Err(reason) => {
            report.limitation(
                &format!("the raid types' essential archetypes are not known: {reason}"),
                "StockpileArchetypes",
            );
            report.compared("StockpileArchetypes", 0);
            return Ok(());
        }
    };
    // Each essential archetype's cap, by the token the rows carry.
    let cap_of: BTreeMap<u32, i32> = tokens
        .iter()
        .filter_map(|(&token, &name)| caps.get(name).map(|&cap| (token, cap)))
        .collect();
    let archetype_of: BTreeMap<u64, u32> = complete::<tables::VariantDefinitions>(frame)?
        .iter()
        .map(|row| (row.id, row.archetype_token))
        .collect();
    let saved = save
        .production
        .countries
        .as_ref()
        .ok_or_else(|| invalid("save lacks production country roster"))?;
    let mut expected: BTreeMap<(&[u8], u32), i128> = BTreeMap::new();
    let mut underived: BTreeMap<&[u8], u64> = BTreeMap::new();
    for (tag, country) in report.canonical_countries(saved) {
        let pool = country
            .production
            .as_ref()
            .and_then(|production| production.stockpile.as_ref())
            .ok_or_else(|| {
                invalid(format!(
                    "{} lacks a serialized stockpile",
                    String::from_utf8_lossy(tag)
                ))
            })?;
        for entry in &pool.equipment {
            match archetype_of.get(&entry.variant) {
                Some(archetype) if !cap_of.contains_key(archetype) => {}
                Some(&archetype) => {
                    *expected.entry((tag.as_slice(), archetype)).or_default() +=
                        i128::from(entry.amount.raw);
                }
                None => {
                    underived.entry(tag.as_slice()).or_insert(entry.variant);
                }
            }
        }
    }
    let by_slot: BTreeMap<u32, &[u8]> = countries
        .by_tag
        .iter()
        .map(|(&tag, country)| (country.canonical_slot, tag))
        .collect();
    let mut compared = 0;
    let mut zero_only = 0;
    let mut seen = BTreeSet::new();
    for row in rows {
        let Some(&cap) = cap_of.get(&row.archetype) else {
            report.difference(
                format!(
                    "StockpileArchetypes slot {} archetype {}",
                    row.canonical_slot, row.archetype
                ),
                row.units,
                "no raid type lists this archetype as essential",
            );
            continue;
        };
        let tag = by_slot.get(&row.canonical_slot).copied().ok_or_else(|| {
            invalid(format!(
                "StockpileArchetypes slot {} has no captured country",
                row.canonical_slot
            ))
        })?;
        if !seen.insert((tag, row.archetype)) {
            return Err(invalid("duplicate StockpileArchetypes country archetype"));
        }
        if underived.contains_key(tag) {
            continue;
        }
        let label = format!(
            "StockpileArchetypes {} archetype {}",
            String::from_utf8_lossy(tag),
            row.archetype
        );
        if !saved.contains_key(tag) {
            report.difference(label, row.units, "no saved country");
            continue;
        }
        // Compare the signed integer obtained by truncating total / 100000.
        let units = |sum: i128| ((sum / 100_000) as i64 as i32).min(cap);
        match expected.remove(&(tag, row.archetype)) {
            Some(sum) if sum.abs() > LIMIT => report.limitation(
                "the saved variant amounts of this archetype sum past the bound variant addition clamps each running total to; the clamped total depends on the unsaved order of additions",
                label,
            ),
            Some(sum) => {
                compared += 1;
                if row.units != units(sum) {
                    report.difference(label, row.units, units(sum));
                }
            }
            None if row.units != 0 => {
                compared += 1;
                report.difference(label, row.units, "no saved variant of this archetype");
            }
            None => zero_only += 1,
        }
    }
    for ((tag, archetype), sum) in expected {
        if !underived.contains_key(tag) && countries.by_tag.contains_key(tag) {
            report.difference(
                format!(
                    "StockpileArchetypes {} archetype {archetype}",
                    String::from_utf8_lossy(tag)
                ),
                "absent",
                sum,
            );
        }
    }
    for (tag, variant) in underived {
        report.limitation(
            "a saved stockpile variant has no VariantDefinitions row (a variant created after the table's last daily read), so its archetype and this country's archetype totals are not derived",
            format!(
                "StockpileArchetypes {}: variant {variant}",
                String::from_utf8_lossy(tag)
            ),
        );
    }
    if zero_only != 0 {
        report.limitation(
            "a zero archetype total with no saved variant is kept by variant addition, which never removes an entry, while the saved pool may omit its zero variants (allow_zero_entries=no); its membership is not certified",
            format!("StockpileArchetypes: {zero_only} zero totals"),
        );
    }
    report.compared("StockpileArchetypes", compared);
    Ok(())
}

/// The game state save writer writes the saving client's `player` tag
/// using the signed-positive local tag, else the fallback tag, the
/// choice `LocalPlayerRow::viewing_country` makes. Only that choice is saved,
/// so the other reference decodes uncertified.
fn compare_local_player(
    frame: &DecodedFrame,
    save: &Save,
    countries: &CountryContext<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let [row] = complete::<tables::LocalPlayer>(frame)? else {
        return Err(invalid("LocalPlayer must contain exactly one row"));
    };
    let words = format!(
        "LocalPlayer: country={}, observer_country={}",
        row.country, row.observer_country
    );
    let Some(raw) = row.viewing_country() else {
        report.compared("LocalPlayer", 0);
        report.limitation(
            "neither client reference is positive; the save names the country database's tag for it, which the recorded links do not resolve",
            words,
        );
        return Ok(());
    };
    let tag = required_country_tag(raw, &countries.links)?;
    if save.player.as_deref() != Some(tag) {
        report.difference(
            "LocalPlayer viewing country",
            super::tag_text(Some(tag)),
            super::tag_text(save.player.as_deref()),
        );
    }
    report.compared("LocalPlayer", 1);
    report.limitation(
        "the save holds only the client's viewing choice; the unchosen reference has no save representation",
        words,
    );
    Ok(())
}

/// Inputs the text save does not carry: they decode, but nothing certifies them.
fn record_inputs(frame: &DecodedFrame, report: &mut Report) -> io::Result<()> {
    for row in complete::<tables::SimulationFeatures>(frame)? {
        if let Some(instant) = report.recorded(
            &row.instant_decisions,
            "SimulationFeatures.instant_decisions",
            "instant decisions console byte",
        ) {
            match instant {
                Some(_) => report.limitation("instant_decisions is the session console byte; no established text-save field certifies it", "SimulationFeatures.instant_decisions"),
                None => report.limitation("instant decisions console byte capture refused", "SimulationFeatures.instant_decisions: other simulation features remain available"),
            }
        }
    }
    report.limitation(
        "the weather and day-night switches gate the hourly weather/day-night update; the manager serializer does not write these bytes or the held local-hour/night outputs, so a text save cannot certify suspension or recover their held history",
        "SimulationFeatures.weather_suspend: runtime-only census, no exact text-save comparison",
    );
    for (table, rows, why) in [
        (
            "SimulationFeatures",
            complete::<tables::SimulationFeatures>(frame)?.len(),
            "the DLC feature mask, human_ai byte and weather-suspension bytes are running-session inputs with no established text-save join",
        ),
        (
            "CountryVisibility",
            complete::<tables::CountryVisibility>(frame)?.len(),
            "the viewer modifier 0x24B is a computed country modifier the save does not write",
        ),
        (
            "StateVisibility",
            complete::<tables::StateVisibility>(frame)?.len(),
            "the state modifier 0x4B is a computed state modifier the save does not write",
        ),
        (
            "RaidCountryInputs",
            complete::<tables::RaidCountryInputs>(frame)?.len(),
            "modifier getter results, autonomous flags and convoy reservations are runtime state the save does not write; the rows are sliced and keep their own sample hours",
        ),
        (
            "CountryCommandPower",
            complete::<tables::CountryCommandPower>(frame)?.len(),
            "allocated command power and the getter results of max_command_power, max_command_power_mult, ace_effectiveness_factor and air_ace_bonuses_factor (runtime-only country modifier totals) are runtime state the save does not write",
        ),
        (
            "RaidAirSites",
            complete::<tables::RaidAirSites>(frame)?.len(),
            "the country air-site lists and container capacities are rebuilt at load, not saved; the rows are sliced and keep their own sample hours",
        ),
        (
            "BorderConflicts",
            complete::<tables::BorderConflicts>(frame)?.len(),
            "each province's border-conflict combat set and the orders groups' border-war operands are rebuilt at load, not saved per province",
        ),
        (
            "CountryWars",
            complete::<tables::CountryWars>(frame)?.len(),
            "the CAN_OCCUPY_NON_WAR rule byte is recomputed from rule sources and the directly read enemy vectors are rebuilt at load; neither is saved",
        ),
        (
            "CountryLocalSubunits",
            complete::<tables::CountryLocalSubunits>(frame)?.len(),
            "country-local sub-unit clones are rebuilt from research on load and their requirement vectors are not saved",
        ),
        (
            "NewDivisionEquipment",
            complete::<tables::NewDivisionEquipment>(frame)?.len(),
            "it holds only divisions minted since the daily DivisionEquipment walk, which a full hour's walk covers, so a load hour leaves it empty",
        ),
    ] {
        report.compared(table, 0);
        report.limitation(
            &format!("{why}, so this table has no save comparison; the recorded rows decode but are not certified against a save"),
            format!("{table}: {rows} decoded rows, zero save comparisons"),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::{
        frame::TableView,
        rows::{LocalPlayerRow, TableId, TableRows},
    };

    /// The save names the tag of the playing reference when positive, else of
    /// the observer's; an alias reference resolves through the links.
    #[test]
    fn the_saved_player_tag_certifies_the_clients_viewing_choice() {
        let (mut frame, mut save, _, _) = super::super::tests::fixture();
        let run = |frame: &DecodedFrame, save: &Save, row: [u32; 2]| {
            let [country, observer_country] = row;
            let mut frame = frame.clone();
            *frame.tables[TableId::LocalPlayer.index()] =
                TableView::Complete(TableRows::LocalPlayer(vec![LocalPlayerRow {
                    country,
                    observer_country,
                }]));
            let countries = super::super::country_context(&frame).unwrap();
            let mut report = Report::default();
            compare_local_player(&frame, save, &countries, &mut report).map(|()| report)
        };
        for row in [[101, 0], [102, 0], [0, 101], [u32::MAX, 102]] {
            let report = run(&frame, &save, row).unwrap();
            assert!(report.differences.is_empty(), "{row:?}");
            assert_eq!(report.compared_tables["LocalPlayer"], 1);
        }
        let report = run(&frame, &save, [0, 0]).unwrap();
        assert!(report.differences.is_empty());
        assert_eq!(report.compared_tables["LocalPlayer"], 0);
        assert!(run(&frame, &save, [0, 999]).is_err());
        save.player = Some(b"ITA".to_vec());
        assert_eq!(run(&frame, &save, [101, 0]).unwrap().differences.len(), 1);
        save.player = None;
        assert_eq!(run(&frame, &save, [101, 0]).unwrap().differences.len(), 1);
        for table in [
            TableView::Unavailable {
                reason: "read failed".into(),
            },
            TableView::Complete(TableRows::LocalPlayer(Vec::new())),
        ] {
            *frame.tables[TableId::LocalPlayer.index()] = table;
            let context = super::super::country_context(&frame).unwrap();
            assert!(compare_local_player(&frame, &save, &context, &mut Report::default()).is_err());
        }
    }
}
