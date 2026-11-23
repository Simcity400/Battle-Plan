
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};
mod market;

use content_reader_save::{Save, economy};
use replay_data::{rows::CountryResourceRow, rows::CountryRow, rows::tables};
use replay_frame::frame::DecodedFrame;

use super::{Report, TokenNames, compare_text, complete, equal, invalid, required_country_tag};

pub fn compare(
    frame: &DecodedFrame,
    saved: &Save,
    links: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let market_complete = market::compare(frame, saved, links, tokens, report).is_ok();
    let mut all_complete = market_complete
        & report.attempt("economy CountryFuel", |report| {
            compare_fuel(frame, &saved.economy, links, report)?;
            super::fuel_demand::compare(frame, saved, links, report)
        });
    all_complete &= report.attempt("economy CountryFuelConsumers", |report| {
        compare_fuel_consumers(frame, &saved.economy, links, report)
    });
    all_complete &= report.attempt("economy CountryExperience", |report| {
        compare_experience(frame, &saved.economy, links, report)
    });
    all_complete &= report.attempt("economy CountryExperienceGains", |report| {
        let rows = complete::<tables::CountryExperienceGains>(frame)?;
        let mut countries = std::collections::BTreeSet::new();
        for row in rows {
            let country = links.get(&row.country).ok_or_else(|| invalid("experience gain country is unresolved"))?;
            if !countries.insert(country.canonical_slot) {
                return Err(invalid("experience gain country has more than one native instance"));
            }
        }
        let expected = complete::<tables::Countries>(frame)?.iter().map(|row| row.canonical_slot)
            .collect::<std::collections::BTreeSet<_>>();
        equal(report, "CountryExperienceGains country census", countries, expected);
        report.compared("CountryExperienceGains", 0);
        report.limitation("accepted experience awards and native country-instance words are not serialized; this checks recording census/ownership only, not gain agreement. Daily counters may clamp or reset and bank deltas include spending/caps, so neither is substituted as a gain oracle", format!("CountryExperienceGains: {} rows, zero save comparisons", rows.len()));
        Ok(())
    });
    all_complete &= report.attempt("economy PurchaseContracts", |report| {
        compare_purchases(frame, &saved.economy, links, report)
    });
    all_complete &= report.attempt("economy CountryResources", |report| {
        compare_resources(frame, &saved.economy, links, tokens, report)
    });
    all_complete &= report.attempt("economy ResourceExchanges", |report| {
        compare_exchanges(frame, &saved.economy, links, tokens, report)
    });
    all_complete &= report.attempt("economy LogisticsEquipment", |report| {
        compare_logistics(frame, &saved.economy, links, tokens, report)
    });
    let owners = state_owners(saved);
    all_complete &= report.attempt("economy HubMotorization", |report| {
        compare_motorization(frame, &saved.economy, &owners, links, report)
    });
    all_complete &= report.attempt("economy DisruptedSupply", |report| {
        compare_disrupted(frame, &saved.economy, &owners, links, report)
    });
    all_complete &= report.attempt("economy ForeignHomebases", |report| {
        compare_foreign(frame, &saved.economy, &owners, links, report)
    });
    for (name, rows, reason) in [
        (
            "ProvinceSupply",
            complete::<tables::ProvinceSupply>(frame).map(<[_]>::len),
            "province supply rows are runtime simulation caches; the save has no established direct representation of controller and capital-country indices, supply-node and naval-base levels or flags; authored map levels and UI supply are not substituted",
        ),
        (
            "ResourceDefinitions",
            complete::<tables::ResourceDefinitions>(frame).map(<[_]>::len),
            "resource definitions are a native content database; the save does not directly certify numeric resource/token IDs, active flags or factory-cost operands",
        ),
    ] {
        all_complete &= report.attempt(&format!("economy {name}"), |report| {
            let rows = rows?;
            report.compared(name, 0);
            report.limitation(
                reason,
                format!("{name}: {rows} decoded rows, zero save comparisons"),
            );
            Ok(())
        });
    }
    report.limitation("economy save country tags compare resolved identity and cannot certify equivalent raw country-link aliases", "CountryFuel, CountryFuelConsumers, CountryExperience, LogisticsEquipment, ResourceExchanges, PurchaseContracts, HubMotorization, DisruptedSupply, ForeignHomebases");
    if all_complete {
        Ok(())
    } else {
        Err(invalid("one or more economy table comparisons failed"))
    }
}

fn compare_experience(
    frame: &DecodedFrame,
    saved: &economy::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut actual = BTreeMap::new();
    for row in complete::<tables::CountryExperience>(frame)? {
        let tag = required_country_tag(row.country, links)?;
        if actual.insert(tag, row).is_some() {
            return Err(invalid("duplicate resolved country in experience table"));
        }
    }
    let mut count = 0;
    for (tag, country) in report.canonical_countries(&saved.countries) {
        let label = format!("{}.experience", String::from_utf8_lossy(tag));
        let Some(actual) = actual.remove(tag.as_slice()) else {
            report.difference(label, "absent", "saved country present");
            continue;
        };
        let Some(expected) = &country.experience else {
            report.difference(label, "present", "experience_status absent");
            continue;
        };
        count += 1;
        for (name, actual, expected) in [
            ("army_raw", actual.army_raw, expected.army_raw),
            (
                "army_daily_raw",
                actual.army_daily_raw,
                expected.army_daily_raw,
            ),
            (
                "army_daily_training_raw",
                actual.army_daily_training_raw,
                expected.army_daily_training_raw,
            ),
            ("navy_raw", actual.navy_raw, expected.navy_raw),
            (
                "navy_daily_raw",
                actual.navy_daily_raw,
                expected.navy_daily_raw,
            ),
            (
                "navy_daily_training_raw",
                actual.navy_daily_training_raw,
                expected.navy_daily_training_raw,
            ),
            ("air_raw", actual.air_raw, expected.air_raw),
            (
                "air_daily_raw",
                actual.air_daily_raw,
                expected.air_daily_raw,
            ),
        ] {
            equal(
                report,
                format!("{label}.{name}"),
                actual,
                expected.unwrap_or(0),
            );
        }
        equal(
            report,
            format!("{label}.armies_for_training_raw"),
            actual.armies_for_training_raw,
            expected.armies_for_training_raw,
        );
    }
    for tag in actual.keys() {
        report.difference(
            format!("{}.experience", String::from_utf8_lossy(tag)),
            "present",
            "country absent",
        );
    }
    report.compared("CountryExperience", count);
    report.limitation("all nine experience scalars compare exact signed /32768 values; native i64::MIN has malformed text serialization and is refused; num_armies_for_training is the fractional training divisor, not an integer count of exercising units", "CountryExperience");
    Ok(())
}

type ResourceField = (
    &'static str,
    fn(&CountryResourceRow) -> i64,
    fn(&economy::ResourcePools) -> &economy::ResourcePool,
);

const RESOURCE_FIELDS: [ResourceField; 9] = [
    ("produced", |row| row.produced, |pools| &pools.produced),
    (
        "adjustment",
        |row| row.adjustment,
        |pools| &pools.transfer_overlord_subject,
    ),
    ("imported", |row| row.imported, |pools| &pools.imported),
    ("used", |row| row.used, |pools| &pools.to_use[0]),
    (
        "special_total - used",
        |row| row.special_total.wrapping_sub(row.used),
        |pools| &pools.to_use[1],
    ),
    (
        "production_demand",
        |row| row.production_demand,
        |pools| &pools.to_use[2],
    ),
    ("to_export", |row| row.to_export, |pools| &pools.to_export),
    (
        "base_export",
        |row| row.base_export,
        |pools| &pools.base_export,
    ),
    ("exported", |row| row.exported, |pools| &pools.exported),
];

const AMBIGUOUS_RESOURCE_JOIN: &str = "a resource name declared more than once (loader appends each declaration) repeats in the saved pools, and the pool writer omits zero amounts, so a pool with fewer occurrences than declarations cannot attribute its amounts to declarations; its nonzero amounts are compared in index order instead";

fn compare_resources(
    frame: &DecodedFrame,
    saved: &economy::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let mut definitions = BTreeMap::new();
    for row in complete::<tables::ResourceDefinitions>(frame)? {
        if definitions.insert(row.id, row.token).is_some() {
            return Err(invalid("duplicate resource definition id"));
        }
    }
    let mut spelled = BTreeMap::<&[u8], Vec<u32>>::new();
    let mut unspelled = BTreeMap::new();
    for (&id, &token) in &definitions {
        match tokens.get(&token) {
            Some(name) => spelled.entry(*name).or_default().push(id),
            None => {
                unspelled.insert(id, token);
            }
        }
    }
    let label = |tag: &[u8], id: u32| {
        let tag = String::from_utf8_lossy(tag);
        match tokens.get(&definitions[&id]) {
            Some(name) if spelled[name].len() > 1 => {
                format!("{tag}.resources[{} id={id}]", String::from_utf8_lossy(name))
            }
            Some(name) => format!("{tag}.resources[{}]", String::from_utf8_lossy(name)),
            None => format!("{tag}.resources token={}", definitions[&id]),
        }
    };
    let mut rows = BTreeMap::new();
    for row in complete::<tables::CountryResources>(frame)? {
        let tag = required_country_tag(row.country, links)?;
        if !definitions.contains_key(&row.resource) {
            return Err(invalid(format!(
                "country resource row {} references absent resource definition {}",
                String::from_utf8_lossy(tag),
                row.resource
            )));
        }
        if rows.insert((tag, row.resource), row).is_some() {
            return Err(invalid(format!(
                "two country resource rows resolve to {} and resource definition {}",
                String::from_utf8_lossy(tag),
                row.resource
            )));
        }
    }
    let countries = links
        .values()
        .map(|country| country.tag.as_slice())
        .collect::<std::collections::BTreeSet<_>>();
    for &tag in &countries {
        for &id in definitions.keys() {
            if !rows.contains_key(&(tag, id)) {
                report.difference(
                    label(tag, id),
                    "absent",
                    "one row per captured country and resource definition",
                );
            }
        }
    }
    let mut count = 0;
    for &tag in &countries {
        let pools = saved
            .countries
            .get(tag)
            .and_then(|country| country.pools.as_ref());
        for (&id, token) in &unspelled {
            if rows.contains_key(&(tag, id)) {
                report.limitation(
                    "resource name comparison lacks the captured lexical token spelling",
                    format!("{}.resources token={token}", String::from_utf8_lossy(tag)),
                );
            }
        }
        for (&name, ids) in &spelled {
            let Some(group) = ids
                .iter()
                .map(|&id| rows.get(&(tag, id)).copied())
                .collect::<Option<Vec<_>>>()
            else {
                continue;
            };
            let Some(pools) = pools else {
                for &id in ids {
                    report.difference(label(tag, id), "present", "save resources object absent");
                }
                continue;
            };
            count += ids.len();
            compare_resource_name(report, tag, name, ids, &group, pools, &label);
        }
    }
    for (tag, country) in report.canonical_countries(&saved.countries) {
        let Some(pools) = &country.pools else {
            continue;
        };
        let captured = countries.contains(tag.as_slice());
        let mut unmatched = std::collections::BTreeSet::new();
        for (_, _, pool) in RESOURCE_FIELDS {
            for (name, _) in pool(pools) {
                if !captured || !spelled.contains_key(name.as_slice()) {
                    unmatched.insert(name.as_slice());
                }
            }
        }
        for name in unmatched {
            let subject = format!(
                "{}.resources[{}]",
                String::from_utf8_lossy(tag),
                String::from_utf8_lossy(name)
            );
            if captured && !unspelled.is_empty() {
                report.limitation(
                    "saved resource name matches no spelled resource definition while some definitions lack the captured lexical token spelling",
                    subject,
                );
            } else {
                report.difference(subject, "absent", "present");
            }
        }
    }
    report.compared("CountryResources", count);
    Ok(())
}

