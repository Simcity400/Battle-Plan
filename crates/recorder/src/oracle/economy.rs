//! Exact native economy comparisons with explicit save coverage limits.

use std::{collections::BTreeMap, io};

use installed_content::save::{Save, economy};
use replay::{
    frame::DecodedFrame,
    rows::{CountryRow, tables},
};

use super::{Report, TokenNames, complete, invalid, required_country_tag};

fn equal<T: std::fmt::Debug + PartialEq>(
    report: &mut Report,
    label: impl AsRef<str>,
    actual: T,
    expected: T,
) {
    if actual != expected {
        report.difference(label, actual, expected);
    }
}

fn compare_text(report: &mut Report, label: &str, actual: &[u8], saved: &[u8]) {
    // Both CAF770's resource name and 1200E10's foreign source use
    // 1424ADF40 -> 1424AF5A0: an MSVC c_str, quoted even when empty,
    // scanned only through the first NUL in text mode.
    let end = actual
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(actual.len());
    equal(report, label, &actual[..end], saved);
    if end < actual.len() {
        report.limitation(
            "native text string serialization stops at the first NUL; only the string prefix is comparable and the captured NUL/suffix remain uncheckable",
            label,
        );
    }
}

pub(super) fn compare(
    frame: &DecodedFrame,
    saved: &Save,
    links: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let mut all_complete = report.attempt("economy CountryFuel", |report| {
        compare_fuel(frame, &saved.economy, links, report)
    });
    all_complete &= report.attempt("economy CountryFuelConsumers", |report| {
        compare_fuel_consumers(frame, &saved.economy, links, report)
    });
    all_complete &= report.attempt("economy CountryExperience", |report| {
        compare_experience(frame, &saved.economy, links, report)
    });
    all_complete &= report.attempt("economy PurchaseContracts", |report| {
        compare_purchases(frame, &saved.economy, links, report)
    });
    all_complete &= report.attempt("economy ResourceExchanges", |report| {
        compare_exchanges(frame, &saved.economy, links, tokens, report)
    });
    all_complete &= report.attempt("economy HubMotorization", |report| {
        compare_motorization(frame, &saved.economy, links, report)
    });
    all_complete &= report.attempt("economy DisruptedSupply", |report| {
        compare_disrupted(frame, &saved.economy, links, report)
    });
    all_complete &= report.attempt("economy ForeignHomebases", |report| {
        compare_foreign(frame, &saved.economy, links, report)
    });
    // These tables still require complete shared decoding, even where the
    // text save has no proven direct representation of their raw operands.
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
        (
            "CountryResources",
            complete::<tables::CountryResources>(frame).map(<[_]>::len),
            "the nine native country resource pool operands have no established exact save mapping; displayed production, use and trade totals are not substituted for the separate prefix-sum allocation pools",
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
    report.limitation("economy save country tags compare resolved identity and cannot certify equivalent raw country-link aliases", "CountryFuel, CountryFuelConsumers, CountryExperience, ResourceExchanges, PurchaseContracts, HubMotorization, DisruptedSupply, ForeignHomebases");
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
    for (tag, country) in &saved.countries {
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
        // 141294210 compares these eight qwords against converted zero and
        // omits precisely equal values. Missing the whole status is handled
        // above, and its unconditional ninth field is required by the parser.
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
    for (tag, country) in &saved.countries {
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
            // CAF770 omits receiver only for a nonpositive raw importer and
            // resource only for a null descriptor pointer. Complete captured
            // rows require resolved importers and positive database descriptors.
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
            equal(
                report,
                format!("{label}.convoys_required_nonzero"),
                actual.convoys_required != 0,
                expected.convoy_subscriber_present,
            );
        }
    }
    for id in actual.keys() {
        report.difference(format!("resource exchange {id:#018x}"), "present", "absent");
    }
    report.compared("ResourceExchanges", count);
    report.limitation("resource exchange no_trade_influence/no_resources bytes and directional native vector ordinals are not directly serialized; convoy required zero/nonzero is checked by native subscriber omission, but nonzero required counts and assigned counts have no established direct mapping", "ResourceExchanges");
    report.limitation("save resource names and country tags cannot certify equivalent raw numeric token/resource identities or country-link alias bits", "ResourceExchanges");
    Ok(())
}

fn compare_motorization(
    frame: &DecodedFrame,
    saved: &economy::Pack,
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
        report.difference(
            format!(
                "{}.motorization[{node}/{entry}]",
                String::from_utf8_lossy(tag)
            ),
            *actual,
            "absent",
        );
    }
    report.compared("HubMotorization", setting_count);
    report.limitation("hub rows capture explicit actor settings, not derived effective motorization or disabled-for-allies; save country tags cannot certify equivalent raw actor alias bits", "HubMotorization");
    Ok(())
}

