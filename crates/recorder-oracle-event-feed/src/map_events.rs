//! Map events are transient simulation edges; text saves hold none of them.
//! This is a recording self-check: each interval of the branch continues the
//! one before it or starts a new epoch with its coverage marked, events stay
//! in sequence, and the save hour's events join the recorded world.
use super::Report;
use replay::campaign::Replay;
use replay_data::{
    rows::MapEventFamily, rows::MapEventFault, rows::MapEventPayload, rows::MapEventsRow,
    rows::TableId, rows::tables,
};
use replay_frame::frame::DecodedFrame;
use std::{collections::BTreeSet, io};

pub fn compare(replay: &Replay, frame: &DecodedFrame, report: &mut Report) -> io::Result<()> {
    report.limitation(
        "recording self-check only: text saves retain no fake division reveals, railway gun overruns, critical shore bombardments, general captures, paradrop landings, resistance activities, building damage notifications or forced scripted map refreshes. Saved state cannot certify which happened, when, or complete capture; matched native observations and world/load, unwind and detach checks are required",
        "MapEvents",
    );
    let selected = [TableId::MapEvents];
    let mut blocks: Vec<_> = replay
        .index
        .iter()
        .filter(|block| block.branch == frame.branch && block.first_hour <= frame.hour)
        .collect();
    blocks.sort_by_key(|block| block.first_hour);
    let mut previous: Option<MapEventsRow> = None;
    let mut count = 0;
    for block in blocks {
        replay.visit_blocks(
            &[block],
            &selected,
            None,
            |_, _, hour_frame| {
            if hour_frame.hour > frame.hour {
                return Ok(false);
            }
            let decoded = Ok::<&replay_frame::frame::DecodedFrame, std::io::Error>(hour_frame)?;
            let Ok(rows) = decoded.complete::<tables::MapEvents>() else {
                if decoded.hour == frame.hour {
                    report.failed_comparisons.insert(
                        "MapEvents".into(),
                        format!("hour {} unavailable", decoded.hour),
                    );
                }
                previous = None;
                return Ok(true);
            };
            let [row] = rows else {
                report.difference(
                    "MapEvents.interval",
                    rows.len().to_string(),
                    "one hourly interval",
                );
                return Ok(true);
            };
            if let Err(reason) = observe(previous.as_ref(), row, decoded.hour) {
                report.difference(
                    format!("MapEvents hour {}", decoded.hour),
                    reason,
                    "a valid interval continuing the previous one or starting a new epoch",
                );
            }
            report_coverage(row, decoded.hour, frame.hour, report);
            count += row.events.len();
            previous = Some(row.clone());
                Ok(true)
            },
            || Ok(()),
        )?;
    }
    if let Some(row) = previous.as_ref().filter(|row| row.to_hour == frame.hour) {
        joins(frame, row, report)?;
    }
    report.compared("MapEvents", count);
    Ok(())
}

/// One interval against the one before it on the branch.
fn observe(previous: Option<&MapEventsRow>, row: &MapEventsRow, hour: i32) -> Result<(), String> {
    row.validate().map_err(|error| error.to_string())?;
    if row.to_hour != hour {
        return Err(format!("interval ends at hour {}", row.to_hour));
    }
    let Some(previous) = previous else {
        return Ok(());
    };
    if row.continues(previous) {
        return Ok(());
    }
    if row.epoch == previous.epoch {
        return Err(format!(
            "interval does not continue the previous one (sequences {}..{} after {}..{})",
            row.first_sequence, row.next_sequence, previous.first_sequence, previous.next_sequence
        ));
    }
    // A new epoch starts with every family's coverage unknown.
    if row
        .health
        .iter()
        .any(|health| health.installed && health.enabled && health.complete)
    {
        return Err("a new epoch's first interval claims complete coverage".into());
    }
    Ok(())
}

fn report_coverage(row: &MapEventsRow, hour: i32, save_hour: i32, report: &mut Report) {
    for family in MapEventFamily::ALL {
        let health = row.health(family);
        if health.complete {
            continue;
        }
        let faults = health
            .faults
            .iter()
            .map(MapEventFault::name)
            .collect::<Vec<_>>()
            .join(", ");
        let subject = format!("MapEvents {} hour {hour}: {faults}", family.name());
        report.limitation(
            "map event interval coverage is incomplete; the save retains no map events and cannot certify the unobserved interval",
            subject.clone(),
        );
        if hour == save_hour {
            report
                .failed_comparisons
                .insert(format!("MapEvents {}", family.name()), subject);
        }
    }
}

