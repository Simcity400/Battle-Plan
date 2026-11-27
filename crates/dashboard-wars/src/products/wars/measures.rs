use super::participation::WarContributionCountry;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum WarMeasureKey {
    Participation,
    EquipmentDamage,
    Occupation,
    AirDamage,
    ShipDamage,
    Casualties,
    StrategicBombing,
    ConvoyDamage,
    LendLeaseSent,
    LendLeaseReceived,
}

pub const MEASURES: [WarMeasureKey; 10] = [
    WarMeasureKey::Participation,
    WarMeasureKey::EquipmentDamage,
    WarMeasureKey::Occupation,
    WarMeasureKey::AirDamage,
    WarMeasureKey::ShipDamage,
    WarMeasureKey::Casualties,
    WarMeasureKey::StrategicBombing,
    WarMeasureKey::ConvoyDamage,
    WarMeasureKey::LendLeaseSent,
    WarMeasureKey::LendLeaseReceived,
];

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct WarMeasureMember {
    pub tag: String,
    pub value: String,
    pub share_of_side: Option<u32>,
    pub share_of_war: Option<u32>,
    pub partial: bool,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct WarMeasureSide {
    pub total: Option<String>,
    pub share_of_war: Option<u32>,
    pub partial: bool,
    pub members: Vec<WarMeasureMember>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct WarMeasure {
    pub key: WarMeasureKey,
    pub viewpoint: WarMeasureSide,
    pub opposing: WarMeasureSide,
}

fn parts(row: &WarContributionCountry, key: WarMeasureKey) -> (i64, i64) {
    let number = |text: &str| text.parse::<i64>().unwrap_or(0);
    let value = match key {
        WarMeasureKey::Participation => &row.participation,
        WarMeasureKey::EquipmentDamage => &row.equipment_damage,
        WarMeasureKey::Occupation => &row.occupation,
        WarMeasureKey::AirDamage => &row.air_damage,
        WarMeasureKey::ShipDamage => &row.ship_damage,
        WarMeasureKey::Casualties => &row.casualties,
        WarMeasureKey::StrategicBombing => &row.strategic_bombing,
        WarMeasureKey::ConvoyDamage => &row.convoy_damage,
        WarMeasureKey::LendLeaseSent => &row.lend_lease_sent,
        WarMeasureKey::LendLeaseReceived => &row.lend_lease_received,
    };
    let unknown = if key == WarMeasureKey::Casualties { number(&row.unknown_casualties) } else { 0 };
    (number(value), unknown)
}

pub(crate) fn permyriad(part: i128, whole: i128) -> Option<u32> {
    if whole <= 0 {
        return None;
    }
    u32::try_from((part.abs() * 20_000 + whole) / (whole * 2)).ok()
}

fn side(tags: &[String], scores: &HashMap<&str, &WarContributionCountry>, key: WarMeasureKey, war_total: Option<i128>) -> WarMeasureSide {
    let mut members: Vec<(usize, &str, i64, i64)> = Vec::new();
    for (order, tag) in tags.iter().enumerate() {
        if members.iter().any(|member| member.1 == tag) {
            continue;
        }
        if let Some(row) = scores.get(tag.as_str()) {
            let (value, unknown) = parts(row, key);
            members.push((order, tag, value, unknown));
        }
    }
    let magnitude: i128 = members.iter().map(|member| i128::from(member.2).abs()).sum();
    let total: i128 = members.iter().map(|member| i128::from(member.2)).sum();
    members.sort_by(|a, b| i128::from(b.2).abs().cmp(&i128::from(a.2).abs()).then(a.0.cmp(&b.0)));
    let participation = key == WarMeasureKey::Participation;
    let members: Vec<WarMeasureMember> = members
        .into_iter()
        .map(|(_, tag, value, unknown)| WarMeasureMember {
            tag: tag.to_owned(),
            value: value.to_string(),
            share_of_side: if participation { permyriad(i128::from(value), 100_000) } else { permyriad(i128::from(value), magnitude) },
            share_of_war: war_total.and_then(|whole| permyriad(i128::from(value), whole)),
            partial: unknown != 0,
        })
        .collect();
    let partial = members.iter().any(|member| member.partial);
    WarMeasureSide { total: (!members.is_empty()).then(|| total.to_string()), share_of_war: war_total.and_then(|whole| permyriad(magnitude, whole)), partial, members }
}

fn magnitude_of(tags: &[String], scores: &HashMap<&str, &WarContributionCountry>, key: WarMeasureKey) -> i128 {
    let mut seen: Vec<&str> = Vec::new();
    let mut sum = 0_i128;
    for tag in tags {
        if seen.contains(&tag.as_str()) {
            continue;
        }
        seen.push(tag);
        if let Some(row) = scores.get(tag.as_str()) {
            sum += i128::from(parts(row, key).0).abs();
        }
    }
    sum
}

pub(crate) fn measures(viewpoint: &[String], opposing: &[String], participants: &[WarContributionCountry]) -> Vec<WarMeasure> {
    let mut scores: HashMap<&str, &WarContributionCountry> = HashMap::new();
    for row in participants {
        scores.entry(row.tag.as_str()).or_insert(row);
    }
    let complete = viewpoint.iter().chain(opposing).all(|tag| scores.contains_key(tag.as_str()));
    MEASURES
        .iter()
        .map(|&key| {
            let war_total = (complete && key != WarMeasureKey::Participation)
                .then(|| magnitude_of(viewpoint, &scores, key) + magnitude_of(opposing, &scores, key))
                .filter(|whole| *whole > 0);
            let viewpoint = side(viewpoint, &scores, key, war_total);
            let opposing = side(opposing, &scores, key, war_total);
            WarMeasure { key, viewpoint, opposing }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::products::wars::WarSide;

    #[test]
    fn shares_match_the_projection_vectors() {
        let vectors: Vec<(String, String, Option<u32>)> =
            serde_json::from_str(include_str!("../../../../../ui/src/widgets/war-contribution/permyriad.test.json")).unwrap();
        for (amount, total, expected) in vectors {
            assert_eq!(permyriad(amount.parse().unwrap(), total.parse().unwrap()), expected, "{amount} of {total}");
        }
    }

    fn row(tag: &str, equipment: i64, casualties: i64, unknown: i64, participation: i64) -> WarContributionCountry {
        WarContributionCountry {
            tag: tag.into(),
            side: WarSide::Viewpoint,
            equipment_damage: equipment.to_string(),
            occupation: "0".into(),
            air_damage: "0".into(),
            strategic_bombing: "0".into(),
            ship_damage: "0".into(),
            convoy_damage: "0".into(),
            casualties: casualties.to_string(),
            unknown_casualties: unknown.to_string(),
            lend_lease_sent: "0".into(),
            lend_lease_received: "-300000".into(),
            score: "0".into(),
            participation: participation.to_string(),
        }
    }

    #[test]
    fn a_measure_gives_each_side_its_share_of_the_war_and_each_member_its_share_of_the_side_largest_first() {
        let viewpoint = vec!["GER".to_owned(), "ITA".to_owned()];
        let opposing = vec!["SOV".to_owned()];
        let participants = vec![row("GER", 300_000, 100_000, 0, 69_200), row("ITA", 600_000, 50_000, 20_000, 30_800), row("SOV", 100_000, 0, 0, 100_000)];
        let all = measures(&viewpoint, &opposing, &participants);
        assert_eq!(all.iter().map(|measure| measure.key).collect::<Vec<_>>(), MEASURES);
        let equipment = all.iter().find(|measure| measure.key == WarMeasureKey::EquipmentDamage).unwrap();
        assert_eq!(equipment.viewpoint.members.iter().map(|member| member.tag.as_str()).collect::<Vec<_>>(), ["ITA", "GER"], "largest part first");
        assert_eq!(equipment.viewpoint.total.as_deref(), Some("900000"));
        assert_eq!((equipment.viewpoint.share_of_war, equipment.opposing.share_of_war), (Some(9_000), Some(1_000)));
        assert_eq!(equipment.viewpoint.members[0].share_of_side, Some(6_667));
        assert_eq!(equipment.viewpoint.members[0].share_of_war, Some(6_000));
        let casualties = all.iter().find(|measure| measure.key == WarMeasureKey::Casualties).unwrap();
        assert!(casualties.viewpoint.partial && casualties.viewpoint.members.iter().any(|member| member.tag == "ITA" && member.partial), "an unattributed part makes the member and its side partial");
        assert!(!casualties.opposing.partial);
        assert_eq!(casualties.opposing.share_of_war, Some(0), "a side with no part has a known zero share");
        let participation = all.iter().find(|measure| measure.key == WarMeasureKey::Participation).unwrap();
        assert_eq!(participation.viewpoint.share_of_war, None, "participation is already a share of the side");
        assert_eq!(participation.viewpoint.members[0].share_of_side, Some(6_920));
        let received = all.iter().find(|measure| measure.key == WarMeasureKey::LendLeaseReceived).unwrap();
        assert_eq!(received.viewpoint.total.as_deref(), Some("-600000"), "the recorded sign is kept in totals; shares use magnitudes");
        let nothing = all.iter().find(|measure| measure.key == WarMeasureKey::Occupation).unwrap();
        assert_eq!((nothing.viewpoint.share_of_war, nothing.viewpoint.members[0].share_of_side), (None, None), "no part anywhere is no share, not zero");
    }
}
