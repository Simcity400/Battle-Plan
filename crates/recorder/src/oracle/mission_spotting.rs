//! Original transport receiver cache comparisons. FA9900 saves +20/+28 only
//! when positive; nonpositive magnitude and nullable ledger provenance need
//! independent native witnesses, rather than an invented save value.

use std::io;

use installed_content::save::{Decimal, SaveDocument, naval_missions::MissionSpottingCaches};
use replay::{
    frame::DecodedFrame,
    rows::{TransportMissionInputs, TransportMissionReceiver, tables},
    transport_mission_view::{self, MissionView},
};

use super::{Report, country_emissions, invalid, lexical_tokens};

const CACHE_TOKENS: [u32; 2] = [0x2fcd, 0x302f];
const FIELD: &str = "ReservedCountryNaming.transport_mission_inputs";

pub(super) fn compare(
    frame: &DecodedFrame,
    document: &SaveDocument,
    report: &mut Report,
) -> io::Result<()> {
    let rows = frame.complete::<tables::ReservedCountryNaming>()?;
    let [row] = rows else {
        return Err(invalid(
            "transport mission inputs require one reserved naming row",
        ));
    };
    let Some(packet) = report.recorded(&row.transport_mission_inputs, FIELD, FIELD) else {
        return Ok(());
    };
    let Some(packet) = packet else {
        report.difference(
            FIELD,
            "current packet refused",
            "current stored observations required",
        );
        return Ok(());
    };
    report.scalar_checks += 1;
    if packet.sample_hour != frame.hour {
        report.difference(
            format!("{FIELD}.sample_hour"),
            packet.sample_hour,
            frame.hour,
        );
        return Ok(());
    }
    let view = MissionView::new(&row.transport_mission_inputs, frame.hour).map_err(invalid)?;
    compare_runtime_domains(packet, frame.hour, report);
    match view.physical_navies() {
        Err(reason) => report.difference(
            format!("{FIELD}.navies"),
            reason,
            "complete original physical domain",
        ),
        Ok(navies) => {
            for navy in navies.iter().filter(|navy| navy.present) {
                let label = format!("{FIELD}.navies[{}]", navy.slot);
                if navy.region_slots.is_none() {
                    report.difference(
                        format!("{label}.region_slots"),
                        "read refused",
                        "original region count",
                    );
                }
                let Some(ledgers) = &navy.ledgers else {
                    report.difference(
                        format!("{label}.ledgers"),
                        "read refused",
                        "original sparse ledger domain",
                    );
                    continue;
                };
                for ledger in ledgers {
                    let label = format!("{label}.ledgers[{}]", ledger.region);
                    let Some(occurrences) = &ledger.occurrences else {
                        report.difference(
                            &label,
                            "read refused",
                            "complete ordered nullable occurrences",
                        );
                        continue;
                    };
                    for &handle in occurrences.iter().filter(|&&handle| handle != 0) {
                        if let Err(reason) = view.receiver(handle) {
                            report.difference(
                                format!("{label}.receiver {handle}"),
                                reason,
                                "original authentic receiver",
                            );
                        }
                    }
                }
            }
        }
    }
    report.limitation(
        "physical navy domain, original sparse region count, nullable ledger order/repeats, receiver identity and world/hour/quiescence association are not saved; matching task-force mission fields cannot certify them",
        FIELD,
    );
    let receivers = match view.receivers() {
        Ok(receivers) => receivers,
        Err(reason) => {
            report.difference(
                format!("{FIELD}.receivers"),
                reason,
                "complete original receiver dictionary",
            );
            return Ok(());
        }
    };
    // Check original current refusals before asking for token spellings. An
    // unavailable content dictionary must not hide independent read failures.
    for receiver in receivers {
        let label = format!("{FIELD}.receivers[{}]", receiver.task_force);
        for (name, missing) in transport_mission_view::RISK_REFUSAL_NAMES
            .into_iter()
            .zip(transport_mission_view::risk_refusals(receiver))
        {
            if missing {
                report.difference(
                    format!("{label}.{name}"),
                    "read refused",
                    "required original capture operand",
                );
            }
        }
        match transport_mission_view::demanded(receiver) {
            Ok(false) => {}
            Err(reason) => report.difference(
                format!("{label}.demand"),
                reason,
                "original native gate operands",
            ),
            Ok(true) => {
                for (name, missing) in [
                    ("logical_owner", receiver.logical_owner.is_none()),
                    ("combined_detection", receiver.combined_detection.is_none()),
                    ("detection_cache", receiver.detection_cache.is_none()),
                    (
                        "received_supply_ratio",
                        receiver.received_supply_ratio.is_none(),
                    ),
                ] {
                    if missing {
                        report.difference(
                            format!("{label}.{name}"),
                            "read refused",
                            "required reached native operand",
                        );
                    }
                }
            }
        }
    }
    report.limitation(
        "original mission owner, contributor and receiver stats, current common modifiers, speed selectors and location context are runtime operands; saved task-force fields do not certify the complete spotting, damage or writer graph",
        FIELD,
    );
    // A genuinely empty receiver dictionary does not need save-key content.
    if !receivers
        .iter()
        .any(|receiver| receiver.detection_cache.is_some())
    {
        return Ok(());
    }
    let tokens = lexical_tokens(frame, report)?;
    let keys = CACHE_TOKENS.map(|token| tokens.get(&token).copied());
    let [Some(radar), Some(air)] = keys else {
        report.limitation(
            "mission spotting save keys lack recorded lexical bindings; no static token spelling is inferred",
            format!("{FIELD} tokens {CACHE_TOKENS:?}"),
        );
        return Ok(());
    };
    if radar.is_empty() || air.is_empty() || radar == air {
        return Err(invalid(
            "recorded mission spotting save keys are empty or aliased",
        ));
    }
    let saved = country_emissions::mission_spotting_caches(document, frame, [radar, air])?;
    for receiver in receivers {
        compare_receiver(receiver, &saved, report);
    }
    report.compared(
        "ReservedCountryNaming transport mission caches",
        receivers.len(),
    );
    report.limitation(
        "combined NAVAL_DETECTION and raw received supply have no text-save counterpart; matching cache words cannot certify their producer, cache writer lifetime or full transport risk",
        FIELD,
    );
    Ok(())
}

