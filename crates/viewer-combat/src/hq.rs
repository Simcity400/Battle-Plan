
use replay_data::{rows::LandCombatSideDetails, rows::tables};
use replay_frame::frame::DecodedFrame;
use std::{fmt, io};
use dashboard_text::table_access::{Available, rows};

const FORMATIONS_FEATURE: u32 = 1 << 17;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Unavailable {
    FeaturesFailed(String),
    FeaturesTruncated,
    FeatureRows(usize),
    FormationsFailed { formation: u64, reason: String },
    FormationNotReached { formation: u64, missing: u32 },
    FormationAbsent { formation: u64 },
}

impl fmt::Display for Unavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FeaturesFailed(reason) => {
                write!(f, "the SimulationFeatures table is unavailable ({reason})")
            }
            Self::FeaturesTruncated => f.write_str("the SimulationFeatures table is truncated"),
            Self::FeatureRows(count) => write!(
                f,
                "the SimulationFeatures table holds {count} rows instead of one"
            ),
            Self::FormationsFailed { formation, reason } => write!(
                f,
                "the OrdersGroups table holding the commander's formation {formation} is unavailable ({reason})"
            ),
            Self::FormationNotReached { formation, missing } => write!(
                f,
                "the truncated OrdersGroups table ({missing} rows not reached) lacks the commander's formation {formation}"
            ),
            Self::FormationAbsent { formation } => write!(
                f,
                "the OrdersGroups table has no row for the commander's formation {formation}"
            ),
        }
    }
}

