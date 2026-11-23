use super::{Arg, Tooltip, counter_content};
use replay_data::{rows::AirCountryRelationRow, rows::NavalAccidentReportRow, rows::tables};
use replay_frame::frame::DecodedFrame;
use std::collections::BTreeMap;
use replay_derive::naval_region_control::raw_roster;
use viewer_foundation::{perspective::Vision, tooltip_text::ensure_newlines};

#[derive(Default, Debug, PartialEq, Eq)]
struct Aggregate {
    damage: i64,
    sunk: i32,
    total: i32,
}
impl Aggregate {
    fn add(&mut self, row: &NavalAccidentReportRow) {
        self.damage = self.damage.wrapping_add(row.damage);
        self.sunk = self.sunk.wrapping_add(i32::from(row.sunk != 0));
        self.total = self.total.wrapping_add(1);
    }
}

pub fn build(
    content: &counter_content::Content,
    frame: &DecodedFrame,
    vision: &Vision,
    region: u32,
) -> Tooltip {
    let mut out = Tooltip::default();
    let built = (|| -> Result<String, String> {
        let links: BTreeMap<_, _> = frame
            .complete::<tables::CountryLinks>()
            .map_err(|e| e.to_string())?
            .iter()
            .map(|row| (row.id, row.canonical_slot))
            .collect();
        let slot = |raw: u32| {
            if raw == 0 {
                Ok(0)
            } else {
                links
                    .get(&raw)
                    .copied()
                    .ok_or_else(|| "naval incident country link is unavailable".to_owned())
            }
        };
        let ours = vision.ours();
        let reports = frame
            .complete::<tables::NavalAccidentReports>()
            .map_err(|e| e.to_string())?;
        let enemies = raw_roster(frame, ours, AirCountryRelationRow::ENEMY)?;
        let mut groups: [Aggregate; 3] = Default::default();
        for report in reports
            .iter()
            .filter(|row| row.region == region && slot(row.country) == Ok(ours))
        {
            match report.kind {
                0 if slot(report.owner)? == ours => groups[0].add(report),
                1 => groups[1].add(report),
                _ => {}
            }
        }
        for enemy in enemies {
            let enemy = slot(enemy)?;
            for report in reports
                .iter()
                .filter(|row| row.region == region && slot(row.country) == Ok(enemy))
            {
                match report.kind {
                    0 if slot(report.owner)? == ours => groups[0].add(report),
                    1 if slot(report.mine_owner)? == ours => groups[2].add(report),
                    _ => {}
                }
            }
        }
        let mut text = String::new();
        for (group, key) in groups.iter().zip([
            "NAVAL_ACCIDENTS_TRAINING",
            "NAVAL_ACCIDENTS_MINES",
            "NAVAL_ACCIDENTS_ENEMY",
        ]) {
            if group.total == 0 {
                continue;
            }
            ensure_newlines(&mut text, 1);
            text.push_str(&format_group(group, key, |key, args| {
                content
                    .localisation
                    .localise_with(key.as_bytes(), args)
                    .map_err(|e| e.to_string())
            })?);
        }
        Ok(text)
    })();
    match built {
        Ok(text) => out.title = text,
        Err(reason) => out.refuse("naval incident summary", reason),
    }
    match content
        .localisation
        .localise(b"NAVAL_ACCIDENTS_ACTIONS")
        .map_err(|e| e.to_string())
    {
        Ok(body) => out.body = body,
        Err(reason) => out.refuse("naval incident actions", reason),
    }
    out
}

fn format_group(
    group: &Aggregate,
    key: &str,
    mut loc: impl FnMut(&str, &[(&str, Arg<'_>)]) -> Result<String, String>,
) -> Result<String, String> {
    let mut text = loc(key, &[("NUM", Arg::Int32(group.total))])?;
    if group.damage > 0 {
        text.push_str(&loc(
            "NAVAL_ACCIDENTS_DAMAGED",
            &[("DMG", Arg::Fixed5(group.damage))],
        )?);
    }
    if group.sunk > 0 {
        text.push_str(&loc(
            "NAVAL_ACCIDENTS_SUNK",
            &[("SHIPS", Arg::Int32(group.sunk))],
        )?);
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn incident_aggregate_preserves_repeated_reports_and_signed_wrapping() {
        let report = NavalAccidentReportRow {
            damage: i64::MAX,
            sunk: 2,
            ..Default::default()
        };
        let mut sum = Aggregate::default();
        sum.add(&report);
        sum.add(&report);
        assert_eq!(
            sum,
            Aggregate {
                damage: -2,
                sunk: 2,
                total: 2
            }
        );
    }
    #[test]
    fn group_text_demands_only_positive_damage_and_sunk_fragments() {
        let mut asked = Vec::new();
        let text = format_group(
            &Aggregate {
                total: 3,
                damage: -1,
                sunk: 0,
            },
            "fixture_group",
            |key, _| {
                asked.push(key.to_owned());
                Ok(key.into())
            },
        )
        .unwrap();
        assert_eq!(text, "fixture_group");
        assert_eq!(asked, ["fixture_group"]);
        let text = format_group(
            &Aggregate {
                total: 4,
                damage: 1,
                sunk: 2,
            },
            "fixture_group",
            |key, _| Ok(key.into()),
        )
        .unwrap();
        assert_eq!(
            text,
            "fixture_groupNAVAL_ACCIDENTS_DAMAGEDNAVAL_ACCIDENTS_SUNK"
        );
    }
}