/// These runtime-only graphs have no complete text-save counterpart. Their
/// own current-hour admission is still checked; save equality cannot mask a
/// refused capture or certify native pointer/writer exclusion.
fn compare_runtime_domains(packet: &TransportMissionInputs, hour: i32, report: &mut Report) {
    let context = packet.spotting_context.as_ref();
    let air = packet.air.as_ref();
    let ships = packet.risk_ships.as_ref();
    for (name, sample) in [
        ("spotting_context", context.map(|value| value.sample_hour)),
        ("air", air.map(|value| value.sample_hour)),
        ("risk_ships", ships.map(|value| value.sample_hour)),
    ] {
        report.scalar_checks += 1;
        if sample != Some(hour) {
            report.difference(format!("{FIELD}.{name}.sample_hour"), sample, Some(hour));
        }
    }
    if let Some(context) = context {
        for (name, complete) in [
            ("owners", context.owners_complete),
            ("regions", context.regions_complete),
        ] {
            if !complete {
                report.difference(
                    format!("{FIELD}.spotting_context.{name}"),
                    "incomplete",
                    "complete original domain",
                );
            }
        }
        if context.owners.iter().any(|owner| owner.owner_slot == 0)
            || context
                .owners
                .windows(2)
                .any(|rows| rows[0].owner_slot >= rows[1].owner_slot)
            || context
                .regions
                .windows(2)
                .any(|rows| rows[0].region_id >= rows[1].region_id)
        {
            report.difference(
                format!("{FIELD}.spotting_context.keys"),
                "malformed",
                "unique increasing native keys",
            );
        }
    }
    if let Some(air) = air {
        match &air.countries {
            None => report.difference(
                format!("{FIELD}.air.countries"),
                "read refused",
                "complete physical air domain",
            ),
            Some(countries) => {
                for (slot, country) in countries.iter().enumerate() {
                    if country.slot as usize != slot {
                        report.difference(
                            format!("{FIELD}.air.country[{slot}].slot"),
                            country.slot,
                            slot,
                        );
                    }
                    if !country.present {
                        continue;
                    }
                    let label = format!("{FIELD}.air.country[{slot}]");
                    if country.country.is_none() || country.factors.is_none() {
                        report.difference(
                            &label,
                            "original owner/factor observation refused",
                            "original independent observations",
                        );
                    }
                    let Some(regions) = &country.regions else {
                        report.difference(
                            format!("{label}.regions"),
                            "read refused",
                            "complete original region headers",
                        );
                        continue;
                    };
                    for (ordinal, region) in regions.iter().enumerate() {
                        if region.region as usize != ordinal
                            || region.count.is_none()
                            || region.count.is_some_and(|count| count < 0)
                            || region.missions.as_ref().map(Vec::len)
                                != region.count.and_then(|count| usize::try_from(count).ok())
                        {
                            report.difference(
                                format!("{label}.region[{ordinal}]"),
                                "incomplete or unsupported",
                                "exact original signed header and occurrences",
                            );
                        }
                    }
                }
            }
        }
    }
    if let Some(ships) = ships {
        match &ships.receivers {
            None => report.difference(
                format!("{FIELD}.risk_ships.receivers"),
                "read refused",
                "complete original dictionary",
            ),
            Some(receivers) => {
                if receivers
                    .windows(2)
                    .any(|rows| rows[0].task_force >= rows[1].task_force)
                {
                    report.difference(
                        format!("{FIELD}.risk_ships.keys"),
                        "malformed",
                        "unique increasing full handles",
                    );
                }
                for receiver in receivers {
                    let count = receiver.count.and_then(|count| usize::try_from(count).ok());
                    if count.is_none() || receiver.ships.as_ref().map(Vec::len) != count {
                        report.difference(
                            format!("{FIELD}.risk_ships.receiver[{}]", receiver.task_force),
                            "incomplete or unsupported",
                            "exact original SHIPS header and occurrences",
                        );
                    }
                }
            }
        }
    }
    report.limitation(
        "original radar/intel/weather context, strategic-air roots, ordered air/ace operands and original fading/repeated SHIPS observations have no complete save counterpart; admission checks do not certify the nested producer or native writer lifetime",
        FIELD,
    );
}