fn compare_disrupted(
    frame: &DecodedFrame,
    saved: &economy::Pack,
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
        report.difference(
            format!(
                "{}.disrupted_supply[{province}/{node_type}]",
                String::from_utf8_lossy(tag)
            ),
            "present",
            "absent",
        );
    }
    report.compared("DisruptedSupply", disrupted_count);
    Ok(())
}

fn compare_foreign(
    frame: &DecodedFrame,
    saved: &economy::Pack,
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
        report.difference(
            format!(
                "{}.foreign_homebase[{ordinal}]",
                String::from_utf8_lossy(tag)
            ),
            "present",
            "absent",
        );
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
    for (tag, country) in &saved.countries {
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
            ("gain_raw", actual.gain_raw, expected.gain.raw),
            ("cost_raw", actual.cost_raw, expected.cost.raw),
            (
                "gain_from_states_raw",
                actual.gain_from_states_raw,
                expected.gain_from_states.raw,
            ),
            (
                "lend_lease_gain_raw",
                actual.lend_lease_gain_raw,
                expected.lend_lease_gain.raw()?,
            ),
            (
                "lend_lease_consumption_raw",
                actual.lend_lease_consumption_raw,
                expected.lend_lease_consumption.raw()?,
            ),
            (
                "gain_per_oil_raw",
                actual.gain_per_oil_raw,
                expected.gain_per_oil.raw,
            ),
        ] {
            equal(report, format!("{label}.{name}"), actual, expected);
        }
        for (name, actual, expected) in [
            (
                "history_index",
                actual.history_index,
                expected.history_index,
            ),
            (
                "remaining_hours",
                actual.remaining_hours,
                expected.remaining_hours,
            ),
        ] {
            equal(report, format!("{label}.{name}"), actual, expected);
        }
        // The ring sums cover the 24 ring slots, never the pending entry.
        let mut sums = [0_i128; 9];
        for entry in expected.history.iter().filter(|entry| entry.index != -1) {
            let values = [
                entry.gain.raw,
                entry.consumed.raw()?,
                entry.other.raw()?,
                entry.requested[0].raw,
                entry.requested[1].raw,
                entry.requested[2].raw,
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
            ("ring_requested_0_raw", actual.ring_requested_0_raw),
            ("ring_requested_1_raw", actual.ring_requested_1_raw),
            ("ring_requested_2_raw", actual.ring_requested_2_raw),
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
        "fuel quantities compare exact /32768 bits through the injective native five-decimal formatter; native i64::MIN has malformed text serialization and is refused; gain/cost are separate /100000 modifiers, not total income/consumption",
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

/// Consumer entries by country and array position, the saved `index`.
/// A country without a saved `fuel_status` is reported by CountryFuel; its
/// recorded consumers are listed here as unmatched.
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
    for (tag, country) in &saved.countries {
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
    use installed_content::parser;
    use replay::{
        frame::{Metrics, TableView},
        rows::{
            CountryExperienceRow, CountryFuelConsumerRow, CountryFuelRow, HubMotorizationRow,
            LexicalTokenRow, ResourceDefinitionRow, ResourceExchangeRow, TableId, TableRows,
        },
    };

    /// A `fuel_status` body in the writer's layout (1410E39B0): the scalars,
    /// three consumers, the pending entry and 24 ring entries. Ring entry `i`
    /// holds fuel and other of `i + 1` native units and otherwise the values
    /// of `RING_SUMS / 24`; the pending entry holds values of its own that the
    /// sums must not include.
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

    /// The raw sums of `fuel_status`'s 24 ring entries, in CountryFuel's
    /// column order; 265.14755 is the formatter's text for 8_688_355 units.
    const RING_SUMS: [i64; 9] = [
        24 * 39_040_000,
        24 * 8_688_355,
        300,
        24 * 4_085_669,
        24 * 814_590,
        24 * 21_614_500,
        24 * 4_085_669,
        24 * 814_590,
        -24,
    ];

    fn with_ring_sums(row: CountryFuelRow) -> CountryFuelRow {
        let [
            gain,
            consumed,
            other,
            requested_0,
            requested_1,
            requested_2,
            received_0,
            received_1,
            received_2,
        ] = RING_SUMS;
        CountryFuelRow {
            ring_gain_raw: gain,
            ring_consumed_raw: consumed,
            ring_other_raw: other,
            ring_requested_0_raw: requested_0,
            ring_requested_1_raw: requested_1,
            ring_requested_2_raw: requested_2,
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
        let saved = installed_content::save::parse(
            br#"HOI4txt
            date=1942.9.6.2 game_unique_id=test version=test game_unique_seed=1 debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 theater_group_index=6 military_deployment_line_index=7 military_deployment_conveyor_index=8 equipment_variant_index=9 country_leader_index=10 navy_id=11 land_combat_id=12 multiplayer_random_count=-13
            countries={} states={} provinces={}
            equipment_market={contracts={}} supply_system_2={countries={}}
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
            "CountryResources",
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
        compare_motorization(&frame, &saved, &links, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
        let difference = &report.differences[0];
        assert!(difference.contains("SIK.motorization[1/0]"));
        assert!(difference.contains("province: 4709"));
        assert!(difference.contains("actor: 11"));
        assert!(difference.contains("level: 0"));
        assert_eq!(report.compared_tables["HubMotorization"], 0);
    }

    #[test]
    fn exchange_without_lexical_names_still_checks_numeric_fields_and_table_completeness() {
        let saved = economy::parse(
            &parser::parse(
                br#"countries={GER={resources={export={
            id={id=1 type=4713} receiver=GER resource=steel efficiency=1
            efficiency_due_to_lost_convoys=0 delivered=-0.00001 required_cic=2 lended_cic=1
        }}}}"#,
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

    fn fuel_row(frame: &mut DecodedFrame) -> &mut CountryFuelRow {
        let TableView::Complete(TableRows::CountryFuel(rows)) =
            &mut *frame.tables[TableId::CountryFuel.index()]
        else {
            unreachable!()
        };
        &mut rows[0]
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
            id: 11,
            canonical_slot: 7,
            tag: b"GER".to_vec(),
            political_power_raw: None,
            command_power_raw: 0,
            stability_raw: 0,
            war_support_raw: 0,
            convoys_raw: None,
            peace_conference_participant: 0,
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
                gain_raw: -1,
                cost_raw: 0,
                gain_from_states_raw: 6_440_000,
                lend_lease_gain_raw: 1,
                lend_lease_consumption_raw: -1,
                gain_per_oil_raw: 360_000,
                history_index: 13,
                remaining_hours: 1,
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
        for change in 0..15 {
            let TableView::Complete(TableRows::CountryFuel(rows)) =
                &mut *frame.tables[TableId::CountryFuel.index()]
            else {
                unreachable!()
            };
            let mut row = rows[0].clone();
            match change {
                0 => row.gain_from_states_raw += 1,
                1 => row.lend_lease_gain_raw += 1,
                2 => row.lend_lease_consumption_raw -= 1,
                3 => row.gain_per_oil_raw -= 1,
                4 => row.history_index = 12,
                5 => row.remaining_hours = 0,
                6 => row.ring_gain_raw += 1,
                7 => row.ring_consumed_raw -= 1,
                8 => row.ring_other_raw += 1,
                9 => row.ring_requested_0_raw += 1,
                10 => row.ring_requested_1_raw += 1,
                11 => row.ring_requested_2_raw += 1,
                12 => row.ring_received_0_raw += 1,
                13 => row.ring_received_1_raw += 1,
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
        // Any one ring slot's saved value moves its sum; the pending entry's
        // values move nothing.
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

        // Every consumer field is compared.
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
        // A missing or extra consumer is a difference, not a skipped row.
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
            id: 11,
            canonical_slot: 7,
            tag: b"GER".to_vec(),
            political_power_raw: None,
            command_power_raw: 0,
            stability_raw: 0,
            war_support_raw: 0,
            convoys_raw: None,
            peace_conference_participant: 0,
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
}
