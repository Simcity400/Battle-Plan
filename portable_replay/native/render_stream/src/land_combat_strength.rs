use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_COMMITTED_UNITS: usize = 128;
pub const D21_STRENGTH_FLAG: u32 = 1 << 11;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LandCombatStrengthUnit {
    pub division_id: u32,
    pub defense_raw: i64,
    pub breakthrough_raw: i64,
    pub hardness_raw: i64,
    pub soft_attack_raw: i64,
    pub hard_attack_raw: i64,
    pub attack_accumulator_raw: i64,
    pub attack_base_raw: i64,
    pub current_hp_raw: i64,
    pub maximum_hp_raw: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LandCombatStrengthSide {
    pub organisation_sum: i64,
    pub org_loss_sample_count: u32,
    pub combat_bonus_raw: i64,
    pub committed: Vec<LandCombatStrengthUnit>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LandCombatStrengthInputs {
    pub attacker: LandCombatStrengthSide,
    pub defender: LandCombatStrengthSide,
}

impl LandCombatStrengthInputs {
    pub fn validate(&self) -> Result<(), String> {
        for (name, side) in [("attacker", &self.attacker), ("defender", &self.defender)] {
            if side.organisation_sum < 0 || side.org_loss_sample_count > i32::MAX as u32 {
                return Err(format!(
                    "land strength {name} has invalid organisation operands"
                ));
            }
            if side.committed.len() > MAX_COMMITTED_UNITS {
                return Err(format!(
                    "land strength {name} committed vector exceeds its bound"
                ));
            }
            let mut seen = BTreeSet::new();
            if side
                .committed
                .iter()
                .any(|unit| unit.division_id == 0 || !seen.insert(unit.division_id))
            {
                return Err(format!(
                    "land strength {name} has zero or duplicate committed identity"
                ));
            }
        }
        Ok(())
    }
}

pub fn append_d21_strength(
    inputs: &LandCombatStrengthInputs,
    out: &mut Vec<u8>,
) -> Result<(), String> {
    inputs.validate()?;
    out.extend_from_slice(&inputs.attacker.combat_bonus_raw.to_le_bytes());
    out.extend_from_slice(&inputs.defender.combat_bonus_raw.to_le_bytes());
    out.extend_from_slice(&(inputs.attacker.committed.len() as u32).to_le_bytes());
    out.extend_from_slice(&(inputs.defender.committed.len() as u32).to_le_bytes());
    for side in [&inputs.attacker, &inputs.defender] {
        for unit in &side.committed {
            out.extend_from_slice(&unit.division_id.to_le_bytes());
            for raw in [
                unit.defense_raw,
                unit.breakthrough_raw,
                unit.hardness_raw,
                unit.soft_attack_raw,
                unit.hard_attack_raw,
                unit.attack_accumulator_raw,
                unit.attack_base_raw,
                unit.current_hp_raw,
                unit.maximum_hp_raw,
            ] {
                out.extend_from_slice(&raw.to_le_bytes());
            }
        }
    }
    Ok(())
}

pub fn decode_d21_strength(
    payload: &[u8],
) -> Result<BTreeMap<(u32, u64), LandCombatStrengthInputs>, String> {
    let u32_at = |at| {
        payload
            .get(at..at + 4)
            .and_then(|s| s.try_into().ok())
            .map(u32::from_le_bytes)
            .ok_or_else(|| "land strength truncated u32".to_string())
    };
    let u64_at = |at| {
        payload
            .get(at..at + 8)
            .and_then(|s| s.try_into().ok())
            .map(u64::from_le_bytes)
            .ok_or_else(|| "land strength truncated i64".to_string())
    };
    let combats = u32_at(8)? as usize;
    let participants = u32_at(20)? as usize;
    let trailer_len = u32_at(28)? as usize;
    let participants_start = combats
        .checked_mul(96)
        .and_then(|n| n.checked_add(32))
        .ok_or("land strength row size overflow")?;
    let trailer_start = participants
        .checked_mul(24)
        .and_then(|n| n.checked_add(participants_start))
        .ok_or("land strength participant size overflow")?;
    if trailer_start.checked_add(trailer_len) != Some(payload.len()) {
        return Err("land strength trailer length disagrees with payload".into());
    }
    let mut membership = BTreeSet::new();
    for at in (participants_start..trailer_start).step_by(24) {
        membership.insert((
            u32_at(at)?,
            u64_at(at + 4)?,
            u32_at(at + 12)?,
            u32_at(at + 16)?,
        ));
    }
    let mut cursor = trailer_start;
    let mut out = BTreeMap::new();
    for at in (32..participants_start).step_by(96) {
        let flags = u32_at(at + 12)?;
        if flags & D21_STRENGTH_FLAG == 0 {
            continue;
        }
        if flags & ((1 << 3) | (1 << 4) | (1 << 6)) != ((1 << 3) | (1 << 4) | (1 << 6)) {
            return Err(
                "land strength flag requires both participant lists and indicator operands".into(),
            );
        }
        let counts = [u32_at(at + 72)?, u32_at(at + 76)?];
        if counts.iter().all(|n| *n >= 24) {
            return Err("land strength stated without a young-combat comparison branch".into());
        }
        let key = (u32_at(at)?, u64_at(at + 4)?);
        let bonuses = [u64_at(cursor)? as i64, u64_at(cursor + 8)? as i64];
        let lengths = [u32_at(cursor + 16)? as usize, u32_at(cursor + 20)? as usize];
        cursor += 24;
        let mut sides = Vec::with_capacity(2);
        for side in 0..2 {
            if lengths[side] > MAX_COMMITTED_UNITS
                || lengths[side] > u32_at(at + 28 + side * 4)? as usize
            {
                return Err("land strength committed roster exceeds its participant bound".into());
            }
            let mut units = Vec::with_capacity(lengths[side]);
            for _ in 0..lengths[side] {
                let division_id = u32_at(cursor)?;
                if !membership.contains(&(key.0, key.1, side as u32 + 1, division_id)) {
                    return Err("land strength committed identity is outside its D21 side".into());
                }
                let unit = LandCombatStrengthUnit {
                    division_id,
                    defense_raw: u64_at(cursor + 4)? as i64,
                    breakthrough_raw: u64_at(cursor + 12)? as i64,
                    hardness_raw: u64_at(cursor + 20)? as i64,
                    soft_attack_raw: u64_at(cursor + 28)? as i64,
                    hard_attack_raw: u64_at(cursor + 36)? as i64,
                    attack_accumulator_raw: u64_at(cursor + 44)? as i64,
                    attack_base_raw: u64_at(cursor + 52)? as i64,
                    current_hp_raw: u64_at(cursor + 60)? as i64,
                    maximum_hp_raw: u64_at(cursor + 68)? as i64,
                };
                cursor += 76;
                units.push(unit);
            }
            sides.push(LandCombatStrengthSide {
                organisation_sum: u64_at(at + 56 + side * 8)? as i64,
                org_loss_sample_count: counts[side],
                combat_bonus_raw: bonuses[side],
                committed: units,
            });
        }
        let defender = sides.pop().unwrap();
        let attacker = sides.pop().unwrap();
        let inputs = LandCombatStrengthInputs { attacker, defender };
        inputs.validate()?;
        if out.insert(key, inputs).is_some() {
            return Err("land strength duplicate combat identity".into());
        }
    }
    if cursor != payload.len() {
        return Err("land strength trailer has unclaimed bytes".into());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strength_trailer_is_bounded_flagged_and_membership_checked() {
        let unit = LandCombatStrengthUnit {
            division_id: 7,
            defense_raw: 11,
            breakthrough_raw: 12,
            hardness_raw: 13,
            soft_attack_raw: 14,
            hard_attack_raw: 15,
            attack_accumulator_raw: 16,
            attack_base_raw: 17,
            current_hp_raw: 18,
            maximum_hp_raw: 19,
        };
        let inputs = LandCombatStrengthInputs {
            attacker: LandCombatStrengthSide {
                organisation_sum: 123,
                org_loss_sample_count: 23,
                combat_bonus_raw: 20,
                committed: vec![unit],
            },
            defender: LandCombatStrengthSide {
                organisation_sum: 456,
                org_loss_sample_count: 0,
                combat_bonus_raw: 21,
                committed: vec![],
            },
        };
        let mut payload = vec![0u8; 32 + 96 + 24];
        for (at, value) in [
            (8, 1),
            (20, 1),
            (32, 1),
            (44, D21_STRENGTH_FLAG | (1 << 3) | (1 << 4) | (1 << 6)),
            (60, 1),
            (104, 23),
            (128, 1),
            (140, 1),
            (144, 7),
        ] {
            payload[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        payload[36..44].copy_from_slice(&1u64.to_le_bytes());
        payload[132..140].copy_from_slice(&1u64.to_le_bytes());
        payload[88..96].copy_from_slice(&123i64.to_le_bytes());
        payload[96..104].copy_from_slice(&456i64.to_le_bytes());
        let mut trailer = Vec::new();
        append_d21_strength(&inputs, &mut trailer).unwrap();
        assert_eq!(trailer.len(), 100);
        payload[28..32].copy_from_slice(&(trailer.len() as u32).to_le_bytes());
        payload.extend_from_slice(&trailer);
        assert_eq!(decode_d21_strength(&payload).unwrap()[&(1, 1)], inputs);
        for end in 0..payload.len() {
            assert!(
                decode_d21_strength(&payload[..end]).is_err(),
                "truncation {end}"
            );
        }
        for (at, word, reason) in [
            (44, 0, "unclaimed"),
            (44, D21_STRENGTH_FLAG, "requires"),
            (168, 129, "bound"),
            (176, 8, "outside"),
        ] {
            let mut bad = payload.clone();
            bad[at..at + 4].copy_from_slice(&word.to_le_bytes());
            assert!(
                decode_d21_strength(&bad).unwrap_err().contains(reason),
                "mutation {at}"
            );
        }
    }

    #[test]
    fn strength_inputs_refuse_unknown_fields_and_unbounded_or_duplicate_rosters() {
        let unit = LandCombatStrengthUnit {
            division_id: 7,
            defense_raw: 0,
            breakthrough_raw: 0,
            hardness_raw: 0,
            soft_attack_raw: 0,
            hard_attack_raw: 0,
            attack_accumulator_raw: 0,
            attack_base_raw: 0,
            current_hp_raw: 0,
            maximum_hp_raw: 0,
        };
        let side = LandCombatStrengthSide {
            organisation_sum: 0,
            org_loss_sample_count: 0,
            combat_bonus_raw: 0,
            committed: vec![unit],
        };
        let mut inputs = LandCombatStrengthInputs {
            attacker: side.clone(),
            defender: side,
        };
        inputs.validate().unwrap();
        let value = serde_json::to_value(&inputs).unwrap();
        assert_eq!(
            serde_json::from_value::<LandCombatStrengthInputs>(value.clone()).unwrap(),
            inputs
        );
        let mut unknown = value;
        unknown["attacker"]["default_strength"] = serde_json::json!(100_000);
        assert!(serde_json::from_value::<LandCombatStrengthInputs>(unknown).is_err());
        inputs
            .attacker
            .committed
            .push(inputs.attacker.committed[0].clone());
        assert!(inputs.validate().unwrap_err().contains("duplicate"));
        inputs.attacker.committed = (1..=129)
            .map(|division_id| {
                let mut unit = inputs.defender.committed[0].clone();
                unit.division_id = division_id;
                unit
            })
            .collect();
        assert!(inputs.validate().unwrap_err().contains("bound"));
    }
}