fn compare_receiver(
    receiver: &TransportMissionReceiver,
    saved: &MissionSpottingCaches,
    report: &mut Report,
) {
    let Some(cache) = receiver.detection_cache else {
        return;
    };
    let label = format!("{FIELD}.receivers[{}].detection_cache", receiver.task_force);
    let Some(values) = saved.get(&receiver.task_force) else {
        report.difference(
            &label,
            "recorded full task-force handle",
            "absent from projected save",
        );
        return;
    };
    for ((name, actual), saved) in ["radar", "air_superiority"]
        .into_iter()
        .zip(cache)
        .zip(values)
    {
        let label = format!("{label}.{name}");
        report.scalar_checks += 1;
        if !matches_saved(actual, saved.as_ref()) {
            report.difference(&label, actual, saved.as_ref().map(|value| value.raw));
        } else if saved.is_none() {
            report.limitation(
                "FA9900 emits mission spotting cache values only when positive; omission proves nonpositive but not zero or exact negative magnitude",
                label,
            );
        }
    }
}

fn matches_saved(actual: i64, saved: Option<&Decimal>) -> bool {
    if actual > 0 {
        saved.is_some_and(|saved| saved.raw == actual)
    } else {
        saved.is_none()
    }
}

#[cfg(test)]
#[path = "mission_spotting_tests.rs"]
mod tests;
