//! Recorded nuclear impacts ([`replay::rows::NuclearStrikeRow`]): who dropped
//! which bomb on whose state, and what the impact did before the authored
//! `on_nuke_drop` effects.

use super::{Builder, EventFeedKind, EventFeedRow, EventFeedStrike, check_cancel};
#[cfg(test)]
use crate::{metadata::validate_campaign, products::country_history::NativeNames as Names};
use replay::{
    frame::{DecodedFrame, TableView},
    rows::{NuclearStrikePart, NuclearStrikeRow, RowCodec, TableId, tables},
};
use std::{collections::BTreeMap, io, sync::atomic::AtomicBool};

#[cfg(test)]
use replay::{container::Replay, frame::FrameDecoder};
#[cfg(test)]
use std::collections::BTreeSet;

/// A full building level's health.
const LEVEL_HEALTH: f64 = 10_000_000.0;
/// War support is a fixed5 fraction; the game shows it in percent.
const WAR_SUPPORT_PERCENT: f64 = 1_000.0;
/// Equipment pools hold fixed5 quantities.
const EQUIPMENT_UNIT: f64 = 100_000.0;

fn part_name(part: NuclearStrikePart) -> &'static str {
    match part {
        NuclearStrikePart::WarSupport => "war support",
        NuclearStrikePart::Buildings => "buildings",
        NuclearStrikePart::Units => "units",
        NuclearStrikePart::Equipment => "equipment",
    }
}

/// The tooltip's totals. A part with a refusal is not totalled: what it lists
/// is not everything the impact did.
fn detail(strike: &NuclearStrikeRow) -> EventFeedStrike {
    let complete = |part| strike.complete(part);
    EventFeedStrike {
        province: strike.province,
        state: (strike.state != 0).then_some(strike.state),
        building_levels: complete(NuclearStrikePart::Buildings).then(|| {
            strike
                .buildings
                .iter()
                .map(|building| building.amount_raw as f64)
                .sum::<f64>()
                / LEVEL_HEALTH
        }),
        war_support: complete(NuclearStrikePart::WarSupport).then(|| {
            strike
                .war_support
                .iter()
                .map(|change| change.after_raw as f64 - change.before_raw as f64)
                .sum::<f64>()
                / WAR_SUPPORT_PERCENT
        }),
        units: complete(NuclearStrikePart::Units).then_some(strike.units.len() as u32),
        // Divisions and railway guns lose manpower; wings lose their crews.
        manpower_lost: (complete(NuclearStrikePart::Units)
            && complete(NuclearStrikePart::Equipment))
        .then(|| {
            let units = strike
                .units
                .iter()
                .map(|unit| unit.manpower_before as f64 - unit.manpower_after as f64);
            let crews = strike
                .wings
                .iter()
                .map(|wing| wing.manpower_before as f64 - wing.manpower_after as f64);
            units.chain(crews).sum()
        }),
        equipment_lost: complete(NuclearStrikePart::Equipment).then(|| {
            strike
                .equipment
                .iter()
                .map(|loss| loss.amount_raw as f64)
                .sum::<f64>()
                / EQUIPMENT_UNIT
        }),
        not_recorded: strike
            .refusals
            .iter()
            .map(|refusal| {
                format!(
                    "{}: {} ({}×)",
                    part_name(refusal.part),
                    String::from_utf8_lossy(&refusal.reason),
                    refusal.count
                )
            })
            .collect(),
    }
}

