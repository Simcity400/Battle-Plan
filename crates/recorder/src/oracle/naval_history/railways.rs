use std::{collections::BTreeMap, io};

use installed_content::save::naval_history as saved;
use replay::{frame::DecodedFrame, rows::tables};

use super::{Report, complete, equal, invalid};

pub(super) fn compare_railways(
    frame: &DecodedFrame,
    saved: &saved::Pack,
    report: &mut Report,
) -> io::Result<()> {
    let expected = saved
        .railways
        .as_ref()
        .ok_or_else(|| invalid("save lacks railway manager"))?;
    let mut rows = BTreeMap::new();
    for row in complete::<tables::RailCooldowns>(frame)? {
        if rows.insert(row.province, row).is_some() {
            return Err(invalid("duplicate recorded railway province"));
        }
    }
    let mut compared = 0;
    for (&province, expected) in expected {
        let label = format!("railway province {province}");
        let Some(actual) = rows.remove(&province) else {
            report.difference(label, "absent", "present");
            continue;
        };
        compared += 1;
        if let Some(cooldown) = expected.cooldown {
            equal(
                report,
                format!("{label}.cooldown"),
                actual.cooldown_ticks,
                cooldown,
            );
        } else if actual.cooldown_ticks > 0 {
            report.difference(
                format!("{label}.cooldown"),
                actual.cooldown_ticks,
                "omitted signed-nonpositive value",
            );
        } else {
            report.limitation("E80C40 omits signed-nonpositive cooldown; absence verifies that predicate but not the exact native integer", label);
        }
    }
    for province in rows.keys() {
        report.difference(format!("railway province {province}"), "present", "absent");
    }
    report.compared("RailCooldowns", compared);
    Ok(())
}

pub(in crate::oracle) fn compare_rail_edges(
    frame: &DecodedFrame,
    saved: &saved::Pack,
    adjacency: &saved::RailwayAdjacency,
    report: &mut Report,
) -> io::Result<()> {
    let expected = saved
        .railways
        .as_ref()
        .ok_or_else(|| invalid("save lacks railway manager"))?;
    let mut rows = BTreeMap::new();
    for row in complete::<tables::RailEdges>(frame)? {
        if rows.insert((row.from, row.to), row.level).is_some() {
            return Err(invalid("duplicate recorded directed railway edge"));
        }
    }
    let mut compared = 0;
    let mut nonpositive = 0;
    for (&source, info) in expected {
        let neighbors = adjacency.neighbors(source).ok_or_else(|| {
            invalid(format!(
                "saved railway province {source} lacks verified content adjacency"
            ))
        })?;
        if neighbors.len() != info.levels.len() {
            return Err(invalid(format!(
                "railway province {source} has {} saved levels but {} verified content neighbors",
                info.levels.len(),
                neighbors.len()
            )));
        }
        for (&target, &level) in neighbors.iter().zip(&info.levels) {
            let label = format!("railway edge {source}->{target}");
            if let Some(actual) = rows.remove(&(source, target)) {
                equal(report, format!("{label}.level"), actual, level);
                compared += 1;
            } else if level > 0 {
                // E80070 reconstructs every positive entry in the sorted pairs.
                report.difference(label, "absent", level);
            }
            if level <= 0 {
                nonpositive += 1;
            }
        }
    }
    for ((source, target), level) in rows {
        report.difference(
            format!("railway edge {source}->{target}"),
            level,
            "absent from saved railway adjacency",
        );
    }
    report.compared("RailEdges", compared);
    if nonpositive != 0 {
        // E80070 loads only positive pairs, while E806E0 can insert a pair
        // after a signed max/clamp produces zero. The serialized indexed array
        // therefore does not encode the presence of every nonpositive pair.
        report.limitation("saved nonpositive railway levels do not uniquely encode sorted-pair presence; present recorded pairs compare their exact signed level, and every positive saved edge is required", format!("RailEdges: {nonpositive} nonpositive saved neighbor positions"));
    }
    Ok(())
}