fn compare_resource_name(
    report: &mut Report,
    tag: &[u8],
    name: &[u8],
    ids: &[u32],
    rows: &[&CountryResourceRow],
    pools: &economy::ResourcePools,
    label: &impl Fn(&[u8], u32) -> String,
) {
    let mut ambiguous = Vec::new();
    for (field, value, pool) in RESOURCE_FIELDS {
        let saved = pool(pools)
            .iter()
            .filter(|(key, _)| key.as_slice() == name)
            .map(|(_, amount)| amount.raw)
            .collect::<Vec<_>>();
        let recorded = rows.iter().map(|row| value(row)).collect::<Vec<_>>();
        if saved.len() == ids.len() {
            for ((&id, recorded), saved) in ids.iter().zip(recorded).zip(saved) {
                equal(
                    report,
                    format!("{}.{field}", label(tag, id)),
                    recorded,
                    saved,
                );
            }
        } else if let ([id], []) = (ids, saved.as_slice()) {
            equal(
                report,
                format!("{}.{field}", label(tag, *id)),
                recorded[0],
                0,
            );
        } else {
            let group = format!(
                "{}.resources[{}].{field}",
                String::from_utf8_lossy(tag),
                String::from_utf8_lossy(name)
            );
            if saved.len() > ids.len() {
                report.difference(
                    format!("{group} ({} definitions)", ids.len()),
                    recorded,
                    format!("{} saved occurrences {saved:?}", saved.len()),
                );
                continue;
            }
            ambiguous.push(format!("{field} {}/{}", saved.len(), ids.len()));
            let nonzero = recorded
                .into_iter()
                .filter(|&amount| amount != 0)
                .collect::<Vec<_>>();
            equal(
                report,
                format!("{group} nonzero in index order"),
                nonzero,
                saved,
            );
        }
    }
    if !ambiguous.is_empty() {
        report.limitation(
            AMBIGUOUS_RESOURCE_JOIN,
            format!(
                "{}.resources[{}] ids {ids:?}: {} (saved occurrences/definitions)",
                String::from_utf8_lossy(tag),
                String::from_utf8_lossy(name),
                ambiguous.join(", ")
            ),
        );
    }
}

fn compare_exchanges(
    frame: &DecodedFrame,
    saved: &economy::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let mut resources = BTreeMap::new();
    for row in complete::<tables::ResourceDefinitions>(frame)? {
        if resources.insert(row.id, row.token).is_some() {
            return Err(invalid("duplicate resource definition id"));
        }
    }
    let mut actual = BTreeMap::new();
    for row in complete::<tables::ResourceExchanges>(frame)? {
        if row.id == 0 || actual.insert(row.id, row).is_some() {
            return Err(invalid("null or duplicate full resource exchange handle"));
        }
    }
    let mut count = 0;
    let mut saved_ids = std::collections::BTreeSet::new();
    for (tag, country) in report.canonical_countries(&saved.countries) {
        let Some(exports) = &country.exports else {
            report.difference(
                format!("{}.resources", String::from_utf8_lossy(tag)),
                "captured resource census",
                "save resources object absent",
            );
            continue;
        };
        for expected in exports {
            if !saved_ids.insert(expected.id) {
                return Err(invalid(
                    "duplicate full resource exchange handle across saved countries",
                ));
            }
            let label = format!(
                "resource exchange type={} id={}",
                expected.id as u32,
                expected.id >> 32
            );
            let Some(actual) = actual.remove(&expected.id) else {
                report.difference(label, "absent", "present");
                continue;
            };
            count += 1;
            equal(
                report,
                format!("{label}.exporter"),
                required_country_tag(actual.exporter, links)?,
                tag.as_slice(),
            );
            equal(
                report,
                format!("{label}.importer"),
                Some(required_country_tag(actual.importer, links)?),
                expected.receiver.as_deref(),
            );
            let resource_token = resources
                .get(&actual.resource)
                .copied()
                .ok_or_else(|| invalid("exchange references absent resource definition"))?;
            if let Some(expected) = expected.resource.as_deref() {
                if let Some(resource) = tokens.get(&resource_token) {
                    compare_text(report, &format!("{label}.resource"), resource, expected);
                } else {
                    report.limitation(
                        "resource name comparison lacks the captured lexical token spelling",
                        format!("{label}.resource token={resource_token}"),
                    );
                }
            } else {
                report.difference(format!("{label}.resource"), "present", "absent");
            }
            for (name, actual, expected) in [
                ("efficiency_a", actual.efficiency_a, expected.efficiency.raw),
                (
                    "efficiency_b",
                    actual.efficiency_b,
                    expected.efficiency_due_to_lost_convoys.raw,
                ),
                ("delivered", actual.delivered, expected.delivered.raw),
            ] {
                equal(report, format!("{label}.{name}"), actual, expected);
            }
            equal(
                report,
                format!("{label}.required_factories"),
                actual.required_factories,
                expected.required_cic,
            );
            equal(
                report,
                format!("{label}.assigned_factories"),
                actual.assigned_factories,
                expected.lended_cic,
            );
            for (name, actual, expected) in [
                (
                    "convoys_assigned",
                    actual.convoys_assigned,
                    expected.convoys.assigned,
                ),
                (
                    "convoys_required",
                    actual.convoys_required,
                    expected.convoys.required,
                ),
            ] {
                equal(report, format!("{label}.{name}"), actual, expected);
            }
        }
    }
    for id in actual.keys() {
        report.difference(format!("resource exchange {id:#018x}"), "present", "absent");
    }
    report.compared("ResourceExchanges", count);
    report.limitation("resource exchange no_trade_influence/no_resources bytes and directional native vector ordinals are not directly serialized", "ResourceExchanges");
    report.limitation("save resource names and country tags cannot certify equivalent raw numeric token/resource identities or country-link alias bits", "ResourceExchanges");
    Ok(())
}

const STATELESS_SUPPLY: &str = "the supply writer saves a country's supply block only while the country owns a state, so the live supply rows of a country that owns none have no save counterpart";

fn state_owners(saved: &Save) -> BTreeSet<&[u8]> {
    saved
        .states
        .values()
        .filter_map(|state| state.owner.as_deref())
        .collect()
}

fn unsaved_supply(
    systems: &BTreeMap<Vec<u8>, economy::Supply>,
    owners: &BTreeSet<&[u8]>,
    tag: &[u8],
) -> bool {
    !systems.contains_key(tag) && !owners.contains(tag)
}

fn compare_motorization(
    frame: &DecodedFrame,
    saved: &economy::Pack,
    owners: &BTreeSet<&[u8]>,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let motorization = complete::<tables::HubMotorization>(frame)?;
    let Some(systems) = &saved.supply else {
        report.difference(
            "HubMotorization",
            motorization.len(),
            "save supply root absent",
        );
        report.compared("HubMotorization", 0);
        return Ok(());
    };
    let mut actual_settings = BTreeMap::new();
    for row in motorization {
        let owner = required_country_tag(row.owner, links)?;
        required_country_tag(row.actor, links)?;
        if actual_settings
            .insert((owner, row.node_index, row.map_entry_index), row)
            .is_some()
        {
            return Err(invalid("duplicate motorization source position"));
        }
    }
    let mut setting_count = 0;
    for (tag, supply) in systems {
        let country = String::from_utf8_lossy(tag);
        for expected in &supply.motorization {
            let label = format!(
                "{country}.motorization[{}/{}]",
                expected.node_index, expected.map_entry_index
            );
            let Some(actual) = actual_settings.remove(&(
                tag.as_slice(),
                expected.node_index,
                expected.map_entry_index,
            )) else {
                report.difference(label, "absent", "present");
                continue;
            };
            setting_count += 1;
            equal(
                report,
                format!("{label}.province"),
                actual.province,
                expected.province,
            );
            equal(
                report,
                format!("{label}.node_type"),
                actual.node_type,
                expected.node_type,
            );
            equal(
                report,
                format!("{label}.actor"),
                required_country_tag(actual.actor, links)?,
                expected.actor.as_slice(),
            );
            equal(
                report,
                format!("{label}.level"),
                actual.level,
                expected.level,
            );
        }
    }
    for ((tag, node, entry), actual) in &actual_settings {
        let label = format!(
            "{}.motorization[{node}/{entry}]",
            String::from_utf8_lossy(tag)
        );
        if unsaved_supply(systems, owners, tag) {
            report.limitation(STATELESS_SUPPLY, label);
            continue;
        }
        report.difference(label, *actual, "absent");
    }
    report.compared("HubMotorization", setting_count);
    report.limitation("hub rows capture explicit actor settings, not derived effective motorization or disabled-for-allies; save country tags cannot certify equivalent raw actor alias bits", "HubMotorization");
    Ok(())
}

