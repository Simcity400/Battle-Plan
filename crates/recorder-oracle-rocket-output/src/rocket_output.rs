use super::{Report, complete, daily_read, invalid};
use replay::campaign::Replay;
use replay_data::{rows::RocketProductionOutputRow, rows::RowCodec, rows::TableId, rows::tables};
use replay_frame::frame::DecodedFrame;
use std::{collections::BTreeMap, io};

pub fn compare(replay: &Replay, frame: &DecodedFrame, report: &mut Report) -> io::Result<()> {
    report.limitation(
        "rocket ready counts and current progress in a save cannot certify historical IC or deliveries; this self-check verifies recorded continuity only",
        "RocketProductionOutput",
    );
    complete::<tables::RocketProductionOutput>(frame)?;
    let selected = [TableId::RocketProductionOutput];
    let mut check = Continuity {
        event_losses: replay.event_losses(frame.branch).collect(),
        ..Continuity::default()
    };
    for block in replay
        .index
        .iter()
        .filter(|block| block.branch == frame.branch && block.first_hour <= frame.hour)
    {
        replay.visit_blocks(
            &[block],
            &selected,
            |_, _, hour_frame| {
            if hour_frame.hour > frame.hour {
                return Ok(false);
            }
            check.observe(Ok::<&replay_frame::frame::DecodedFrame, std::io::Error>(hour_frame)?)?;
                Ok(true)
            },
            || Ok(()),
        )?;
    }
    report.compared("RocketProductionOutput continuity", check.compared);
    Ok(())
}

#[derive(Default)]
struct Continuity {
    previous: BTreeMap<(u32, u64), RocketProductionOutputRow>,
    read: Option<i32>,
    event_losses: Vec<(i32, i32)>,
    compared: usize,
}

impl Continuity {
    fn observe(&mut self, frame: &DecodedFrame) -> io::Result<()> {
        if self.read.is_some_and(|read| {
            self.event_losses
                .iter()
                .any(|&(_, lost)| read < lost && lost <= frame.hour)
        }) {
            self.previous.clear();
        }
        if !daily_read(&frame.metrics, TableId::RocketProductionOutput) {
            return Ok(());
        }
        self.read = Some(frame.hour);
        let Ok(rows) = complete::<tables::RocketProductionOutput>(frame) else {
            self.previous.clear();
            return Ok(());
        };
        check_rows(&mut self.previous, rows)?;
        self.compared += rows.len();
        Ok(())
    }
}

fn check_rows(
    previous: &mut BTreeMap<(u32, u64), RocketProductionOutputRow>,
    rows: &[RocketProductionOutputRow],
) -> io::Result<()> {
    let world = rows
        .iter()
        .find(|row| (row.country, row.variant) == (0, 0))
        .ok_or_else(|| invalid("rocket output lacks world coverage"))?;
    let mut current = BTreeMap::new();
    for row in rows {
        row.check_key_fields()?;
        if row.since_hour != world.since_hour {
            return Err(invalid(
                "rocket output contains different observation starts",
            ));
        }
        let key = (row.country, row.variant);
        if current.insert(key, row.clone()).is_some() {
            return Err(invalid("rocket output repeats a country/design"));
        }
        if let Some(before) = previous.get(&key) {
            check_change(before, row)?;
        }
    }
    for row in current.values() {
        if row.country != 0 && row.variant != 0 && !current.contains_key(&(row.country, 0)) {
            return Err(invalid("rocket design lacks country coverage"));
        }
    }
    if previous.keys().any(|key| !current.contains_key(key)) {
        return Err(invalid("retained rocket output disappeared"));
    }
    *previous = current;
    Ok(())
}