/// The save hour's events name provinces, countries and building types the
/// recording holds at that hour.
fn joins(frame: &DecodedFrame, row: &MapEventsRow, report: &mut Report) -> io::Result<()> {
    let provinces = frame
        .complete::<tables::Provinces>()?
        .iter()
        .map(|province| province.id)
        .collect::<BTreeSet<_>>();
    let countries = frame
        .complete::<tables::CountryLinks>()?
        .iter()
        .map(|link| link.id)
        .collect::<BTreeSet<_>>();
    let tokens = frame
        .complete::<tables::LexicalTokens>()?
        .iter()
        .map(|token| token.token)
        .collect::<BTreeSet<_>>();
    for event in &row.events {
        let mut province = |id: u32, field: &str| {
            if !provinces.contains(&id) {
                report.difference(
                    format!("MapEvents sequence {} {field}", event.sequence),
                    id.to_string(),
                    "a recorded province",
                );
            }
        };
        let mut checked = Vec::new();
        match &event.payload {
            MapEventPayload::FakeDivisionRevealed(unit)
            | MapEventPayload::RailwayGunOverrun(unit) => {
                province(unit.anchor_province, "anchor");
                province(unit.visibility_province, "visibility province");
                checked.push(("owner", unit.owner));
            }
            MapEventPayload::CriticalShoreBombardment(critical) => {
                province(critical.province, "province")
            }
            MapEventPayload::GeneralCaptured(capture) => {
                province(capture.province, "province");
                checked.extend([("owner", capture.owner), ("captor", capture.captor)]);
            }
            MapEventPayload::ParadropLanding(paradrop) => {
                province(paradrop.province, "province");
                checked.push(("country", paradrop.country));
            }
            MapEventPayload::ResistanceActivity(activity) => {
                province(activity.province, "province");
                checked.extend([
                    ("controller", activity.controller),
                    ("overlord", activity.overlord),
                ]);
            }
            MapEventPayload::ScriptedMapRefresh(_) => {}
            MapEventPayload::BuildingDamage(building) => {
                province(building.province, "province");
                checked.push(("controller", building.controller));
                if !tokens.contains(&building.building_type) {
                    report.difference(
                        format!("MapEvents sequence {} building type", event.sequence),
                        building.building_type.to_string(),
                        "a recorded lexical token",
                    );
                }
            }
        }
        // Zero is the reserved country's raw reference, which has no link.
        for (field, raw) in checked {
            if raw != 0 && !countries.contains(&raw) {
                report.difference(
                    format!("MapEvents sequence {} {field}", event.sequence),
                    raw.to_string(),
                    "a recorded country reference",
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::{rows::MapEventFamilyHealth, rows::MapEventFaults};

    fn row(epoch: u64, hours: (i32, i32), sequences: (u64, u64), complete: bool) -> MapEventsRow {
        let mut faults = MapEventFaults::default();
        if !complete {
            faults.insert(MapEventFault::ObservationStart);
        }
        MapEventsRow {
            epoch,
            from_hour: hours.0,
            to_hour: hours.1,
            first_sequence: sequences.0,
            next_sequence: sequences.1,
            health: [MapEventFamilyHealth {
                installed: true,
                enabled: true,
                complete,
                faults,
                missed: 0,
            }; MapEventFamily::COUNT],
            events: Vec::new(),
        }
    }

    #[test]
    fn intervals_continue_or_start_an_epoch_with_unknown_coverage() {
        let first = row(3, (9, 10), (5, 8), false);
        assert_eq!(observe(None, &first, 10), Ok(()));
        assert_eq!(
            observe(Some(&first), &row(3, (10, 11), (8, 8), true), 11),
            Ok(())
        );
        // A gap in the same epoch is a difference.
        assert!(observe(Some(&first), &row(3, (10, 11), (9, 10), true), 11).is_err());
        assert!(observe(Some(&first), &row(3, (11, 12), (8, 9), true), 12).is_err());
        // A new epoch must not claim complete coverage at its start.
        assert_eq!(
            observe(Some(&first), &row(4, (10, 11), (9, 9), false), 11),
            Ok(())
        );
        assert!(observe(Some(&first), &row(4, (10, 11), (9, 9), true), 11).is_err());
        // The interval must end at its frame's hour.
        assert!(observe(None, &first, 11).is_err());
    }

    #[test]
    fn incomplete_coverage_is_a_limitation_and_fails_only_the_save_hour() {
        let mut report = Report::default();
        let incomplete = row(1, (9, 10), (1, 1), false);
        report_coverage(&incomplete, 10, 11, &mut report);
        assert!(report.failed_comparisons.is_empty());
        assert!(!report.limitations.is_empty());
        report_coverage(&incomplete, 11, 11, &mut report);
        assert_eq!(report.failed_comparisons.len(), MapEventFamily::COUNT);
        let mut complete = Report::default();
        report_coverage(&row(1, (10, 11), (1, 1), true), 11, 11, &mut complete);
        assert!(complete.failed_comparisons.is_empty() && complete.limitations.is_empty());
    }
}