pub(super) fn unit(
    frame: &DecodedFrame,
    side: &LandCombatSideDetails,
) -> io::Result<Result<u64, Unavailable>> {
    if side.commander == 0 || side.commander_category as u32 >= 2 {
        return Ok(Ok(0));
    }
    let flags = match rows::<tables::SimulationFeatures>(frame)? {
        Available::Complete([flags]) => flags,
        Available::Complete(rows) => return Ok(Err(Unavailable::FeatureRows(rows.len()))),
        Available::Truncated { .. } => return Ok(Err(Unavailable::FeaturesTruncated)),
        Available::Unavailable { reason } => {
            return Ok(Err(Unavailable::FeaturesFailed(reason.to_owned())));
        }
        Available::NotRecorded { mode } => {
            return Ok(Err(Unavailable::FeaturesFailed(format!(
                "not recorded in a {mode} recording"
            ))));
        }
    };
    let formation = side.commander_formation;
    if flags.dlc_mask & FORMATIONS_FEATURE == 0 || formation == 0 {
        return Ok(Ok(0));
    }
    let groups = rows::<tables::OrdersGroups>(frame)?;
    let Some(row) = groups.rows().iter().find(|row| row.handle == formation) else {
        return Ok(Err(match groups {
            Available::Complete(_) => Unavailable::FormationAbsent { formation },
            Available::Truncated { missing, .. } => {
                Unavailable::FormationNotReached { formation, missing }
            }
            Available::Unavailable { reason } => Unavailable::FormationsFailed {
                formation,
                reason: reason.to_owned(),
            },
            Available::NotRecorded { mode } => Unavailable::FormationsFailed {
                formation,
                reason: format!("not recorded in a {mode} recording"),
            },
        }));
    };
    Ok(Ok(
        if row.leader != 0 && row.hq_provider != 0 && side.committed.contains(&row.hq_provider) {
            row.hq_provider
        } else {
            0
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::{
        rows::OrdersGroupRow, rows::SimulationFeatureRow, rows::TableId, rows::TableRows,
    };
    use replay_frame::frame::TableView;

    const LEADER: u64 = (11 << 32) | 4713;
    const FORMATION: u64 = (19 << 32) | 4721;
    const HQ: u64 = (23 << 32) | 4714;

    fn fixture() -> (DecodedFrame, LandCombatSideDetails) {
        let mut frame = DecodedFrame {
            branch: 0,
            hour: 1,
            tables: std::array::from_fn(|_| {
                TableView::Unavailable {
                    reason: "missing".into(),
                }
                .into()
            }),
            metrics: Default::default(),
        };
        *frame.tables[TableId::SimulationFeatures.index()] =
            TableView::Complete(TableRows::SimulationFeatures(vec![SimulationFeatureRow {
                dlc_mask: FORMATIONS_FEATURE,
                ..Default::default()
            }]));
        *frame.tables[TableId::OrdersGroups.index()] =
            TableView::Complete(TableRows::OrdersGroups(vec![OrdersGroupRow {
                handle: FORMATION,
                leader: LEADER + (1 << 32),
                hq_provider: HQ,
                ..Default::default()
            }]));
        (
            frame,
            LandCombatSideDetails {
                commander: LEADER,
                commander_category: 1,
                commander_formation: FORMATION,
                committed: vec![HQ],
                ..Default::default()
            },
        )
    }

    #[test]
    fn the_forward_formation_reference_decides_and_the_reverse_leader_need_not_match() {
        let (frame, mut side) = fixture();
        assert_eq!(unit(&frame, &side).unwrap(), Ok(HQ));
        side.commander_formation = 0;
        assert_eq!(unit(&frame, &side).unwrap(), Ok(0));
    }

    #[test]
    fn missing_operands_name_their_cause_but_unrelated_truncation_is_usable() {
        let (mut frame, mut side) = fixture();
        let groups = std::mem::replace(
            &mut *frame.tables[TableId::OrdersGroups.index()],
            TableView::Unavailable {
                reason: "walk failed".into(),
            },
        );
        let unavailable = unit(&frame, &side).unwrap().unwrap_err();
        assert_eq!(
            unavailable,
            Unavailable::FormationsFailed {
                formation: FORMATION,
                reason: "walk failed".into()
            }
        );
        assert!(unavailable.to_string().contains("(walk failed)"));
        let TableView::Complete(groups) = groups else {
            panic!()
        };
        *frame.tables[TableId::OrdersGroups.index()] = TableView::Truncated {
            rows: groups.clone(),
            missing: 1,
        };
        assert_eq!(unit(&frame, &side).unwrap(), Ok(HQ));
        side.commander_formation = FORMATION + 1;
        assert_eq!(
            unit(&frame, &side).unwrap(),
            Err(Unavailable::FormationNotReached {
                formation: FORMATION + 1,
                missing: 1
            })
        );
        *frame.tables[TableId::OrdersGroups.index()] = TableView::Complete(groups);
        assert_eq!(
            unit(&frame, &side).unwrap(),
            Err(Unavailable::FormationAbsent {
                formation: FORMATION + 1
            })
        );
        side.commander_formation = FORMATION;
        let enabled = SimulationFeatureRow {
            dlc_mask: FORMATIONS_FEATURE,
            ..Default::default()
        };
        for (features, expected) in [
            (
                TableView::Complete(TableRows::SimulationFeatures(Vec::new())),
                Unavailable::FeatureRows(0),
            ),
            (
                TableView::Complete(TableRows::SimulationFeatures(vec![
                    enabled.clone(),
                    enabled.clone(),
                ])),
                Unavailable::FeatureRows(2),
            ),
            (
                TableView::Truncated {
                    rows: TableRows::SimulationFeatures(vec![enabled]),
                    missing: 1,
                },
                Unavailable::FeaturesTruncated,
            ),
            (
                TableView::Unavailable {
                    reason: "absent".into(),
                },
                Unavailable::FeaturesFailed("absent".into()),
            ),
        ] {
            *frame.tables[TableId::SimulationFeatures.index()] = features;
            assert_eq!(unit(&frame, &side).unwrap(), Err(expected));
        }
    }

    #[test]
    fn reserves_null_links_non_army_commanders_and_the_feature_hide_the_badge() {
        let (mut frame, mut side) = fixture();
        side.committed.clear();
        side.reserves.push(HQ);
        assert_eq!(unit(&frame, &side).unwrap(), Ok(0));
        side.committed.push(HQ);
        for (leader, provider) in [(0, HQ), (LEADER, 0)] {
            let TableView::Complete(TableRows::OrdersGroups(groups)) =
                &mut *frame.tables[TableId::OrdersGroups.index()]
            else {
                panic!()
            };
            groups[0].leader = leader;
            groups[0].hq_provider = provider;
            assert_eq!(unit(&frame, &side).unwrap(), Ok(0));
        }
        let TableView::Complete(TableRows::SimulationFeatures(features)) =
            &mut *frame.tables[TableId::SimulationFeatures.index()]
        else {
            panic!()
        };
        features[0].dlc_mask = 0;
        assert_eq!(unit(&frame, &side).unwrap(), Ok(0));
        for category in [-1, 2, i32::MAX] {
            side.commander_category = category;
            assert_eq!(unit(&frame, &side).unwrap(), Ok(0));
        }
    }
}