fn compare_disrupted(
    frame: &DecodedFrame,
    saved: &economy::Pack,
    owners: &BTreeSet<&[u8]>,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let disrupted = complete::<tables::DisruptedSupply>(frame)?;
    let Some(systems) = &saved.supply else {
        report.difference(
            "DisruptedSupply",
            disrupted.len(),
            "save supply root absent",
        );
        report.compared("DisruptedSupply", 0);
        return Ok(());
    };
    let mut actual_disrupted = BTreeMap::new();
    for row in disrupted {
        let country = required_country_tag(row.country, links)?;
        if actual_disrupted
            .insert((country, row.province, row.node_type), row)
            .is_some()
        {
            return Err(invalid("duplicate full disrupted supply key"));
        }
    }
    let mut disrupted_count = 0;
    for (tag, supply) in systems {
        let country = String::from_utf8_lossy(tag);
        let mut saved_keys = std::collections::BTreeSet::new();
        for expected in &supply.disrupted {
            let key = (
                tag.as_slice(),
                expected.province as u32,
                expected.node_type as u32,
            );
            if !saved_keys.insert(key) {
                return Err(invalid("duplicate full disrupted supply key in save"));
            }
            let label = format!(
                "{country}.disrupted_supply[{}/{}]",
                expected.province, expected.node_type
            );
            let Some(actual) = actual_disrupted.remove(&key) else {
                report.difference(label, "absent", "present");
                continue;
            };
            disrupted_count += 1;
            equal(
                report,
                format!("{label}.value"),
                actual.value,
                expected.value.raw,
            );
        }
    }
    for (tag, province, node_type) in actual_disrupted.keys() {
        let label = format!(
            "{}.disrupted_supply[{province}/{node_type}]",
            String::from_utf8_lossy(tag)
        );
        if unsaved_supply(systems, owners, tag) {
            report.limitation(STATELESS_SUPPLY, label);
            continue;
        }
        report.difference(label, "present", "absent");
    }
    report.compared("DisruptedSupply", disrupted_count);
    Ok(())
}

fn compare_foreign(
    frame: &DecodedFrame,
    saved: &economy::Pack,
    owners: &BTreeSet<&[u8]>,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let foreign = complete::<tables::ForeignHomebases>(frame)?;
    let Some(systems) = &saved.supply else {
        report.difference("ForeignHomebases", foreign.len(), "save supply root absent");
        report.compared("ForeignHomebases", 0);
        return Ok(());
    };
    let mut actual_foreign = BTreeMap::new();
    for row in foreign {
        let country = required_country_tag(row.country, links)?;
        if actual_foreign.insert((country, row.ordinal), row).is_some() {
            return Err(invalid("duplicate foreign homebase source position"));
        }
    }
    let mut foreign_count = 0;
    for (tag, supply) in systems {
        let country = String::from_utf8_lossy(tag);
        for expected in &supply.foreign_homebases {
            let label = format!("{country}.foreign_homebase[{}]", expected.ordinal);
            let Some(actual) = actual_foreign.remove(&(tag.as_slice(), expected.ordinal)) else {
                report.difference(label, "absent", "present");
                continue;
            };
            foreign_count += 1;
            equal(
                report,
                format!("{label}.province"),
                actual.province,
                expected.province,
            );
            equal(
                report,
                format!("{label}.inner_province"),
                actual.province,
                expected.inner_province,
            );
            equal(
                report,
                format!("{label}.base"),
                actual.base,
                u8::from(expected.base),
            );
            compare_text(
                report,
                &format!("{label}.source (save add)"),
                &actual.source,
                &expected.source,
            );
            for (name, actual, expected) in [
                ("supply", actual.supply, expected.supply.raw),
                ("start", actual.start, expected.start.raw),
                ("penalty", actual.penalty, expected.penalty.raw),
                ("duration", actual.duration, expected.duration.raw),
                ("hours", actual.hours, expected.hours.raw),
                ("decay", actual.decay, expected.decay.raw),
            ] {
                equal(report, format!("{label}.{name}"), actual, expected);
            }
        }
    }
    for (tag, ordinal) in actual_foreign.keys() {
        let label = format!(
            "{}.foreign_homebase[{ordinal}]",
            String::from_utf8_lossy(tag)
        );
        if unsaved_supply(systems, owners, tag) {
            report.limitation(STATELESS_SUPPLY, label);
            continue;
        }
        report.difference(label, "present", "absent");
    }
    report.compared("ForeignHomebases", foreign_count);
    report.limitation("foreign homebase serializer omits added_penalty; source is serialized under add and compares through its first NUL; the other captured fields compare literally, including native array order and duplicate provinces", "ForeignHomebases");
    Ok(())
}

fn compare_fuel(
    frame: &DecodedFrame,
    saved: &economy::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut actual = BTreeMap::new();
    for row in complete::<tables::CountryFuel>(frame)? {
        let tag = required_country_tag(row.country, links)?;
        if actual.insert(tag, row).is_some() {
            return Err(invalid("duplicate resolved country in fuel table"));
        }
    }
    let mut count = 0;
    for (tag, country) in report.canonical_countries(&saved.countries) {
        let label = format!("{}.fuel", String::from_utf8_lossy(tag));
        let Some(actual) = actual.remove(tag.as_slice()) else {
            report.difference(label, "absent", "saved country present");
            continue;
        };
        let Some(expected) = &country.fuel else {
            report.difference(label, "present", "fuel_status absent");
            continue;
        };
        count += 1;
        for (name, actual, expected) in [
            ("current_raw", actual.current_raw, expected.current.raw()?),
            ("maximum_raw", actual.maximum_raw, expected.maximum.raw()?),
            (
                "lend_lease_gain_raw",
                actual.lend_lease_gain_raw,
                expected.lend_lease_gain.raw()?,
            ),
        ] {
            equal(report, format!("{label}.{name}"), actual, expected);
        }
        let mut sums = [0_i128; 6];
        for entry in expected.history.iter().filter(|entry| entry.index != -1) {
            let values = [
                entry.gain.raw,
                entry.consumed.raw()?,
                entry.other.raw()?,
                entry.received[0].raw,
                entry.received[1].raw,
                entry.received[2].raw,
            ];
            for (sum, value) in sums.iter_mut().zip(values) {
                *sum += i128::from(value);
            }
        }
        for ((name, actual), expected) in [
            ("ring_gain_raw", actual.ring_gain_raw),
            ("ring_consumed_raw", actual.ring_consumed_raw),
            ("ring_other_raw", actual.ring_other_raw),
            ("ring_received_0_raw", actual.ring_received_0_raw),
            ("ring_received_1_raw", actual.ring_received_1_raw),
            ("ring_received_2_raw", actual.ring_received_2_raw),
        ]
        .into_iter()
        .zip(sums)
        {
            equal(
                report,
                format!("{label}.{name}"),
                i128::from(actual),
                expected,
            );
        }
        report_fuel_award_witness(actual, report);
    }
    for tag in actual.keys() {
        report.difference(
            format!("{}.fuel", String::from_utf8_lossy(tag)),
            "present",
            "country absent",
        );
    }
    report.compared("CountryFuel", count);
    report.limitation(
        "fuel quantities compare exact /32768 bits through the injective native five-decimal formatter; native i64::MIN has malformed text serialization and is refused",
        "CountryFuel",
    );
    report.limitation(
        "the per-hour fuel history ring entries are not recorded, only the exact sums of the 24 ring slots, so a sum difference cannot be attributed to an hour",
        "CountryFuel",
    );
    report.limitation(
        "the pending fuel history entry (saved index=-1) of the hour in progress is not recorded",
        "CountryFuel",
    );
    Ok(())
}

fn report_fuel_award_witness(row: &replay_data::rows::CountryFuelRow, report: &mut Report) {
    let reason = match &row.award_day {
        replay_data::rows::Recorded::Value(_) => {
            "applied fuel updates are recorded as day sums; their per-update source witness, which only the save comparison read, is not recorded"
        }
        replay_data::rows::Recorded::NotRecorded => {
            "applied fuel source and recipient history not recorded"
        }
    };
    report.limitation(reason, "CountryFuel");
}

fn compare_fuel_consumers(
    frame: &DecodedFrame,
    saved: &economy::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut actual = BTreeMap::new();
    for row in complete::<tables::CountryFuelConsumers>(frame)? {
        let tag = required_country_tag(row.country, links)?;
        if actual.insert((tag, row.index), row).is_some() {
            return Err(invalid("duplicate resolved fuel consumer"));
        }
    }
    let mut count = 0;
    for (tag, country) in report.canonical_countries(&saved.countries) {
        let Some(fuel) = &country.fuel else {
            continue;
        };
        for expected in &fuel.consumers {
            let label = format!(
                "{}.fuel_consumer[{}]",
                String::from_utf8_lossy(tag),
                expected.index
            );
            let Some(actual) = actual.remove(&(tag.as_slice(), expected.index)) else {
                report.difference(label, "absent", "saved consumer present");
                continue;
            };
            count += 1;
            equal(
                report,
                format!("{label}.priority"),
                actual.priority,
                expected.priority,
            );
            for (name, actual, expected) in [
                (
                    "requested_raw",
                    actual.requested_raw,
                    expected.requested.raw,
                ),
                ("received_raw", actual.received_raw, expected.received.raw),
            ] {
                equal(report, format!("{label}.{name}"), actual, expected);
            }
        }
    }
    for (tag, index) in actual.keys() {
        report.difference(
            format!("{}.fuel_consumer[{index}]", String::from_utf8_lossy(tag)),
            "present",
            "absent",
        );
    }
    report.compared("CountryFuelConsumers", count);
    Ok(())
}

