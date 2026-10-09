//! Retained rocket output has no historical spending/delivery serializer.
//! Check the recorded event counters and independent signed totals along a branch.
use super::{Report, complete, daily_read, invalid};
use replay::{
    container::Replay,
    frame::{DecodedFrame, FrameDecoder},
    rows::{RocketProductionOutputRow, RowCodec, TableId, tables},
};
use std::{collections::BTreeMap, io};

pub(super) fn compare(
    replay: &Replay,
    frame: &DecodedFrame,
    report: &mut Report,
) -> io::Result<()> {
    report.limitation(
        "rocket ready counts and current progress in a save cannot certify historical IC or deliveries; this self-check verifies recorded continuity only",
        "RocketProductionOutput",
    );
    complete::<tables::RocketProductionOutput>(frame)?;
    let selected = [TableId::RocketProductionOutput];
    let mut previous = BTreeMap::new();
    let mut compared = 0;
    for block in replay
        .index
        .iter()
        .filter(|block| block.branch == frame.branch && block.first_hour <= frame.hour)
    {
        let mut decoder = FrameDecoder::for_tables(&selected);
        for encoded in replay.read_block_selected(block, &selected)? {
            if encoded.hour > frame.hour {
                break;
            }
            let decoded = decoder.advance(&encoded)?;
            if !daily_read(
                decoded.hour,
                &decoded.metrics,
                TableId::RocketProductionOutput,
            ) {
                continue;
            }
            let Ok(rows) = complete::<tables::RocketProductionOutput>(decoded) else {
                previous.clear();
                continue;
            };
            check_rows(&mut previous, rows)?;
            compared += rows.len();
        }
    }
    report.compared("RocketProductionOutput continuity", compared);
    Ok(())
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
            ..before.clone()
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
            ..world.clone()
        };
        let mut previous = BTreeMap::new();
        assert!(check_rows(&mut previous, &[]).is_err());
        assert!(check_rows(&mut previous, &[world.clone(), design.clone()]).is_err());
        check_rows(&mut previous, &[world.clone(), scope.clone(), design]).unwrap();
        assert!(check_rows(&mut previous, &[world, scope]).is_err());
    }
}
