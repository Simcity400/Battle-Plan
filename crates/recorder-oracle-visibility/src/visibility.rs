
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use engine_native_parts::{
    cair_base_access_relation,
    cattache_relation,
    cdocking_rights_relation,
    cmilitary_access_relation,
    coffer_air_base_access_relation,
    coffer_docking_rights_relation,
    coffer_military_access_relation,
};
use content_reader_save::Save;
use replay_data::{rows::RelationLinkRow, rows::tables};
use replay_frame::frame::DecodedFrame;

use super::{CountryContext, Report, TokenNames, complete, invalid, required_country_tag};

pub fn compare(
    frame: &DecodedFrame,
    save: &Save,
    countries: &CountryContext<'_>,
    tokens: &TokenNames<'_>,
    essential_caps: Result<&BTreeMap<Vec<u8>, i32>, String>,
    report: &mut Report,
) -> io::Result<()> {
    let mut complete_comparison = report
        .attempt_table(replay_data::rows::TableId::LocalPlayer, |report| {
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

pub fn compare_links(
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
    let air = spelling(cair_base_access_relation::TYPE)?;
    let offered_air = spelling(coffer_air_base_access_relation::TYPE)?;
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
        } else if kind == air {
            (forward, RelationLinkRow::AIR_BASE_ACCESS)
        } else if kind == offered_air {
            (reverse, RelationLinkRow::AIR_BASE_ACCESS)
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
            super::eq(
                report,
                &format!("{label}.certificate status inputs"),
                (packet.target_raw, packet.subject),
                (None, None),
            );
            super::eq(
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
    use replay_data::{rows::Recorded, rows::TransportSubjectAccess};
    use replay_frame::{frame::Metrics, frame::TableView};
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
                sample_hour: replay_data::rows::PacketHour::Host,
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

pub fn saved_access_modes(pairs: &[(u32, u8)]) -> io::Result<BTreeMap<u32, u8>> {
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

pub fn compare_naval_access(
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

pub fn compare_ace_inputs(
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

const AMOUNT_LIMIT: i128 = 0x53e2_d620_fb40;

const RUNTIME_TOTAL: &str = "away from a load an archetype total is its own running sum, clamped at the bound apart from its variants' amounts, and the save keeps only the variants: they do not certify a total that may have reached the bound since the load";

const ORDERED_TOTAL: &str = "a load inserting these saved variant amounts could clamp the archetype total at the bound in more than one way, depending on the order of insertion";

#[derive(Clone, Copy)]
struct LoadedAmount {
    last: i64,
    high: i64,
    low: i64,
}

#[derive(Default)]
struct LoadedVariants {
    variants: BTreeMap<u64, LoadedAmount>,
    repeated: bool,
}

impl LoadedVariants {
    fn insert(&mut self, variant: u64, raw: i64) {
        let (high, low) = match self.variants.get(&variant) {
            Some(amount) => {
                self.repeated = true;
                (amount.high, amount.low)
            }
            None => (0, 0),
        };
        self.variants.insert(
            variant,
            LoadedAmount {
                last: raw,
                high: high.max(raw),
                low: low.min(raw),
            },
        );
    }

    fn total(&self) -> Option<i128> {
        let (mut sum, mut high, mut low) = (0_i128, 0_i128, 0_i128);
        for amount in self.variants.values() {
            sum += i128::from(amount.last);
            high += i128::from(amount.high);
            low += i128::from(amount.low);
        }
        if high <= AMOUNT_LIMIT && low >= -AMOUNT_LIMIT {
            Some(sum)
        } else if self.repeated {
            None
        } else if low == 0 {
            Some(sum.min(AMOUNT_LIMIT))
        } else if high == 0 {
            Some(sum.max(-AMOUNT_LIMIT))
        } else {
            None
        }
    }
}

pub fn compare_stockpile_archetypes(
    frame: &DecodedFrame,
    save: &Save,
    countries: &CountryContext<'_>,
    tokens: &TokenNames<'_>,
    essential_caps: Result<&BTreeMap<Vec<u8>, i32>, String>,
    report: &mut Report,
) -> io::Result<()> {
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
    let mut expected: BTreeMap<(&[u8], u32), LoadedVariants> = BTreeMap::new();
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
                    expected
                        .entry((tag.as_slice(), archetype))
                        .or_default()
                        .insert(entry.variant, entry.amount.raw);
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
    let certified = report.load_hour;
    let whole = (AMOUNT_LIMIT / 100_000) as i32;
    let mut compared = 0;
    let mut zero_only = 0;
    let mut runtime = 0;
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
        let label = format!(
            "StockpileArchetypes {} archetype {}",
            String::from_utf8_lossy(tag),
            row.archetype
        );
        let loaded = expected.remove(&(tag, row.archetype));
        let (low, high) = ((-whole).min(cap), whole.min(cap));
        if row.units < low || row.units > high {
            compared += 1;
            report.difference(
                label,
                row.units,
                format!("whole units from {low} to {high}"),
            );
            continue;
        }
        if underived.contains_key(tag) {
            continue;
        }
        if !saved.contains_key(tag) {
            report.difference(label, row.units, "no saved country");
            continue;
        }
        let units = |total: i128| ((total / 100_000) as i32).min(cap);
        let total = loaded.as_ref().map(LoadedVariants::total);
        if !certified {
            match total {
                Some(Some(total)) if units(total) != row.units => report.limitation(
                    RUNTIME_TOTAL,
                    format!(
                        "{label}: replay {}, a load of the saved variants {}",
                        row.units,
                        units(total)
                    ),
                ),
                _ => runtime += 1,
            }
            continue;
        }
        match total {
            Some(None) => report.limitation(ORDERED_TOTAL, label),
            Some(Some(total)) => {
                compared += 1;
                if row.units != units(total) {
                    report.difference(label, row.units, units(total));
                }
            }
            None if row.units != 0 => {
                compared += 1;
                report.difference(label, row.units, "no saved variant of this archetype");
            }
            None => zero_only += 1,
        }
    }
    for ((tag, archetype), loaded) in expected {
        if !underived.contains_key(tag) && countries.by_tag.contains_key(tag) {
            report.difference(
                format!(
                    "StockpileArchetypes {} archetype {archetype}",
                    String::from_utf8_lossy(tag)
                ),
                "absent",
                format!("{} saved variants", loaded.variants.len()),
            );
        }
    }
    if runtime != 0 {
        report.limitation(
            RUNTIME_TOTAL,
            format!("StockpileArchetypes: {runtime} totals"),
        );
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

pub fn compare_volunteer_stockpile(
    frame: &DecodedFrame,
    save: &Save,
    countries: &CountryContext<'_>,
    is_plane: &dyn Fn(u32) -> io::Result<bool>,
    report: &mut Report,
) -> io::Result<()> {
    let rows = complete::<tables::RaidCountryInputs>(frame)?;
    for row in rows {
        row.volunteer_stockpile_planes
            .require("volunteer stockpile aircraft")
            .map_err(invalid)?;
    }
    if !rows.iter().any(|row| {
        matches!(
            row.volunteer_stockpile_planes,
            replay_data::rows::Recorded::Value(Some(_))
        )
    }) {
        report.compared("RaidCountryInputs.volunteer_stockpile_planes", 0);
        return Ok(());
    }
    let variants: BTreeMap<_, _> = complete::<tables::VariantDefinitions>(frame)?
        .iter()
        .map(|row| (row.id, row.archetype_token))
        .collect();
    let tags: BTreeMap<_, _> = countries
        .by_tag
        .iter()
        .map(|(&tag, country)| (country.canonical_slot, tag))
        .collect();
    let saved = save
        .production
        .countries
        .as_ref()
        .ok_or_else(|| invalid("save lacks production countries"))?;
    let certified = report.load_hour;
    let mut compared = 0;
    let mut runtime = 0;
    for row in rows {
        let Some(actual) = *row
            .volunteer_stockpile_planes
            .require("volunteer stockpile aircraft")
            .map_err(invalid)?
        else {
            continue;
        };
        let tag = *tags
            .get(&row.canonical_slot)
            .ok_or_else(|| invalid("volunteer stockpile country is unresolved"))?;
        let stockpile = saved
            .get(tag)
            .and_then(|country| country.production.as_ref())
            .and_then(|production| production.stockpile.as_ref())
            .ok_or_else(|| invalid("volunteer sender has no serialized stockpile"))?;
        let mut archetypes = BTreeMap::<u32, LoadedVariants>::new();
        for entry in &stockpile.equipment {
            let token = *variants
                .get(&entry.variant)
                .ok_or_else(|| invalid("volunteer stockpile variant definition is missing"))?;
            archetypes
                .entry(token)
                .or_default()
                .insert(entry.variant, entry.amount.raw);
        }
        let mut expected = Some(0_i32);
        for (&token, loaded) in &archetypes {
            if is_plane(token)? {
                expected = expected
                    .zip(loaded.total())
                    .map(|(expected, total)| expected.wrapping_add((total / 100_000) as i32));
            }
        }
        let label = format!(
            "RaidCountryInputs {} volunteer_stockpile_planes",
            String::from_utf8_lossy(tag)
        );
        match expected {
            Some(expected) if !certified && expected != actual => report.limitation(
                RUNTIME_TOTAL,
                format!("{label}: replay {actual}, a load of the saved variants {expected}"),
            ),
            _ if !certified => runtime += 1,
            None => report.limitation(ORDERED_TOTAL, label),
            Some(expected) => {
                compared += 1;
                if actual != expected {
                    report.difference(label, actual, expected);
                }
            }
        }
    }
    if runtime != 0 {
        report.limitation(
            RUNTIME_TOTAL,
            format!("RaidCountryInputs.volunteer_stockpile_planes: {runtime} totals"),
        );
    }
    report.compared("RaidCountryInputs.volunteer_stockpile_planes", compared);
    Ok(())
}

fn record_inputs(frame: &DecodedFrame, report: &mut Report) -> io::Result<()> {
    for row in complete::<tables::SimulationFeatures>(frame)? {
        if let Some(extent) = report.recorded(
            &row.state_array_len,
            "SimulationFeatures.state_array_len",
            "global state-array extent",
        ) {
            report.limitation(
                if extent.is_some() {
                    "text saves omit the physical state-array extent and its null slots; membership cannot certify global.states length"
                } else {
                    "the native state-array header was unavailable; global.states remains refused"
                },
                "SimulationFeatures.state_array_len",
            );
        }
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
    use replay_data::{rows::LocalPlayerRow, rows::TableId, rows::TableRows};
    use replay_frame::frame::TableView;

    #[test]
    fn the_saved_player_tag_certifies_the_clients_viewing_choice() {
        let (mut frame, mut save, _, _) = super::super::fixtures::fixture();
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