fn check_change(
    before: &RocketProductionOutputRow,
    row: &RocketProductionOutputRow,
) -> io::Result<()> {
    if row.since_hour != before.since_hour
        || row.ticks < before.ticks
        || row.inexact_ticks < before.inexact_ticks
        || row.unknown_design_ticks < before.unknown_design_ticks
        || row.unknown_country_ticks < before.unknown_country_ticks
        || row.deliveries < before.deliveries
        || row.inexact_deliveries < before.inexact_deliveries
        || row.exact && !before.exact
        || row.units_exact && !before.units_exact
        || row.ticks == before.ticks
            && (row.spent_raw != before.spent_raw || row.instant_raw != before.instant_raw)
        || row.deliveries == before.deliveries && row.units != before.units
    {
        return Err(invalid(
            "rocket output changed without its recorded event or recovered exactness",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn design() -> RocketProductionOutputRow {
        RocketProductionOutputRow {
            country: 5001,
            variant: 42,
            spent_raw: 3,
            units: 7,
            ticks: 1,
            deliveries: 1,
            exact: true,
            units_exact: true,
            ..Default::default()
        }
    }

    #[test]
    fn signed_rocket_changes_require_independent_events_and_keep_refusals() {
        let before = design();
        let mut next = before.clone();
        next.spent_raw = -4;
        assert!(check_change(&before, &next).is_err());
        next.ticks += 1;
        assert!(check_change(&before, &next).is_ok());
        next.units = -2;
        assert!(check_change(&before, &next).is_err());
        next.deliveries += 1;
        assert!(check_change(&before, &next).is_ok());
        let refused = RocketProductionOutputRow {
            exact: false,
            ..before
        };
        assert!(check_change(&refused, &before).is_err());
    }

    #[test]
    fn rocket_self_check_refuses_missing_scopes_and_retained_designs() {
        let design = design();
        let world = RocketProductionOutputRow {
            country: 0,
            variant: 0,
            ..Default::default()
        };
        let scope = RocketProductionOutputRow {
            country: 5001,
            ..world
        };
        let mut previous = BTreeMap::new();
        assert!(check_rows(&mut previous, &[]).is_err());
        assert!(check_rows(&mut previous, &[world.clone(), design.clone()]).is_err());
        check_rows(&mut previous, &[world.clone(), scope.clone(), design]).unwrap();
        assert!(check_rows(&mut previous, &[world, scope]).is_err());
    }

    #[test]
    fn a_recorded_event_loss_restarts_rocket_continuity_and_nothing_else_does() {
        let rows = |since_hour: i32, ticks: u64| {
            let scope = |country| RocketProductionOutputRow {
                country,
                since_hour,
                ..Default::default()
            };
            let design = RocketProductionOutputRow {
                since_hour,
                ticks,
                ..design()
            };
            vec![scope(0), scope(5001), design]
        };
        let frame = |hour: i32, read: Option<Vec<RocketProductionOutputRow>>| {
            let mut frame = crate::test_support::frame(
                read.clone()
                    .map(replay_data::rows::TableRows::RocketProductionOutput),
            );
            frame.hour = hour;
            if read.is_some() {
                frame.metrics.table_ns[TableId::RocketProductionOutput.index()] = 1;
            }
            frame
        };
        let frames = [
            frame(100, Some(rows(90, 3))),
            frame(101, None),
            frame(102, Some(rows(101, 1))),
        ];
        let run = |event_losses: Vec<(i32, i32)>| {
            let mut check = Continuity {
                event_losses,
                ..Continuity::default()
            };
            for frame in &frames {
                check.observe(frame)?;
            }
            Ok::<_, io::Error>(check.compared)
        };
        assert!(run(Vec::new()).is_err());
        assert!(run(vec![(102, 103)]).is_err());
        assert_eq!(run(vec![(100, 101)]).unwrap(), 6);
    }

    #[test]
    fn a_refresh_off_the_slot_is_checked_where_the_rocket_output_is_marked_read() {
        use replay_capture_spool::spool::Record;
        use replay_data::rows::TableRows;
        use replay_frame::frame::TableUpdate;
        let scope = |country| RocketProductionOutputRow {
            country,
            since_hour: 2,
            ..Default::default()
        };
        let rows = vec![
            scope(0),
            scope(5001),
            RocketProductionOutputRow {
                since_hour: 2,
                ..design()
            },
        ];
        let mut records = crate::test_support::allocation_records();
        for record in &mut records {
            if let Record::Frame(frame) = record {
                assert!(!replay_data::cadence::is_refresh(
                    TableId::RocketProductionOutput,
                    frame.hour
                ));
                frame.tables[TableId::RocketProductionOutput.index()] =
                    TableUpdate::Snapshot(TableRows::RocketProductionOutput(rows.clone()));
                frame.metrics.table_ns[TableId::ProductionLines.index()] = 1;
                if frame.hour != 3 {
                    frame.metrics.table_ns[TableId::RocketProductionOutput.index()] = 1;
                }
            }
        }
        let replay = crate::test_support::replay(records);
        let mut frame = crate::test_support::frame([TableRows::RocketProductionOutput(rows)]);
        (frame.branch, frame.hour) = (9, 4);
        let mut report = Report::default();
        compare(&replay, &frame, &mut report).unwrap();
        assert_eq!(
            report
                .compared_tables
                .get("RocketProductionOutput continuity")
                .copied(),
            Some(6)
        );
    }
}
