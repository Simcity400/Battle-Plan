use super::{WarSide, native::Blob};
use dashboard_base::catalogue::Catalogue;
use dashboard_query::coverage::witnessed_rows;
use replay_data::{
    rows::tables,
    war_participation::{self as derive, DeliveryHistory, Factors, Scores},
};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct WarContributionCountry {
    pub tag: String,
    pub side: WarSide,
    #[doc = " All quantities are exact signed fixed5 words."]
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
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct WarContributionGroup {
    pub viewpoint_side: Vec<String>,
    pub opposing_side: Vec<String>,
    pub participants: Vec<WarContributionCountry>,
    pub measures: Vec<super::measures::WarMeasure>,
    pub reason: Option<String>,
}

pub(crate) fn factors(cat: Option<&Catalogue>) -> Result<Factors, String> {
    cat.ok_or("Recorded content is needed for lend-lease participation")?
        .war_participation_factors()
}

pub(crate) fn contributions(
    blobs: &[Blob],
    (tags, raw_to_slot): (&HashMap<u32, String>, &HashMap<u32, u32>),
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
    let participation = witnessed_rows::<tables::WarParticipation>(frame);
    let inputs = (|| -> Result<_, String> {
        let factors = factors?;
        if participation.rows.is_empty() && let Some(reason) = &participation.missing {
            return Err(reason.clone());
        }
        let mut ledgers = HashMap::new();
        for row in participation.rows {
            let (Some(&a), Some(&b)) = (raw_to_slot.get(&row.war_first), raw_to_slot.get(&row.war_second)) else { continue };
            let key = match row.direction {
                0 => (a, b),
                1 => (b, a),
                _ => continue,
            };
            ledgers.entry(key).and_modify(|held| *held = None).or_insert_with(|| Some(row.scores()));
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
                measures: Vec::new(),
                reason: None,
            };
            let (ledgers, histories, factors) = match &inputs {
                Ok(inputs) => inputs,
                Err(reason) => {
                    group.reason = Some(reason.clone());
                    return group;
                }
            };
            'sides: for (side, own, opponents) in [
                (WarSide::Viewpoint, &blob.first, &blob.second),
                (WarSide::Opponent, &blob.second, &blob.first),
            ] {
                let mut scores = Vec::with_capacity(own.len());
                for &country in own {
                    let mut directional = Vec::new();
                    for &opponent in opponents {
                        if active(country, opponent) {
                            match ledgers.get(&(country, opponent)) {
                                Some(Some(ledger)) => directional.push(*ledger),
                                unresolved => {
                                    group.reason.get_or_insert_with(|| match unresolved {
                                        Some(_) => "Ambiguous directional participation ledger".into(),
                                        None => participation.missing.clone().unwrap_or_else(|| "Directional participation ledger not recorded".into()),
                                    });
                                    continue 'sides;
                                }
                            }
                        }
                    }
                    let deliveries = own.iter().filter_map(|&ally| {
                        (ally != country && !active(country, ally)).then(|| histories.get(&(country, ally)).copied().unwrap_or_default())
                    });
                    scores.push(derive::breakdown(directional, deliveries, *factors));
                }
                for (&country, score) in own.iter().zip(&scores) {
                    group.participants.push(country_view(
                        tags[&country].clone(), side, *score,
                        derive::share(score.total(false), scores.iter().map(|s| s.total(false))),
                    ));
                }
            }
            if !group.participants.is_empty() {
                group.measures = super::measures::measures(&group.viewpoint_side, &group.opposing_side, &group.participants);
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