/// Retained strike facts and refusals from one supplying read.
pub(super) fn read(
    frame: &DecodedFrame,
    builder: &mut Builder,
    cancelled: &AtomicBool,
) -> io::Result<Vec<NuclearStrikeRow>> {
    let read = match &*frame.tables[TableId::NuclearStrikes.index()] {
        TableView::Unavailable { reason } => Err(reason.to_string()),
        _ => frame
            .complete::<tables::NuclearStrikes>()
            .map_err(|error| error.to_string()),
    };
    let rows = match read {
        Ok(rows) => rows,
        Err(reason) => {
            builder.sampled_unavailable(
                "nuclearStrikeCoverage",
                "Nuclear strikes",
                format!(
                    "Strikes up to the read at {} are not recorded: {reason}. Missing rows do not prove no strike occurred.",
                    frame.hour
                ),
                1,
            );
            return Ok(Vec::new());
        }
    };
    let mut accepted = Vec::new();
    for strike in rows {
        check_cancel(cancelled)?;
        if strike.hour > frame.hour {
            builder.sampled_unavailable(
                "nuclearStrikeCoverage",
                "Nuclear strikes",
                format!(
                    "Strike {} is dated after its supplying read at {}.",
                    strike.id, frame.hour
                ),
                1,
            );
            continue;
        }
        accepted.push(strike.try_clone()?);
    }
    Ok(accepted)
}

/// Naming and tooltip semantics shared by the recording oracle and prepared feed.
pub(super) fn project(
    read_hour: i32,
    strike: &NuclearStrikeRow,
    mut tag_at: impl FnMut(i32, replay::event_feed_rows::EventFeedCountry) -> Option<String>,
    builder: &mut Builder,
    items: &BTreeMap<u32, u32>,
) {
    // The reserved country a stateless strike falls on has no tag.
    let participants = [strike.launcher, strike.owner].map(|country| tag_at(strike.hour, country));
    for (role, tag, expected) in [
        ("launcher", &participants[0], true),
        ("target owner", &participants[1], strike.state != 0),
    ] {
        if expected && tag.is_none() {
            builder.sampled_unavailable(
                "nuclearStrikeCountry",
                "Nuclear strike country",
                format!(
                    "The {role} of strike {} at {} has no exact country-reference, ID and instance witness.",
                    strike.id, strike.hour
                ),
                1,
            );
        }
    }
    let item = items.get(&strike.nuke_type).copied();
    if item.is_none() {
        builder.sampled_unavailable(
            "nuclearType",
            "Nuclear type",
            format!(
                "Nuclear type {} has no native content-name mapping.",
                strike.nuke_type
            ),
            1,
        );
    }
    if strike.hour < builder.feed.first || strike.hour > builder.feed.last {
        return;
    }
    let kind = EventFeedKind::NuclearStrike;
    builder.feed.rows.push(EventFeedRow {
        id: format!("nuclear:{}", strike.id),
        hour: strike.hour,
        read_hour,
        name_hour: strike.hour,
        exact: true,
        category: kind.category(),
        kind,
        tags: participants.iter().flatten().cloned().collect(),
        participants: Some(participants),
        item,
        amount: None,
        cause: None,
        strike: Some(detail(strike)),
    });
}