const LOGISTICS_STOCK_SERIES: u32 = 15;
const LOGISTICS_BALANCE_SERIES: u32 = 16;
const LOGISTICS_DAILY_RECORDS: i32 = 30;

fn compare_logistics(
    frame: &DecodedFrame,
    saved: &economy::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let mut actual: BTreeMap<&[u8], Vec<&replay_data::rows::LogisticsEquipmentRow>> =
        BTreeMap::new();
    for row in complete::<tables::LogisticsEquipment>(frame)? {
        let tag = required_country_tag(row.country, links)?;
        actual.entry(tag).or_default().push(row);
    }
    for rows in actual.values_mut() {
        rows.sort_by_key(|row| row.archetype);
        if rows
            .iter()
            .zip(0..)
            .any(|(row, index)| row.archetype != index)
        {
            return Err(invalid(
                "logistics rows of a country are not every archetype index from zero",
            ));
        }
    }
    let mut count = 0;
    for (tag, country) in report.canonical_countries(&saved.countries) {
        let name = String::from_utf8_lossy(tag);
        let history = match &country.logistics {
            Some(economy::Logistics::History(history)) => Some(history),
            Some(economy::Logistics::Empty) | None => None,
        };
        let (history, rows) = match (history, actual.remove(tag.as_slice())) {
            (None, None) => continue,
            (Some(_), None) => {
                report.difference(
                    format!("{name}.logistics"),
                    "no rows",
                    "saved history (not AI-controlled)",
                );
                continue;
            }
            (None, Some(_)) => {
                report.difference(
                    format!("{name}.logistics"),
                    "rows",
                    "no saved history (AI-controlled)",
                );
                continue;
            }
            (Some(history), Some(rows)) => (history, rows),
        };
        let fields = rows.len();
        let mut records = [None, None];
        for (record, series) in records
            .iter_mut()
            .zip([LOGISTICS_STOCK_SERIES, LOGISTICS_BALANCE_SERIES])
        {
            let label = format!("{name}.logistics.series[{series}]");
            let Some(window) = history.get(&series).map(|history| &history.windows[0]) else {
                report.difference(label, "read", "series absent");
                continue;
            };
            equal(
                report,
                format!("{label}.max_elements"),
                window.max_elements,
                LOGISTICS_DAILY_RECORDS,
            );
            let slot = usize::try_from(window.offset)
                .map_err(|_| invalid("saved logistics offset is negative"))?;
            match window.record(slot, fields) {
                Ok(values) => *record = Some(values),
                Err(error) => {
                    report.difference(
                        format!("{label}.data"),
                        format!("{fields} archetypes"),
                        error.to_string(),
                    );
                }
            }
        }
        let [Some(stock), Some(balance)] = records else {
            continue;
        };
        for (row, (stock, balance)) in rows.iter().zip(stock.iter().zip(&balance)) {
            let archetype = tokens.get(&row.token).map_or_else(
                || row.archetype.to_string(),
                |token| String::from_utf8_lossy(token).into_owned(),
            );
            let label = format!("{name}.logistics[{archetype}]");
            equal(report, format!("{label}.stock_raw"), row.stock_raw, *stock);
            equal(
                report,
                format!("{label}.balance_raw"),
                row.balance_raw,
                *balance,
            );
            if matches!(row.available_raw, replay_data::rows::Recorded::Value(None)) {
                report.limitation("available_raw is unsaved and no matching native write operand is retained at this read", format!("{label}.available_raw"));
            }
            count += 1;
        }
    }
    for tag in actual.keys() {
        report.difference(
            format!("{}.logistics", String::from_utf8_lossy(tag)),
            "rows",
            "country absent",
        );
    }
    report.compared("LogisticsEquipment", count);
    report.limitation(
        "historical available_raw is a temporary operand at history writes, not a loop-history field; the serializer writes only 19 histories and never this operand. Current saved convoy/supply state cannot replace it. Validate it independently at matched native writes; all saved stock15/balance16 differences remain fatal",
        "LogisticsEquipment.available_raw",
    );
    report.limitation(
        "the save does not name equipment archetypes: rows compare by record index, and the recorded archetype token has no save counterpart",
        "LogisticsEquipment",
    );
    report.limitation(
        "only the current daily record of the stock and balance series is recorded; the save's other daily records, its twelve- and seven-record windows and its other 17 series are not compared",
        "LogisticsEquipment",
    );
    Ok(())
}

