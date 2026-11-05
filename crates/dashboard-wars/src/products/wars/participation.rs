//! Requested-hour participation. These values are independent of the prepared
//! structural history, so damage changes never duplicate the war timeline.
use super::*;
use replay_data::war_participation::{self as derive, DeliveryHistory, Factors, Scores};

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WarContributionCountry {
    pub tag: String,
    pub side: WarSide,
    /// All quantities are exact signed fixed5 words.
    pub equipment_damage: String,
    pub occupation: String,
    pub air_damage: String,
    pub strategic_bombing: String,
    pub ship_damage: String,
    pub convoy_damage: String,
    pub casualties: String,
    pub unknown_casualties: String,
    pub lend_lease_sent: String,
    pub lend_lease_received: String,
    pub score: String,
    pub participation: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct WarContributionGroup {
    pub viewpoint_side: Vec<String>,
    pub opposing_side: Vec<String>,
    pub participants: Vec<WarContributionCountry>,
    pub reason: Option<String>,
}

pub(super) fn factors(cat: Option<&Catalogue>) -> Result<Factors, String> {
    cat.ok_or("Recorded content is needed for lend-lease participation")?
        .war_participation_factors()
}

#[cfg(any(test, feature = "test-support"))]
pub(super) fn read(
    frame: &DecodedFrame,
    view: Option<&str>,
    cat: Option<&Catalogue>,
) -> Vec<WarContributionGroup> {
    read_with_factors(frame, view, factors(cat))
}
#[cfg(any(test, feature = "test-support"))]
fn read_with_factors(
    frame: &DecodedFrame,
    view: Option<&str>,
    factors: Result<Factors, String>,
) -> Vec<WarContributionGroup> {
    let Ok(hour) = Hour::read(frame, view) else {
        return Vec::new();
    };
    contributions(
        &hour.world.blobs(hour.view),
        (&hour.tags, &hour.raw_to_slot),
        &|a, b| hour.world.active(a, b),
        frame,
        factors,
    )
}
/// Each of the viewpoint's `blobs`' contributions from the participation tables `frame` holds,
/// countries resolved through the hour's tags and links; `active` tells the hour's relations.
pub(super) fn contributions(
    blobs: &[Blob],
    (tags, raw_to_slot): (
        &foldhash::HashMap<u32, String>,
        &foldhash::HashMap<u32, u32>,
    ),
    active: &dyn Fn(u32, u32) -> bool,
    frame: &DecodedFrame,
    factors: Result<Factors, String>,
) -> Vec<WarContributionGroup> {
    let slot = |raw| {
        raw_to_slot
            .get(&raw)
            .copied()
            .ok_or_else(|| format!("Unresolved participation country {raw}"))
    };
    let inputs = (|| -> Result<_, String> {
        let factors = factors?;
        let mut ledgers = HashMap::new();
        for row in frame
            .complete::<tables::WarParticipation>()
            .map_err(|e| e.to_string())?
        {
            let (a, b) = (slot(row.war_first)?, slot(row.war_second)?);
            let key = match row.direction {
                0 => (a, b),
                1 => (b, a),
                _ => return Err("Invalid participation direction".into()),
            };
            if ledgers.insert(key, row.scores()).is_some() {
                return Err("Ambiguous directional participation ledger".into());
            }
        }
        let mut histories = HashMap::new();
        for row in frame
            .complete::<tables::WarLendLeaseHistories>()
            .map_err(|e| e.to_string())?
        {
            let key = (slot(row.country)?, slot(row.target)?);
            let value = DeliveryHistory {
                industry_sent: row.industry_sent,
                fuel_sent: row.fuel_sent,
                industry_received: row.industry_received,
                fuel_received: row.fuel_received,
            };
            if histories.insert(key, value).is_some() {
                return Err("Ambiguous participation delivery history".into());
            }
        }
        Ok((ledgers, histories, factors))
    })();
    blobs
        .iter()
        .map(|blob| {
            let mut group = WarContributionGroup {
                viewpoint_side: blob.first.iter().map(|id| tags[id].clone()).collect(),
                opposing_side: blob.second.iter().map(|id| tags[id].clone()).collect(),
                participants: Vec::new(),
                reason: None,
            };
            let result = (|| -> Result<Vec<WarContributionCountry>, String> {
                let (ledgers, histories, factors) = inputs.as_ref().map_err(Clone::clone)?;
                let mut participants = Vec::new();
                for (side, own, opponents) in [
                    (WarSide::Viewpoint, &blob.first, &blob.second),
                    (WarSide::Opponent, &blob.second, &blob.first),
                ] {
                    let mut scores = Vec::with_capacity(own.len());
                    for &country in own {
                        let mut directional = Vec::new();
                        for &opponent in opponents {
                            if active(country, opponent) {
                                directional.push(
                                    *ledgers
                                        .get(&(country, opponent))
                                        .ok_or("Directional participation ledger not recorded")?,
                                );
                            }
                        }
                        let mut deliveries = Vec::new();
                        for &ally in own {
                            if ally != country && !active(country, ally) {
                                deliveries.push(
                                    histories.get(&(country, ally)).copied().unwrap_or_default(),
                                );
                            }
                        }
                        scores.push(derive::breakdown(directional, deliveries, *factors));
                    }
                    for (&country, score) in own.iter().zip(&scores) {
                        participants.push(country_view(
                            tags[&country].clone(),
                            side,
                            *score,
                            derive::share(
                                score.total(false),
                                scores.iter().map(|s| s.total(false)),
                            ),
                        ));
                    }
                }
                Ok(participants)
            })();
            match result {
                Ok(participants) => group.participants = participants,
                Err(reason) => group.reason = Some(reason),
            }
            group
        })
        .collect()
}
fn country_view(tag: String, side: WarSide, s: Scores, share: i64) -> WarContributionCountry {
    WarContributionCountry {
        tag,
        side,
        equipment_damage: s.equipment_damage.to_string(),
        occupation: s.province_capture.to_string(),
        air_damage: s.air_damage.to_string(),
        strategic_bombing: s.strategic_bombing.to_string(),
        ship_damage: s.sunk_ships.to_string(),
        convoy_damage: s.sunk_convoys.to_string(),
        casualties: s.casualties.to_string(),
        unknown_casualties: s.unknown_casualties.to_string(),
        lend_lease_sent: s.lend_lease_sent.to_string(),
        lend_lease_received: s.lend_lease_received.to_string(),
        score: s.total(false).to_string(),
        participation: share.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::rows::TableRows;

    fn recorded() -> DecodedFrame {
        let mut frame = super::super::test_fixtures::recorded(100);
        let ledgers = [
            (2, 0, 100_000),
            (2, 1, 100_000),
            (3, 0, 200_000),
            (3, 1, 300_000),
        ]
        .into_iter()
        .map(|(second, direction, damage)| rows::WarParticipationRow {
            war_first: 1,
            war_second: second,
            direction,
            equipment_damage: damage,
            first_country: if direction == 0 { 1 } else { second },
            second_country: if direction == 0 { second } else { 1 },
            ..Default::default()
        })
        .collect();
        frame.tables[TableId::WarParticipation.index()] =
            TableView::Complete(TableRows::WarParticipation(ledgers)).into();
        frame.tables[TableId::WarLendLeaseHistories.index()] =
            TableView::Complete(TableRows::WarLendLeaseHistories(vec![
                rows::WarLendLeaseHistoryRow {
                    country: 2,
                    target: 3,
                    industry_sent: 100_000,
                    ..Default::default()
                },
                rows::WarLendLeaseHistoryRow {
                    country: 3,
                    target: 2,
                    industry_received: 100_000,
                    ..Default::default()
                },
            ]))
            .into();
        frame
    }
    fn factors() -> Result<Factors, String> {
        Ok(Factors {
            industry_given: 100_000,
            fuel_given: 100_000,
            industry_received: 100_000,
            fuel_received: 100_000,
        })
    }
    #[test]
    fn participation_uses_directional_credit_and_same_side_deliveries_for_each_native_view() {
        let frame = recorded();
        for view in [None, Some("TWO")] {
            let groups = read_with_factors(&frame, view, factors());
            let group = &groups[0];
            assert!(group.reason.is_none(), "{:?}", group.reason);
            let two = group.participants.iter().find(|r| r.tag == "TWO").unwrap();
            let three = group
                .participants
                .iter()
                .find(|r| r.tag == "THREE")
                .unwrap();
            assert_eq!(two.score, "200000");
            assert_eq!(three.score, "200000");
            assert_eq!(two.participation, "50000");
            assert_eq!(three.participation, "50000");
            assert_eq!(three.lend_lease_received, "-100000");
        }
    }
    #[test]
    fn participation_screen_and_hover_gate_ignore_native_histories_outside_each_displayed_side() {
        let mut frame = recorded();
        let mut histories = frame
            .complete::<tables::WarLendLeaseHistories>()
            .unwrap()
            .to_vec();
        // These native histories remain recorded even while their countries
        // are opponents, or when the row targets its own owner.
        histories.extend(
            [(1, 1), (1, 2), (1, 3), (2, 1), (2, 2), (3, 1), (3, 3)].map(|(country, target)| {
                rows::WarLendLeaseHistoryRow {
                    country,
                    target,
                    industry_sent: 900_000_000,
                    fuel_received: -700_000_000,
                    ..Default::default()
                }
            }),
        );
        let baseline: Vec<_> = [None, Some("ONE"), Some("TWO"), Some("THREE")]
            .map(|view| serde_json::to_value(read_with_factors(&frame, view, factors())).unwrap())
            .into();
        frame.tables[TableId::WarLendLeaseHistories.index()] =
            TableView::Complete(TableRows::WarLendLeaseHistories(histories)).into();
        for (index, view) in [None, Some("ONE"), Some("TWO"), Some("THREE")]
            .into_iter()
            .enumerate()
        {
            // This shared payload supplies both war-screen participation and
            // contribution hovers, including all exact component strings.
            assert_eq!(
                serde_json::to_value(read_with_factors(&frame, view, factors())).unwrap(),
                baseline[index]
            );
        }
        // Retained/asymmetric ownership still uses the complete native roster:
        // THREE need not own a reciprocal occurrence to appear on TWO's side.
        let mut owned = frame
            .complete::<tables::WarOwnedRelations>()
            .unwrap()
            .to_vec();
        owned.retain(|row| row.owner != 3);
        for row in &mut owned {
            if row.second_country == 3 {
                row.retired = 1;
            }
        }
        frame.tables[TableId::WarOwnedRelations.index()] =
            TableView::Complete(TableRows::WarOwnedRelations(owned)).into();
        for view in [None, Some("TWO")] {
            let groups = read_with_factors(&frame, view, factors());
            let participants = &groups[0].participants;
            assert_eq!(
                participants
                    .iter()
                    .find(|p| p.tag == "TWO")
                    .unwrap()
                    .lend_lease_sent,
                "100000"
            );
            assert_eq!(
                participants
                    .iter()
                    .find(|p| p.tag == "THREE")
                    .unwrap()
                    .lend_lease_received,
                "-100000"
            );
        }
    }

    #[test]
    fn participation_missing_side_input_refuses_the_share_instead_of_inventing_zero() {
        let mut frame = recorded();
        frame.tables[TableId::WarLendLeaseHistories.index()] =
            TableView::Complete(TableRows::WarLendLeaseHistories(Vec::new())).into();
        let complete = read_with_factors(&frame, None, factors());
        assert_eq!(
            complete[0]
                .participants
                .iter()
                .find(|p| p.tag == "TWO")
                .unwrap()
                .participation,
            "25000"
        );
        frame.tables[TableId::WarLendLeaseHistories.index()] = TableView::Unavailable {
            reason: "synthetic missing history certificate".into(),
        }
        .into();
        let group = read_with_factors(&frame, None, factors()).remove(0);
        assert!(group.participants.is_empty());
        assert!(
            group
                .reason
                .unwrap()
                .contains("synthetic missing history certificate")
        );
    }
    #[test]
    fn participation_contract_retains_exact_integer_strings() {
        let row = country_view(
            "MOD".into(),
            WarSide::Viewpoint,
            Scores {
                equipment_damage: 9_007_199_254_740_993,
                ..Default::default()
            },
            1,
        );
        let value = serde_json::to_value(row).unwrap();
        assert_eq!(value["equipmentDamage"], "9007199254740993");
    }
}