/// Two selected-table passes: the strike history, then the country identity
/// tables of only the hours a strike happened in.
#[cfg(test)]
pub(super) fn scan(
    replay: &Replay,
    branch: u64,
    builder: &mut Builder,
    items: &BTreeMap<u32, u32>,
    cancelled: &AtomicBool,
) -> io::Result<()> {
    if let Some(reason) = replay.format.table_refusal(TableId::NuclearStrikes) {
        builder.unavailable("nuclearStrikes", "Nuclear strikes", reason, None);
        return Ok(());
    }
    let mut blocks: Vec<_> = replay
        .index
        .iter()
        .filter(|block| block.branch == branch)
        .collect();
    blocks.sort_by_key(|block| block.first_hour);
    let selected = [TableId::Campaign, TableId::NuclearStrikes];
    let mut strikes: Vec<(i32, NuclearStrikeRow)> = Vec::new();
    for block in &blocks {
        check_cancel(cancelled)?;
        let mut decoder = FrameDecoder::for_tables(&selected);
        let frames = match replay.read_block_selected(block, &selected) {
            Ok(frames) => frames,
            Err(error) => {
                builder.sampled_unavailable(
                    "nuclearStrikeCoverage",
                    "Nuclear strikes",
                    error.to_string(),
                    1,
                );
                continue;
            }
        };
        for encoded in frames {
            check_cancel(cancelled)?;
            let frame = match decoder.advance(&encoded) {
                Ok(frame) => frame,
                Err(error) => {
                    builder.sampled_unavailable(
                        "nuclearStrikeCoverage",
                        "Nuclear strikes",
                        error.to_string(),
                        1,
                    );
                    break;
                }
            };
            if let Err(error) = validate_campaign(frame, replay) {
                builder.sampled_unavailable(
                    "nuclearStrikeCoverage",
                    "Nuclear strikes",
                    error.to_string(),
                    1,
                );
                continue;
            }
            for strike in read(frame, builder, cancelled)? {
                strikes.push((frame.hour, strike));
            }
        }
    }
    if strikes.is_empty() {
        return Ok(());
    }
    // Participants are named by the impact hour's identity witnesses, as the
    // native events are: a reference resolves through the country array of
    // its hour, whose holder can change.
    let wanted: BTreeSet<i32> = strikes.iter().map(|(_, strike)| strike.hour).collect();
    let mut names_at: BTreeMap<i32, Names> = BTreeMap::new();
    let selected = [
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::CountryExperienceGains,
    ];
    for block in blocks {
        check_cancel(cancelled)?;
        if wanted
            .range(block.first_hour..=block.last_hour)
            .next()
            .is_none()
        {
            continue;
        }
        let mut decoder = FrameDecoder::for_tables(&selected);
        let Ok(frames) = replay.read_block_selected(block, &selected) else {
            continue;
        };
        for encoded in frames {
            check_cancel(cancelled)?;
            let Ok(frame) = decoder.advance(&encoded) else {
                break;
            };
            if !wanted.contains(&frame.hour) || validate_campaign(frame, replay).is_err() {
                continue;
            }
            if let Ok(names) = Names::at(frame, branch, frame.hour) {
                names_at.insert(frame.hour, names);
            }
        }
    }
    for (read_hour, strike) in strikes {
        check_cancel(cancelled)?;
        project(
            read_hour,
            &strike,
            |hour, country| names_at.get(&hour).and_then(|names| names.tag(country)),
            builder,
            items,
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::rows::{
        NuclearBuildingDamage, NuclearEquipmentLoss, NuclearStrikeRefusal, NuclearUnitDamage,
        NuclearWarSupport,
    };

    #[test]
    fn totals_use_native_units_and_leave_refused_parts_untotalled() {
        let strike = NuclearStrikeRow {
            province: 13_000,
            state: 1_204,
            war_support: vec![NuclearWarSupport {
                country: 1_500,
                before_raw: 60_000,
                after_raw: 35_000,
            }],
            buildings: vec![
                NuclearBuildingDamage {
                    amount_raw: 20_000_000,
                    ..Default::default()
                },
                NuclearBuildingDamage {
                    amount_raw: 5_000_000,
                    ..Default::default()
                },
            ],
            units: vec![NuclearUnitDamage {
                manpower_before: 9_000,
                manpower_after: 6_500,
                ..Default::default()
            }],
            equipment: vec![NuclearEquipmentLoss {
                amount_raw: 1_200_000,
                ..Default::default()
            }],
            refusals: vec![NuclearStrikeRefusal {
                part: NuclearStrikePart::Equipment,
                reason: b"equipment destruction writers not forwarded".to_vec(),
                count: 1,
            }],
            ..Default::default()
        };
        let detail = detail(&strike);
        assert_eq!(detail.state, Some(1_204));
        assert_eq!(detail.building_levels, Some(2.5));
        assert_eq!(detail.war_support, Some(-25.0));
        assert_eq!(detail.units, Some(1));
        // Air crews belong to the refused equipment calls.
        assert_eq!(detail.manpower_lost, None);
        assert_eq!(detail.equipment_lost, None);
        assert_eq!(
            detail.not_recorded,
            ["equipment: equipment destruction writers not forwarded (1×)"]
        );
        let stateless = NuclearStrikeRow::default();
        assert_eq!(super::detail(&stateless).state, None);
    }
}