fn compare_purchases(
    frame: &DecodedFrame,
    saved: &economy::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let rows = complete::<tables::PurchaseContracts>(frame)?;
    let Some(expected) = &saved.purchases else {
        report.difference("purchase contracts", rows.len(), "equipment_market absent");
        report.compared("PurchaseContracts", 0);
        return Ok(());
    };
    let mut actual = BTreeMap::new();
    for row in rows {
        if row.id == 0 || actual.insert(row.id, row).is_some() {
            return Err(invalid("null or duplicate full purchase contract handle"));
        }
    }
    let mut count = 0;
    for (id, expected) in expected {
        let label = format!("purchase contract type={} id={}", *id as u32, *id >> 32);
        let Some(actual) = actual.remove(id) else {
            report.difference(label, "absent", "present");
            continue;
        };
        count += 1;
        equal(
            report,
            format!("{label}.seller"),
            required_country_tag(actual.seller, links)?,
            expected.seller.as_slice(),
        );
        equal(
            report,
            format!("{label}.buyer"),
            required_country_tag(actual.buyer, links)?,
            expected.buyer.as_slice(),
        );
        for (name, actual, expected) in [
            ("speed", actual.speed, expected.speed),
            ("route_days", actual.route_days, expected.route_days),
            (
                "completed_deliveries",
                actual.completed_deliveries,
                expected.completed_deliveries,
            ),
        ] {
            equal(report, format!("{label}.{name}"), actual, expected);
        }
        for (name, actual, expected) in [
            (
                "route_efficiency_sum_raw",
                actual.route_efficiency_sum_raw,
                expected.route_efficiency_sum.raw,
            ),
            (
                "factory_cic_progress_raw",
                actual.factory_cic_progress_raw,
                expected.factory_cic_progress.raw,
            ),
            (
                "equipment_transfer_progress_raw",
                actual.equipment_transfer_progress_raw,
                expected.equipment_transfer_progress.raw,
            ),
            (
                "collected_raw",
                actual.collected_raw,
                expected.collected.raw,
            ),
        ] {
            equal(report, format!("{label}.{name}"), actual, expected);
        }
    }
    for id in actual.keys() {
        report.difference(format!("purchase contract {id:#018x}"), "present", "absent");
    }
    report.compared("PurchaseContracts", count);
    report.limitation("purchase convoy assigned/total counters have no established direct save mapping; route efficiency is the native accumulated sum and route days remain separate", "PurchaseContracts");
    report.limitation("save country tags identify resolved countries but cannot certify equivalent raw country-link alias bits", "PurchaseContracts");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use content_reader_parser::parser;
    use replay_data::{
        rows::CountryExperienceRow, rows::CountryFuelConsumerRow, rows::CountryFuelRow,
        rows::CountryResourceRow, rows::HubMotorizationRow, rows::LexicalTokenRow,
        rows::ResourceDefinitionRow, rows::ResourceExchangeRow, rows::TableId, rows::TableRows,
    };
    use replay_frame::{frame::Metrics, frame::TableView};

    fn fuel_status(scalars: &str) -> String {
        let quantity = |raw: i128| {
            let numerator = raw * 100_000 / 32_768;
            format!("{}.{:05}", numerator / 100_000, numerator % 100_000)
        };
        let mut text = format!(
            "{scalars} fuel_consumer_data={{ index=0 priority=1 received=83.62571 requested=83.62571 }} fuel_consumer_data={{ index=1 priority=1 received=0 requested=8.1459 }} fuel_consumer_data={{ index=2 priority=2 received=-0.00001 requested=0 }} history_index=13 remaining_hours=1 history={{ index=-1 fuel=0.00000 consumed=1.00000 fuel_gain=999 other=1.00000 requested={{ 1 2 3 }} received={{ 4 5 6 }} }}"
        );
        for index in 0..24 {
            let raw = i128::from(index + 1);
            text += &format!(
                " history={{ index={index} fuel={} consumed=265.14755 fuel_gain=390.4 other={} requested={{ 40.85669 8.1459 216.145 }} received={{ 40.85669 8.1459 -0.00001 }} }}",
                quantity(raw),
                quantity(raw)
            );
        }
        text
    }

    const RING_SUMS: [i64; 6] = [
        24 * 39_040_000,
        24 * 8_688_355,
        300,
        24 * 4_085_669,
        24 * 814_590,
        -24,
    ];

    fn with_ring_sums(row: CountryFuelRow) -> CountryFuelRow {
        let [gain, consumed, other, received_0, received_1, received_2] = RING_SUMS;
        CountryFuelRow {
            ring_gain_raw: gain,
            ring_consumed_raw: consumed,
            ring_other_raw: other,
            ring_received_0_raw: received_0,
            ring_received_1_raw: received_1,
            ring_received_2_raw: received_2,
            ..row
        }
    }

    #[test]
    fn foreign_source_compares_native_text_prefix_without_certifying_nul_suffix() {
        let mut report = Report::default();
        compare_text(&mut report, "source", b"fleet\0hidden", b"fleet");
        assert!(report.differences.is_empty());
        assert_eq!(report.limitations.len(), 1);
        compare_text(&mut report, "source", b"fleet\0hidden", b"changed");
        assert_eq!(report.differences.len(), 1);
        compare_text(&mut report, "empty", b"", b"");
        assert_eq!(report.limitations.len(), 1);
    }

    #[test]
    fn failed_economy_tables_preserve_independent_diagnostics_and_strict_result() {
        let saved = content_reader_save::parse(
            br#"HOI4txt
            date=1942.9.6.2 game_unique_id=test version=test game_unique_seed=1 debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 theater_group_index=6 military_deployment_line_index=7 military_deployment_conveyor_index=8 equipment_variant_index=9 country_leader_index=10 navy_id=11 land_combat_id=12 multiplayer_random_count=-13
            countries={} states={} provinces={}
            equipment_market={contracts={} requests={0}} supply_system_2={countries={}}
            "#,
        )
        .unwrap();
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 2,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index]))
            })
            .map(Into::into),
        };
        for table in [
            TableId::CountryFuel,
            TableId::ResourceDefinitions,
            TableId::ProvinceSupply,
        ] {
            *frame.tables[table.index()] = TableView::Unavailable {
                reason: "read failed".into(),
            };
        }
        *frame.tables[TableId::HubMotorization.index()] = TableView::Truncated {
            rows: TableRows::HubMotorization(Vec::new()),
            missing: 1,
        };
        let mut report = Report::default();
        assert!(
            compare(
                &frame,
                &saved,
                &BTreeMap::new(),
                &TokenNames::new(),
                &mut report,
            )
            .is_err()
        );
        for name in [
            "CountryFuel",
            "ResourceExchanges",
            "HubMotorization",
            "ProvinceSupply",
            "ResourceDefinitions",
            "CountryResources",
        ] {
            assert!(
                report
                    .failed_comparisons
                    .contains_key(&format!("economy {name}"))
            );
        }
        for name in [
            "CountryFuelConsumers",
            "CountryExperience",
            "PurchaseContracts",
            "DisruptedSupply",
            "ForeignHomebases",
        ] {
            assert_eq!(report.compared_tables[name], 0);
        }
        assert!(report.outcome().is_err());
    }

    #[test]
    fn extra_motorization_setting_retains_exact_operands_even_at_zero() {
        let saved = economy::parse(
            &parser::parse(
                b"countries={} supply_system_2={countries={SIK={settings={node={id={2015 2} data={motorization_level={}}} node={id={4709 2} data={motorization_level={}}}}}}}",
            )
            .unwrap()
            .items,
        )
        .unwrap();
        let country = CountryRow {
            id: 11,
            tag: b"SIK".to_vec(),
            ..Default::default()
        };
        let links = BTreeMap::from([(11, &country)]);
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 2,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index]))
            })
            .map(Into::into),
        };
        *frame.tables[TableId::HubMotorization.index()] =
            TableView::Complete(TableRows::HubMotorization(vec![HubMotorizationRow {
                owner: 11,
                actor: 11,
                node_index: 1,
                map_entry_index: 0,
                province: 4709,
                node_type: 2,
                level: 0,
            }]));
        let mut report = Report::default();
        compare_motorization(
            &frame,
            &saved,
            &BTreeSet::from([b"SIK".as_slice()]),
            &links,
            &mut report,
        )
        .unwrap();
        assert_eq!(report.differences.len(), 1);
        let difference = &report.differences[0];
        assert!(difference.contains("SIK.motorization[1/0]"));
        assert!(difference.contains("province: 4709"));
        assert!(difference.contains("actor: 11"));
        assert!(difference.contains("level: 0"));
        assert_eq!(report.compared_tables["HubMotorization"], 0);
    }

    #[test]
    fn supply_rows_of_a_country_without_states_have_no_save_counterpart() {
        let saved = economy::parse(
            &parser::parse(b"countries={} supply_system_2={countries={GER={settings={}}}}")
                .unwrap()
                .items,
        )
        .unwrap();
        let countries = [(11, "SND"), (12, "GER"), (13, "ITA")].map(|(id, tag)| CountryRow {
            id,
            tag: tag.as_bytes().to_vec(),
            ..Default::default()
        });
        let links = countries
            .iter()
            .map(|row| (row.id, row))
            .collect::<BTreeMap<_, _>>();
        let owners = BTreeSet::from([b"GER".as_slice(), b"ITA".as_slice()]);
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 2,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index]))
            })
            .map(Into::into),
        };
        let row = |owner| HubMotorizationRow {
            owner,
            actor: 12,
            node_index: 0,
            map_entry_index: 0,
            province: 1069,
            node_type: 2,
            level: 2,
        };
        *frame.tables[TableId::HubMotorization.index()] =
            TableView::Complete(TableRows::HubMotorization(vec![row(11), row(13)]));
        *frame.tables[TableId::DisruptedSupply.index()] =
            TableView::Complete(TableRows::DisruptedSupply(vec![
                replay_data::rows::DisruptedSupplyRow {
                    country: 11,
                    province: 1069,
                    node_type: 2,
                    value: 1,
                },
            ]));
        let mut report = Report::default();
        compare_motorization(&frame, &saved, &owners, &links, &mut report).unwrap();
        compare_disrupted(&frame, &saved, &owners, &links, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        assert!(report.differences[0].contains("ITA.motorization[0/0]"));
        let unsaved = &report.limitations[STATELESS_SUPPLY];
        assert_eq!(unsaved.len(), 2);
        assert!(
            unsaved
                .iter()
                .any(|subject| subject == "SND.motorization[0/0]")
        );
        assert!(
            unsaved
                .iter()
                .any(|subject| subject == "SND.disrupted_supply[1069/2]")
        );
    }

    #[test]
    fn exchange_without_lexical_names_still_checks_numeric_fields_and_table_completeness() {
        let saved = economy::parse(
            &parser::parse(
                br#"countries={GER={resources={export={
            id={id=1 type=4713} receiver=GER resource=steel efficiency=1
            efficiency_due_to_lost_convoys=0 delivered=-0.00001 required_cic=2 lended_cic=1
        } produced={} transfer_overlord_subject={} imported={} to_use={ {} {} {} } to_export={} base_export={} exported={}}}}"#,
            )
            .unwrap()
            .items,
        )
        .unwrap();
        let country = CountryRow {
            id: 11,
            canonical_slot: 7,
            tag: b"GER".to_vec(),
            ..Default::default()
        };
        let links = BTreeMap::from([(11, &country)]);
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 2,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index]))
            })
            .map(Into::into),
        };
        *frame.tables[TableId::ResourceDefinitions.index()] =
            TableView::Complete(TableRows::ResourceDefinitions(vec![
                ResourceDefinitionRow {
                    id: 1,
                    token: 77,
                    active: 1,
                    factory_cost: 100_000,
                },
            ]));
        *frame.tables[TableId::ResourceExchanges.index()] =
            TableView::Complete(TableRows::ResourceExchanges(vec![ResourceExchangeRow {
                id: (1_u64 << 32) | 4713,
                exporter: 11,
                importer: 11,
                resource: 1,
                efficiency_a: 100_000,
                delivered: -1,
                required_factories: 2,
                assigned_factories: 1,
                ..Default::default()
            }]));
        *frame.tables[TableId::LexicalTokens.index()] = TableView::Unavailable {
            reason: "lexer writer closure unproved".into(),
        };
        let mut report = Report::default();
        let tokens = super::super::lexical_tokens(&frame, &mut report).unwrap();
        compare_exchanges(&frame, &saved, &links, &tokens, &mut report).unwrap();
        assert!(report.differences.is_empty());
        assert_eq!(report.compared_tables["ResourceExchanges"], 1);
        assert!(
            report
                .limitations
                .values()
                .flatten()
                .any(|subject| subject.contains(".resource token=77"))
        );
        let TableView::Complete(TableRows::ResourceExchanges(rows)) =
            &mut *frame.tables[TableId::ResourceExchanges.index()]
        else {
            unreachable!()
        };
        rows[0].convoys_assigned = 1;
        rows[0].convoys_required = 2;
        let mut report = Report::default();
        let tokens = super::super::lexical_tokens(&frame, &mut report).unwrap();
        compare_exchanges(&frame, &saved, &links, &tokens, &mut report).unwrap();
        assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
        assert!(report.differences[0].contains(".convoys_assigned"));
        assert!(report.differences[1].contains(".convoys_required"));
        let TableView::Complete(TableRows::ResourceExchanges(rows)) =
            &mut *frame.tables[TableId::ResourceExchanges.index()]
        else {
            unreachable!()
        };
        rows[0].convoys_assigned = 0;
        rows[0].convoys_required = 0;
        for (literal, differences, nul_suffix) in [
            (b"steel".as_slice(), 0, false),
            (b"Steel".as_slice(), 1, false),
            (b"steel\0hidden".as_slice(), 0, true),
            (b"".as_slice(), 1, false),
        ] {
            *frame.tables[TableId::LexicalTokens.index()] =
                TableView::Complete(TableRows::LexicalTokens(vec![
                    LexicalTokenRow {
                        token: 1,
                        literal: b"wrong direct resource-id lookup".to_vec(),
                    },
                    LexicalTokenRow {
                        token: 77,
                        literal: literal.to_vec(),
                    },
                ]));
            let mut report = Report::default();
            let tokens = super::super::lexical_tokens(&frame, &mut report).unwrap();
            compare_exchanges(&frame, &saved, &links, &tokens, &mut report).unwrap();
            assert_eq!(report.differences.len(), differences);
            assert_eq!(report.compared_tables["ResourceExchanges"], 1);
            assert_eq!(
                report
                    .limitations
                    .keys()
                    .any(|reason| reason.contains("NUL/suffix")),
                nul_suffix,
            );
            assert!(
                !report
                    .limitations
                    .keys()
                    .any(|reason| reason.contains("lacks the captured lexical token spelling"))
            );
        }
        *frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
                token: 1,
                literal: b"steel".to_vec(),
            }]));
        let mut report = Report::default();
        let tokens = super::super::lexical_tokens(&frame, &mut report).unwrap();
        compare_exchanges(&frame, &saved, &links, &tokens, &mut report).unwrap();
        assert!(report.differences.is_empty());
        assert!(
            report
                .limitations
                .values()
                .flatten()
                .any(|subject| subject.contains(".resource token=77"))
        );
        *frame.tables[TableId::LexicalTokens.index()] = TableView::Unavailable {
            reason: "lexer writer closure unproved".into(),
        };
        let TableView::Complete(TableRows::ResourceExchanges(rows)) =
            &mut *frame.tables[TableId::ResourceExchanges.index()]
        else {
            unreachable!()
        };
        rows[0].delivered += 1;
        let mut report = Report::default();
        let tokens = super::super::lexical_tokens(&frame, &mut report).unwrap();
        compare_exchanges(&frame, &saved, &links, &tokens, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
        *frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
                token: 77,
                literal: b"rubber".to_vec(),
            }]));
        let mut report = Report::default();
        let tokens = super::super::lexical_tokens(&frame, &mut report).unwrap();
        compare_exchanges(&frame, &saved, &links, &tokens, &mut report).unwrap();
        assert_eq!(report.differences.len(), 2);
        let mut absent_resource = saved.clone();
        absent_resource
            .countries
            .get_mut(b"GER".as_slice())
            .unwrap()
            .exports
            .as_mut()
            .unwrap()[0]
            .resource = None;
        let mut report = Report::default();
        compare_exchanges(&frame, &absent_resource, &links, &tokens, &mut report).unwrap();
        assert_eq!(report.differences.len(), 2);
        assert!(report.differences.iter().any(|difference| {
            difference.contains(".resource:") && difference.contains("save \"absent\"")
        }));
        *frame.tables[TableId::LexicalTokens.index()] = TableView::Truncated {
            rows: TableRows::LexicalTokens(Vec::new()),
            missing: 1,
        };
        assert!(super::super::lexical_tokens(&frame, &mut Report::default()).is_err());
        *frame.tables[TableId::LexicalTokens.index()] = TableView::Unavailable {
            reason: "unproved".into(),
        };
        *frame.tables[TableId::ResourceDefinitions.index()] = TableView::Unavailable {
            reason: "read failed".into(),
        };
        let mut report = Report::default();
        let tokens = super::super::lexical_tokens(&frame, &mut report).unwrap();
        assert!(compare_exchanges(&frame, &saved, &links, &tokens, &mut report).is_err());
    }

    #[test]
    fn country_resources_compare_every_saved_pool() {
        let saved = economy::parse(
            &parser::parse(
                br#"countries={GER={resources={
            produced={ steel=301 } transfer_overlord_subject={ steel=-2.1 } imported={ steel=0.00001 }
            to_use={ { steel=24 } { steel=6 } { steel=-30 } }
            to_export={ steel=8 } base_export={ steel=7 } exported={ steel=5 }
        }} ENG={resources={
            produced={ } transfer_overlord_subject={ } imported={ } to_use={ { } { } { } }
            to_export={ } base_export={ } exported={ }
        }}}"#,
            )
            .unwrap()
            .items,
        )
        .unwrap();
        let rows = [
            CountryRow {
                id: 11,
                tag: b"GER".to_vec(),
                ..Default::default()
            },
            CountryRow {
                id: 12,
                tag: b"ENG".to_vec(),
                ..Default::default()
            },
        ];
        let links = BTreeMap::from([(11, &rows[0]), (12, &rows[1])]);
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 2,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index]))
            })
            .map(Into::into),
        };
        *frame.tables[TableId::ResourceDefinitions.index()] =
            TableView::Complete(TableRows::ResourceDefinitions(vec![
                ResourceDefinitionRow {
                    id: 1,
                    token: 77,
                    active: 1,
                    factory_cost: 100_000,
                },
            ]));
        let recorded = |country| CountryResourceRow {
            country,
            resource: 1,
            ..Default::default()
        };
        let german = CountryResourceRow {
            produced: 30_100_000,
            adjustment: -210_000,
            imported: 1,
            used: 2_400_000,
            special_total: 3_000_000,
            production_demand: -3_000_000,
            to_export: 800_000,
            base_export: 700_000,
            exported: 500_000,
            ..recorded(11)
        };
        let set = |frame: &mut DecodedFrame, rows: Vec<CountryResourceRow>| {
            *frame.tables[TableId::CountryResources.index()] =
                TableView::Complete(TableRows::CountryResources(rows));
        };
        set(&mut frame, vec![german.clone(), recorded(12)]);
        let tokens = TokenNames::from([(77, b"steel".as_slice())]);
        let mut report = Report::default();
        compare_resources(&frame, &saved, &links, &tokens, &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["CountryResources"], 2);

        for change in 0..9 {
            let mut row = german.clone();
            let field = [
                &mut row.produced,
                &mut row.adjustment,
                &mut row.imported,
                &mut row.used,
                &mut row.special_total,
                &mut row.production_demand,
                &mut row.to_export,
                &mut row.base_export,
                &mut row.exported,
            ]
            .into_iter()
            .nth(change)
            .unwrap();
            *field += 1;
            set(&mut frame, vec![row, recorded(12)]);
            let mut report = Report::default();
            compare_resources(&frame, &saved, &links, &tokens, &mut report).unwrap();
            let expected = if change == 3 { 2 } else { 1 };
            assert_eq!(report.differences.len(), expected, "operand {change}");
        }

        set(&mut frame, vec![recorded(12)]);
        let mut report = Report::default();
        compare_resources(&frame, &saved, &links, &tokens, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
        assert!(report.differences[0].contains("GER.resources[steel]"));

        set(&mut frame, vec![german, recorded(12)]);
        let mut report = Report::default();
        compare_resources(&frame, &saved, &links, &TokenNames::new(), &mut report).unwrap();
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("lacks the captured lexical token spelling"))
        );
    }

    fn resource_frame(definitions: &[(u32, u32)], rows: Vec<CountryResourceRow>) -> DecodedFrame {
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 2,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index]))
            })
            .map(Into::into),
        };
        *frame.tables[TableId::ResourceDefinitions.index()] =
            TableView::Complete(TableRows::ResourceDefinitions(
                definitions
                    .iter()
                    .map(|&(id, token)| ResourceDefinitionRow {
                        id,
                        token,
                        active: 1,
                        factory_cost: 100_000,
                    })
                    .collect(),
            ));
        *frame.tables[TableId::CountryResources.index()] =
            TableView::Complete(TableRows::CountryResources(rows));
        frame
    }

    fn saved_resources(countries: &[(&str, &str, &str)]) -> economy::Pack {
        let mut text = String::from("countries={");
        for (tag, produced, to_use) in countries {
            text += &format!(
                "{tag}={{resources={{produced={{ {produced} }} transfer_overlord_subject={{ }} imported={{ }} to_use={{ {to_use} }} to_export={{ }} base_export={{ }} exported={{ }}}}}} "
            );
        }
        text += "}";
        economy::parse(&parser::parse(text.as_bytes()).unwrap().items).unwrap()
    }

    #[test]
    fn country_resources_census_detects_deleted_all_zero_rows() {
        let saved = saved_resources(&[
            ("GER", "steel=1", "{ } { } { }"),
            ("ENG", "", "{ } { } { }"),
        ]);
        let countries = [
            CountryRow {
                id: 11,
                tag: b"GER".to_vec(),
                ..Default::default()
            },
            CountryRow {
                id: 12,
                tag: b"ENG".to_vec(),
                ..Default::default()
            },
        ];
        let links = BTreeMap::from([(11, &countries[0]), (12, &countries[1])]);
        let tokens = TokenNames::from([(77, b"steel".as_slice()), (78, b"coal".as_slice())]);
        let row = |country, resource| CountryResourceRow {
            country,
            resource,
            ..Default::default()
        };
        let steel = CountryResourceRow {
            produced: 100_000,
            ..row(11, 1)
        };
        let all = vec![steel.clone(), row(11, 2), row(12, 1), row(12, 2)];
        let definitions = [(1, 77), (2, 78)];
        let mut report = Report::default();
        let frame = resource_frame(&definitions, all);
        compare_resources(&frame, &saved, &links, &tokens, &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["CountryResources"], 4);

        let frame = resource_frame(&definitions, vec![steel.clone(), row(12, 1), row(12, 2)]);
        let mut report = Report::default();
        compare_resources(&frame, &saved, &links, &tokens, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        assert!(report.differences[0].starts_with("GER.resources[coal]:"));
        assert!(report.differences[0].contains("one row per captured country"));
        assert_eq!(report.compared_tables["CountryResources"], 3);

        let frame = resource_frame(&definitions, vec![steel, row(11, 2)]);
        let mut report = Report::default();
        compare_resources(&frame, &saved, &links, &tokens, &mut report).unwrap();
        assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
        assert!(
            report
                .differences
                .iter()
                .all(|difference| difference.starts_with("ENG.resources["))
        );
    }

    #[test]
    fn country_resources_keep_repeated_resource_names_apart() {
        let saved = saved_resources(&[("GER", "steel=3 coal=2 steel=5", "{ steel=2 } { } { }")]);
        let country = CountryRow {
            id: 11,
            tag: b"GER".to_vec(),
            ..Default::default()
        };
        let links = BTreeMap::from([(11, &country)]);
        let definitions = [(1, 77), (2, 77), (3, 78)];
        let tokens = TokenNames::from([(77, b"steel".as_slice()), (78, b"coal".as_slice())]);
        let rows = |first_used: i64, second_used: i64, first: i64, second: i64| {
            vec![
                CountryResourceRow {
                    country: 11,
                    resource: 1,
                    produced: first,
                    used: first_used,
                    special_total: first_used,
                    ..Default::default()
                },
                CountryResourceRow {
                    country: 11,
                    resource: 2,
                    produced: second,
                    used: second_used,
                    special_total: second_used,
                    ..Default::default()
                },
                CountryResourceRow {
                    country: 11,
                    resource: 3,
                    produced: 200_000,
                    ..Default::default()
                },
            ]
        };
        let run = |rows: Vec<CountryResourceRow>, saved: &economy::Pack| {
            let mut report = Report::default();
            compare_resources(
                &resource_frame(&definitions, rows),
                saved,
                &links,
                &tokens,
                &mut report,
            )
            .unwrap();
            report
        };

        let report = run(rows(200_000, 0, 300_000, 500_000), &saved);
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["CountryResources"], 3);
        let subjects = &report.limitations[AMBIGUOUS_RESOURCE_JOIN];
        assert_eq!(subjects.len(), 1);
        assert!(subjects[0].starts_with("GER.resources[steel] ids [1, 2]:"));
        assert!(subjects[0].contains("used 1/2"));
        assert!(!subjects[0].contains("produced"));

        assert!(
            run(rows(0, 200_000, 300_000, 500_000), &saved)
                .differences
                .is_empty()
        );
        let report = run(rows(200_000, 0, 500_000, 300_000), &saved);
        assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
        assert!(report.differences[0].starts_with("GER.resources[steel id=1].produced:"));
        assert!(report.differences[1].starts_with("GER.resources[steel id=2].produced:"));
        let report = run(rows(200_001, 0, 300_000, 500_000), &saved);
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        assert!(
            report.differences[0].starts_with("GER.resources[steel].used nonzero in index order:")
        );
        let mut changed = rows(200_000, 0, 300_000, 500_000);
        changed[2].produced += 1;
        let report = run(changed, &saved);
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        assert!(report.differences[0].starts_with("GER.resources[coal].produced:"));
        let repeated = saved_resources(&[(
            "GER",
            "steel=3 coal=2 steel=5 steel=1",
            "{ steel=2 } { } { }",
        )]);
        let report = run(rows(200_000, 0, 300_000, 500_000), &repeated);
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        assert!(report.differences[0].contains("GER.resources[steel].produced (2 definitions)"));
    }

    fn fuel_row(frame: &mut DecodedFrame) -> &mut CountryFuelRow {
        let TableView::Complete(TableRows::CountryFuel(rows)) =
            &mut *frame.tables[TableId::CountryFuel.index()]
        else {
            unreachable!()
        };
        &mut rows[0]
    }

    #[test]
    fn applied_fuel_day_sums_report_their_unrecorded_witness_as_a_limitation() {
        use replay_data::{rows::FuelAwardDay, rows::Recorded};
        let row = CountryFuelRow {
            country_instance: Recorded::Value(Some(7)),
            award_day: Recorded::Value(FuelAwardDay {
                country_instance: 7,
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut report = Report::default();
        report_fuel_award_witness(&row, &mut report);
        assert!(report.differences.is_empty());
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("per-update source witness"))
        );
    }

    #[test]
    fn fuel_oracle_detects_one_native_unit_and_requires_complete_table() {
        let body = fuel_status(
            "fuel=1.00003 max_fuel=-1.00000 fuel_gain=-0.00001 fuel_cost=0 fuel_gain_per_oil=3.6 fuel_gain_from_states=64.4 fuel_gain_from_lend_lease=0.00003 fuel_consumption_from_lend_lease=-0.00003",
        );
        let saved = economy::parse(
            &parser::parse(format!("countries={{GER={{fuel_status={{{body}}}}}}}").as_bytes())
                .unwrap()
                .items,
        )
        .unwrap();
        let country = CountryRow {
            border_land_doctrine_level: replay_data::rows::Recorded::Value(None),
            id: 11,
            canonical_slot: 7,
            tag: b"GER".to_vec(),
            political_power_raw: None,
            command_power_raw: 0,
            stability_raw: 0,
            war_support_raw: 0,
            convoys_raw: None,
            peace_conference_participant: 0,
            manpower_residual: 0,
        };
        let links = BTreeMap::from([(11, &country), (12, &country)]);
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 2,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index]))
            })
            .map(Into::into),
        };
        *frame.tables[TableId::CountryFuel.index()] = TableView::Complete(TableRows::CountryFuel(
            vec![with_ring_sums(CountryFuelRow {
                country: 12,
                current_raw: 32_769,
                maximum_raw: -32_768,
                lend_lease_gain_raw: 1,
                ..Default::default()
            })],
        ));
        let mut report = Report::default();
        compare_fuel(&frame, &saved, &links, &mut report).unwrap();
        assert_eq!(report.differences, Vec::<String>::new());
        assert_eq!(report.compared_tables["CountryFuel"], 1);
        for reason in ["per-hour fuel history", "pending fuel history entry"] {
            assert!(report.limitations.keys().any(|key| key.contains(reason)));
        }
        for change in 0..7 {
            let TableView::Complete(TableRows::CountryFuel(rows)) =
                &mut *frame.tables[TableId::CountryFuel.index()]
            else {
                unreachable!()
            };
            let mut row = rows[0].clone();
            match change {
                0 => row.lend_lease_gain_raw += 1,
                1 => row.ring_gain_raw += 1,
                2 => row.ring_consumed_raw -= 1,
                3 => row.ring_other_raw += 1,
                4 => row.ring_received_0_raw += 1,
                5 => row.ring_received_1_raw += 1,
                _ => row.ring_received_2_raw -= 1,
            }
            let original = std::mem::replace(&mut rows[0], row);
            let mut report = Report::default();
            compare_fuel(&frame, &saved, &links, &mut report).unwrap();
            assert_eq!(report.differences.len(), 1, "change {change}");
            let TableView::Complete(TableRows::CountryFuel(rows)) =
                &mut *frame.tables[TableId::CountryFuel.index()]
            else {
                unreachable!()
            };
            rows[0] = original;
        }
        let TableView::Complete(rows) = &mut *frame.tables[TableId::CountryFuel.index()] else {
            unreachable!()
        };
        let TableRows::CountryFuel(rows) = rows else {
            unreachable!()
        };
        let fuel = &mut rows[0];
        fuel.current_raw -= 1;
        let mut report = Report::default();
        compare_fuel(&frame, &saved, &links, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
        fuel_row(&mut frame).current_raw += 1;
        fn fuel_text(saved: &mut economy::Pack) -> &mut economy::Fuel {
            saved
                .countries
                .get_mut(b"GER".as_slice())
                .unwrap()
                .fuel
                .as_mut()
                .unwrap()
        }
        let mut changed = saved.clone();
        fuel_text(&mut changed).history[24].received[1].raw += 1;
        let mut report = Report::default();
        compare_fuel(&frame, &changed, &links, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
        assert!(report.differences[0].contains("GER.fuel.ring_received_1_raw"));
        let mut changed = saved.clone();
        let pending = &mut fuel_text(&mut changed).history[0];
        assert_eq!(pending.index, -1);
        pending.gain.raw += 1;
        pending.requested[0].raw += 1;
        let mut report = Report::default();
        compare_fuel(&frame, &changed, &links, &mut report).unwrap();
        assert!(report.differences.is_empty());
        *frame.tables[TableId::CountryFuel.index()] = TableView::Unavailable {
            reason: "read failed".into(),
        };
        assert!(compare_fuel(&frame, &saved, &links, &mut Report::default()).is_err());
    }

    #[test]
    fn retired_fuel_values_are_not_compared() {
        let body = fuel_status(
            "fuel=1.00003 max_fuel=-1.00000 fuel_gain=-0.00001 fuel_cost=7 fuel_gain_per_oil=3.6 fuel_gain_from_states=64.4 fuel_gain_from_lend_lease=0.00003 fuel_consumption_from_lend_lease=-0.00003",
        );
        let saved = economy::parse(
            &parser::parse(format!("countries={{GER={{fuel_status={{{body}}}}}}}").as_bytes())
                .unwrap()
                .items,
        )
        .unwrap();
        let fuel = saved.countries[b"GER".as_slice()].fuel.as_ref().unwrap();
        assert_ne!(fuel.gain_from_states.raw, 0);
        assert_ne!(fuel.history_index, 0);
        let country = CountryRow {
            id: 12,
            tag: b"GER".to_vec(),
            ..Default::default()
        };
        let links = BTreeMap::from([(12, &country)]);
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 2,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index]))
            })
            .map(Into::into),
        };
        let row = with_ring_sums(CountryFuelRow {
            country: 12,
            current_raw: 32_769,
            maximum_raw: -32_768,
            lend_lease_gain_raw: 1,
            ..Default::default()
        });
        assert_eq!(row.gain_raw, Default::default());
        assert_eq!(row.history_index, Default::default());
        *frame.tables[TableId::CountryFuel.index()] =
            TableView::Complete(TableRows::CountryFuel(vec![row]));
        let run = |frame: &DecodedFrame, saved: &economy::Pack| {
            let mut report = Report::default();
            compare_fuel(frame, saved, &links, &mut report).unwrap();
            report
        };
        let report = run(&frame, &saved);
        assert_eq!(report.differences, Vec::<String>::new());
        assert_eq!(report.compared_tables["CountryFuel"], 1);
        let mut changed = saved.clone();
        let fuel = changed
            .countries
            .get_mut(b"GER".as_slice())
            .unwrap()
            .fuel
            .as_mut()
            .unwrap();
        fuel.gain.raw += 1;
        fuel.cost.raw += 1;
        fuel.gain_per_oil.raw += 1;
        fuel.gain_from_states.raw += 1;
        fuel.history_index += 1;
        fuel.remaining_hours += 1;
        for entry in &mut fuel.history {
            for requested in &mut entry.requested {
                requested.raw += 1;
            }
        }
        assert!(run(&frame, &changed).differences.is_empty());
        fuel_row(&mut frame).current_raw += 1;
        let differences = run(&frame, &changed).differences;
        assert_eq!(differences.len(), 1, "{differences:?}");
        assert!(differences[0].contains("GER.fuel.current_raw"));
    }

    #[test]
    fn fuel_consumer_oracle_compares_every_field_by_saved_index() {
        let body = fuel_status(
            "fuel=1.00003 max_fuel=1.00003 fuel_gain=2 fuel_cost=0 fuel_gain_per_oil=3.6 fuel_gain_from_states=64.4 fuel_gain_from_lend_lease=0.00000 fuel_consumption_from_lend_lease=0.00000",
        );
        let saved = economy::parse(
            &parser::parse(
                format!("countries={{ENG={{fuel_status={{{body}}}}} D01={{}}}}").as_bytes(),
            )
            .unwrap()
            .items,
        )
        .unwrap();
        let country = CountryRow {
            id: 11,
            canonical_slot: 7,
            tag: b"ENG".to_vec(),
            ..Default::default()
        };
        let links = BTreeMap::from([(11, &country)]);
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 2,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index]))
            })
            .map(Into::into),
        };
        let consumers = vec![
            CountryFuelConsumerRow {
                country: 11,
                index: 0,
                priority: 1,
                requested_raw: 8_362_571,
                received_raw: 8_362_571,
            },
            CountryFuelConsumerRow {
                country: 11,
                index: 1,
                priority: 1,
                requested_raw: 814_590,
                received_raw: 0,
            },
            CountryFuelConsumerRow {
                country: 11,
                index: 2,
                priority: 2,
                requested_raw: 0,
                received_raw: -1,
            },
        ];
        *frame.tables[TableId::CountryFuelConsumers.index()] =
            TableView::Complete(TableRows::CountryFuelConsumers(consumers.clone()));
        let mut report = Report::default();
        compare_fuel_consumers(&frame, &saved, &links, &mut report).unwrap();
        assert_eq!(report.differences, Vec::<String>::new());
        assert_eq!(report.compared_tables["CountryFuelConsumers"], 3);

        for field in 0..3 {
            let mut rows = consumers.clone();
            match field {
                0 => rows[2].priority = 1,
                1 => rows[1].requested_raw += 1,
                _ => rows[0].received_raw -= 1,
            }
            *frame.tables[TableId::CountryFuelConsumers.index()] =
                TableView::Complete(TableRows::CountryFuelConsumers(rows));
            let mut report = Report::default();
            compare_fuel_consumers(&frame, &saved, &links, &mut report).unwrap();
            assert_eq!(report.differences.len(), 1, "consumer field {field}");
        }
        let mut rows = consumers.clone();
        rows[2].index = 3;
        *frame.tables[TableId::CountryFuelConsumers.index()] =
            TableView::Complete(TableRows::CountryFuelConsumers(rows));
        let mut report = Report::default();
        compare_fuel_consumers(&frame, &saved, &links, &mut report).unwrap();
        assert_eq!(report.differences.len(), 2);
        assert_eq!(report.compared_tables["CountryFuelConsumers"], 2);

        *frame.tables[TableId::CountryFuelConsumers.index()] = TableView::Unavailable {
            reason: "read failed".into(),
        };
        assert!(compare_fuel_consumers(&frame, &saved, &links, &mut Report::default()).is_err());
    }

    #[test]
    fn experience_oracle_checks_every_channel_and_only_proven_zero_omissions() {
        let mut saved = economy::parse(&parser::parse(br#"countries={GER={experience_status={
            army_experience=0.00003 army_experience_daily=0.00006 army_experience_daily_training=0.00009
            navy_experience=0.00012 navy_experience_daily=0.00015 navy_experience_daily_training=0.00018
            air_experience=0.00021 air_experience_daily=0.00024 num_armies_for_training=1.00003
        }}}"#).unwrap().items).unwrap();
        let country = CountryRow {
            border_land_doctrine_level: replay_data::rows::Recorded::Value(None),
            id: 11,
            canonical_slot: 7,
            tag: b"GER".to_vec(),
            political_power_raw: None,
            command_power_raw: 0,
            stability_raw: 0,
            war_support_raw: 0,
            convoys_raw: None,
            peace_conference_participant: 0,
            manpower_residual: 0,
        };
        let links = BTreeMap::from([(11, &country), (12, &country)]);
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 2,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index]))
            })
            .map(Into::into),
        };
        *frame.tables[TableId::CountryExperience.index()] =
            TableView::Complete(TableRows::CountryExperience(vec![CountryExperienceRow {
                country: 12,
                army_raw: 1,
                army_daily_raw: 2,
                army_daily_training_raw: 3,
                navy_raw: 4,
                navy_daily_raw: 5,
                navy_daily_training_raw: 6,
                air_raw: 7,
                air_daily_raw: 8,
                armies_for_training_raw: 32_769,
            }]));
        let mut report = Report::default();
        compare_experience(&frame, &saved, &links, &mut report).unwrap();
        assert!(report.differences.is_empty());
        let TableView::Complete(TableRows::CountryExperience(rows)) =
            &mut *frame.tables[TableId::CountryExperience.index()]
        else {
            unreachable!()
        };
        let row = &mut rows[0];
        for value in [
            &mut row.army_raw,
            &mut row.army_daily_raw,
            &mut row.army_daily_training_raw,
            &mut row.navy_raw,
            &mut row.navy_daily_raw,
            &mut row.navy_daily_training_raw,
            &mut row.air_raw,
            &mut row.air_daily_raw,
            &mut row.armies_for_training_raw,
        ] {
            *value += 1;
        }
        let mut report = Report::default();
        compare_experience(&frame, &saved, &links, &mut report).unwrap();
        assert_eq!(report.differences.len(), 9);

        saved
            .countries
            .get_mut(b"GER".as_slice())
            .unwrap()
            .experience = Some(economy::Experience {
            army_raw: None,
            army_daily_raw: None,
            army_daily_training_raw: None,
            navy_raw: None,
            navy_daily_raw: None,
            navy_daily_training_raw: None,
            air_raw: None,
            air_daily_raw: None,
            armies_for_training_raw: 0,
        });
        *frame.tables[TableId::CountryExperience.index()] =
            TableView::Complete(TableRows::CountryExperience(vec![CountryExperienceRow {
                country: 12,
                army_raw: 0,
                army_daily_raw: 0,
                army_daily_training_raw: 0,
                navy_raw: 0,
                navy_daily_raw: 0,
                navy_daily_training_raw: 0,
                air_raw: 0,
                air_daily_raw: 0,
                armies_for_training_raw: 0,
            }]));
        let mut report = Report::default();
        compare_experience(&frame, &saved, &links, &mut report).unwrap();
        assert!(report.differences.is_empty());
        saved
            .countries
            .get_mut(b"GER".as_slice())
            .unwrap()
            .experience = None;
        let mut report = Report::default();
        compare_experience(&frame, &saved, &links, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
        *frame.tables[TableId::CountryExperience.index()] = TableView::Truncated {
            rows: TableRows::CountryExperience(Vec::new()),
            missing: 1,
        };
        assert!(compare_experience(&frame, &saved, &links, &mut Report::default()).is_err());
    }

    fn logistics_series(index: u32, current: &str) -> String {
        let mut data = Vec::new();
        for slot in 0..30 {
            data.push(match slot {
                11 => "7 7 7",
                12 => current,
                _ => "0 0 0",
            });
        }
        format!(
            "{index}={{ history_queue={{ 0={{ max_elements=30 offset=12 is_full=yes data={{ {} }} }} 1={{ max_elements=12 offset=1 is_full=yes }} 2={{ max_elements=7 offset=2 is_full=yes }} }} }}",
            data.join(" ")
        )
    }

    #[test]
    fn logistics_oracle_compares_the_current_daily_record_and_the_computed_countries() {
        use replay_data::rows::LogisticsEquipmentRow;
        let history = format!(
            "history={{ {} {} }}",
            logistics_series(15, "145075 0 2017.5"),
            logistics_series(16, "-112 0.00001 2017.5"),
        );
        let saved = economy::parse(
            &parser::parse(
                format!("countries={{GER={{logistics={{{history}}}}} ENG={{logistics={{}}}}}}")
                    .as_bytes(),
            )
            .unwrap()
            .items,
        )
        .unwrap();
        let country = |id, tag: &[u8]| CountryRow {
            border_land_doctrine_level: replay_data::rows::Recorded::Value(None),
            id,
            canonical_slot: id,
            tag: tag.to_vec(),
            political_power_raw: None,
            command_power_raw: 0,
            stability_raw: 0,
            war_support_raw: 0,
            convoys_raw: None,
            peace_conference_participant: 0,
            manpower_residual: 0,
        };
        let (germany, england) = (country(11, b"GER"), country(12, b"ENG"));
        let links = BTreeMap::from([(11, &germany), (12, &england)]);
        let row = |country, archetype, balance_raw, stock_raw| LogisticsEquipmentRow {
            country,
            archetype,
            token: 0x5000 + archetype,
            balance_raw,
            stock_raw,
            available_raw: replay_data::rows::Recorded::NotRecorded,
        };
        let matching = vec![
            row(11, 0, -11_200_000, 14_507_500_000),
            row(11, 1, 1, 0),
            row(11, 2, 201_750_000, 201_750_000),
        ];
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 2,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index]))
            })
            .map(Into::into),
        };
        let mut run = |rows: Vec<LogisticsEquipmentRow>| {
            *frame.tables[TableId::LogisticsEquipment.index()] =
                TableView::Complete(TableRows::LogisticsEquipment(rows));
            let mut report = Report::default();
            let result = compare_logistics(&frame, &saved, &links, &BTreeMap::new(), &mut report);
            (result.is_ok(), report)
        };
        let (ok, report) = run(matching.clone());
        assert!(ok);
        assert_eq!(report.differences, Vec::<String>::new());
        assert_eq!(report.compared_tables["LogisticsEquipment"], 3);
        assert!(
            report
                .limitations
                .keys()
                .any(|key| key.contains("does not name equipment archetypes"))
        );
        let mut observed = matching.clone();
        for row in &mut observed {
            row.available_raw = replay_data::rows::Recorded::Value(Some(0));
        }
        assert!(run(observed.clone()).1.differences.is_empty());
        observed[1].available_raw = replay_data::rows::Recorded::Value(None);
        let report = run(observed).1;
        assert!(report.differences.is_empty());
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("no matching native write"))
        );
        let mut changed = matching.clone();
        changed[1].balance_raw = 0;
        changed[2].stock_raw -= 1;
        assert_eq!(run(changed).1.differences.len(), 2);
        let mut extra = matching.clone();
        extra.push(row(12, 0, 0, 0));
        assert_eq!(run(extra).1.differences.len(), 1);
        assert_eq!(run(Vec::new()).1.differences.len(), 1);
        let (ok, report) = run(matching[..2].to_vec());
        assert!(ok);
        assert_eq!(report.differences.len(), 2);
        assert!(!run(matching[1..].to_vec()).0);
    }
}
