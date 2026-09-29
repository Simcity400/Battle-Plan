//! Reinforcement's movement factor, `140C76E90 -> 140C70670(1, 1, nullptr)`.
//!
//! This call ignores terrain and combat movement penalties. It is not the
//! route's effective speed and must not inherit route organisation/fuel terms.
//! The collector preserves the dependency graph; all arithmetic below belongs
//! to playback. Native fixed5 products wrap at 64 bits before signed division.
//! Every refusal names the recorded operand, scope or division that failed.
use super::fixed::{EXCEPTIONAL, ONE, TRAP, mul, ratio};
use replay::rows::{LandCombatDivisionRow, LandReinforcementModifier};

/// The factor, or which recorded operand refuses it and why.
type Speed = Result<i64, String>;

struct Inputs<'a> {
    row: &'a LandCombatDivisionRow,
}

/// A fixed5 division that traps natively, named by what it divides.
fn divided(what: &str, quotient: Option<i64>) -> Speed {
    quotient.ok_or_else(|| format!("the {what} division: {TRAP}"))
}

impl Inputs<'_> {
    fn terms(&self, scope: u8) -> impl Iterator<Item = &LandReinforcementModifier> {
        self.row
            .reinforcement_modifiers
            .iter()
            .filter(move |term| term.scope == scope)
    }

    fn term(&self, scope: u8, key: u32) -> Result<&LandReinforcementModifier, String> {
        let mut terms = self.terms(scope).filter(|term| term.key == key);
        let term = terms
            .next()
            .ok_or_else(|| format!("modifier scope {scope} records no term {key:#x}"))?;
        match terms.count() {
            0 => Ok(term),
            more => Err(format!(
                "modifier scope {scope} records term {key:#x} {} times",
                more + 1
            )),
        }
    }

    fn effective(&self, term: &LandReinforcementModifier, raw: i64) -> i64 {
        let ids = &self.row.reinforcement.ids;
        if term.present == 0
            || (term.feature_selector != 0 && (ids[6] == 0 || term.feature_selector & ids[5] == 0))
        {
            0
        } else if term.metadata_flags & 0x10 != 0 {
            raw.clamp(0, ONE)
        } else {
            raw
        }
    }

    fn value(&self, scope: u8, key: u32) -> Speed {
        let term = self.term(scope, key)?;
        Ok(self.effective(term, term.raw))
    }

    /// Entirely absent scopes mean a null native leader/weather operand;
    /// a partially present scope cannot turn an omitted key into numeric zero.
    fn optional(&self, scope: u8, key: u32) -> Speed {
        if self.terms(scope).next().is_none() {
            Ok(0)
        } else {
            self.value(scope, key)
        }
    }

    /// The scope holds exactly `keys` (or nothing, when `optional`), or
    /// which keys it holds instead.
    fn exact_keys(&self, scope: u8, keys: &[u32], optional: bool) -> Result<(), String> {
        let count = self.terms(scope).count();
        if (optional && count == 0)
            || (count == keys.len() && keys.iter().all(|&key| self.term(scope, key).is_ok()))
        {
            return Ok(());
        }
        let hex = |keys: &mut dyn Iterator<Item = u32>| {
            keys.map(|key| format!("{key:#x}"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        Err(format!(
            "modifier scope {scope} records the terms [{}], not exactly [{}]{}",
            hex(&mut self.terms(scope).map(|term| term.key)),
            hex(&mut keys.iter().copied()),
            if optional { " or none" } else { "" }
        ))
    }
}

/// A recorded flag or id outside the values the native code distinguishes.
fn check(name: &str, value: impl Into<i64> + Copy, most: i64) -> Result<(), String> {
    let value = value.into();
    if value > most {
        return Err(format!(
            "its recorded {name} is {value}, outside 0..={most}"
        ));
    }
    Ok(())
}

/// Numeric movement modifier (fixed5), before the caller multiplies base speed
/// and compares it with `SLOWEST_SPEED`. `Err` names the partial or invalid
/// operand, or the arithmetic division the native code would trap on.
pub(super) fn modifier(row: &LandCombatDivisionRow) -> Speed {
    let context = &row.reinforcement;
    if context.ids[0] == 0 {
        return Err("its reinforcement operands record no division (ids[0] is 0)".into());
    }
    check("feature flag ids[6]", context.ids[6], 1)?;
    check("supply mode flags[11]", context.flags[11], 2)?;
    if let Some(term) = row
        .reinforcement_modifiers
        .iter()
        .find(|term| term.scope > 9 || term.key == 0 || term.present > 1)
    {
        return Err(format!(
            "its modifier term {:#x} records scope {}, presence {}, outside the recorded forms",
            term.key, term.scope, term.present
        ));
    }
    let inputs = Inputs { row };
    if context.flags[11] == 2 {
        return transport(&inputs);
    }
    for (name, index, most) in [
        ("weather flag", 4, 1),
        ("attacking flag", 5, 1),
        ("controller flag", 6, 2),
        ("air flag", 7, 1),
        ("supply penalty flag", 8, 1),
        ("infrastructure flag", 9, 1),
    ] {
        check(
            &format!("{name} flags[{index}]"),
            context.flags[index],
            most,
        )?;
    }
    inputs.exact_keys(0, &[0x2b, 0x1e9, 0x5e, 0x17f, 0x286], false)?;
    inputs.exact_keys(1, &[0x2b, 0x5e, 0x17f], true)?;
    inputs.exact_keys(2, &[0x2b, 0x1e9, 0x1e7, 0x1e8], context.ids[2] == 0)?;
    if context.ids[2] == 0 && inputs.terms(2).next().is_some() {
        return Err(
            "modifier scope 2 records terms although the division has no controller operand (ids[2] is 0)"
                .into(),
        );
    }
    inputs.exact_keys(3, &[0x2b, 0x1e9], true)?;
    inputs.exact_keys(4, &[0x2b, 0x1e9], true)?;
    if inputs.terms(5).count() > 1 {
        return Err(format!(
            "modifier scope 5 records {} priority terms, not at most one",
            inputs.terms(5).count()
        ));
    }
    if let Some(term) = row
        .reinforcement_modifiers
        .iter()
        .find(|term| term.scope > 6)
    {
        return Err(format!(
            "its land modifier term {:#x} records the transport scope {}",
            term.key, term.scope
        ));
    }
    let attacking = context.flags[5] != 0;
    let mut value = ONE
        .wrapping_add(inputs.value(0, 0x2b)?)
        .wrapping_add(inputs.optional(1, 0x2b)?);
    if attacking {
        value = value.wrapping_add(inputs.value(0, 0x1e9)?);
    }
    if context.ids[2] != 0 {
        value = value.wrapping_add(inputs.value(2, 0x2b)?);
        if attacking {
            value = value.wrapping_add(inputs.value(2, 0x1e9)?);
        }
        let controller_key = match context.flags[6] {
            1 => Some(0x1e7),
            2 => Some(0x1e8),
            _ => None,
        };
        if let Some(key) = controller_key {
            value = value.wrapping_add(inputs.value(2, key)?);
        }
    }
    value = value.wrapping_add(weather(&inputs)?);
    // The priority subunit's own speed key, then each category in native order.
    // Repeated category keys remain repeated contributions.
    for term in inputs.terms(5).chain(inputs.terms(6)) {
        value = value.wrapping_add(inputs.effective(term, term.raw));
    }
    value = value.wrapping_add(air(&inputs)?);
    value = value.wrapping_add(supply(&inputs)?);
    value = value.wrapping_add(infrastructure(&inputs)?);
    if context.flags[0] != 0 {
        value = value
            .wrapping_add(context.scalar_values[11])
            .wrapping_add(inputs.value(0, 0x286)?);
    } else if context.flags[1] != 0 {
        value = value.wrapping_add(context.scalar_values[12]);
    }
    Ok(value)
}

fn weather(inputs: &Inputs<'_>) -> Speed {
    let context = &inputs.row.reinforcement;
    let mut factor = ONE;
    if context.flags[4] != 0 {
        let mut winner = None;
        let mut maximum = 0;
        for (index, term) in inputs.row.reinforcement_acclimatization.iter().enumerate() {
            if term.active > 1 {
                return Err(format!(
                    "its acclimatization term {index} records activity {}, not 0 or 1",
                    term.active
                ));
            }
            if term.weight > maximum {
                maximum = term.weight;
                winner = Some(term);
            }
        }
        if winner.is_some_and(|term| term.active != 0) {
            let fraction = divided(
                "acclimatization weight",
                ratio(maximum, context.scalar_values[0], EXCEPTIONAL),
            )?;
            let adjustment = mul(fraction, context.scalar_values[1]).clamp(0, ONE);
            factor = ONE.wrapping_sub(adjustment);
        }
    }
    let mut value = 0_i64;
    for scope in [3, 4] {
        value = value.wrapping_add(mul(factor, inputs.optional(scope, 0x2b)?));
        if context.flags[5] != 0 {
            value = value.wrapping_add(mul(factor, inputs.optional(scope, 0x1e9)?));
        }
    }
    Ok(value)
}

fn air(inputs: &Inputs<'_>) -> Speed {
    let context = &inputs.row.reinforcement;
    let s = &context.scalar_values;
    if context.flags[7] == 0 || s[14] <= 0 {
        return Ok(0);
    }
    let sum = s[13].wrapping_add(s[14]);
    let share = if sum > 0 {
        divided("enemy air share", ratio(s[14], sum, EXCEPTIONAL))?
    } else {
        0
    };
    let penalty = mul(mul(s[15], share), s[17]);
    let reduction =
        engine::csub_unit_definition::movement_air_defense_value(s[16], s[18], s[19])
            .map_err(|reason| format!("the air-defence reduction cannot be evaluated: {reason}"))?;
    Ok(penalty.wrapping_add(reduction).min(0))
}

fn supply(inputs: &Inputs<'_>) -> Speed {
    let context = &inputs.row.reinforcement;
    if context.flags[2] != 0 {
        return if context.flags[11] == 0 {
            Ok(0)
        } else {
            Err(format!(
                "it is supplied (flags[2]) yet records supply mode {}",
                context.flags[11]
            ))
        };
    }
    if context.flags[11] != 1 {
        return Err(format!(
            "it is out of supply yet records supply mode {}, not 1",
            context.flags[11]
        ));
    }
    let s = &context.scalar_values;
    // These native source fields are i32; preserve their sign extension.
    for (index, value) in s.iter().enumerate().take(8).skip(5) {
        i32::try_from(*value).map_err(|_| {
            format!("its supply operand {index} is {value}, outside the native i32 field")
        })?;
    }
    let supplied = if s[2] <= 0 {
        0
    } else {
        let grace = divided(
            "supply grace",
            ratio(s[3], s[6].wrapping_mul(ONE), EXCEPTIONAL),
        )?;
        s[4].max(grace).clamp(0, ONE)
    };
    let days = s[5].wrapping_mul(ONE);
    let denominator = s[7].wrapping_mul(ONE);
    let days_ratio = if denominator == 0 || days <= i64::MIN + 1 {
        EXCEPTIONAL
    } else {
        divided(
            "supply days",
            days.wrapping_mul(ONE).checked_div(denominator),
        )?
    };
    let mut severity = mul(ONE.wrapping_sub(supplied), days_ratio);
    if severity != 0 {
        let mut total = 0_i32;
        let mut special = 0_i32;
        for term in &inputs.row.reinforcement_supply {
            total = total.wrapping_add(term.count);
            if term.special != 0 {
                special = special.wrapping_add(term.count);
            }
        }
        let denominator = i64::from(total).wrapping_mul(ONE);
        let share = if denominator == 0 {
            0
        } else {
            divided(
                "special supply share",
                i64::from(special)
                    .wrapping_mul(ONE)
                    .wrapping_mul(ONE)
                    .checked_div(denominator),
            )?
        };
        let mut factor = ONE.wrapping_add(inputs.value(0, 0x5e)?);
        if share > 0 {
            factor = factor.wrapping_add(mul(share, inputs.value(0, 0x17f)?));
        }
        factor = factor.wrapping_add(inputs.optional(1, 0x5e)?);
        if share > 0 {
            factor = factor.wrapping_add(mul(share, inputs.optional(1, 0x17f)?));
        }
        severity = mul(factor.max(0), severity);
    }
    let mut value = mul(s[8], severity);
    if context.flags[8] != 0 {
        value = value.wrapping_add(s[9]);
    }
    Ok(value)
}

fn infrastructure(inputs: &Inputs<'_>) -> Speed {
    let context = &inputs.row.reinforcement;
    if context.flags[9] == 0 {
        return if context.ids[3] == 0 {
            Ok(0)
        } else {
            Err(format!(
                "it records the infrastructure operand {} without its infrastructure flag",
                context.ids[3]
            ))
        };
    }
    if context.ids[3] == 0 {
        return Err("its infrastructure flag is set without an infrastructure operand".into());
    }
    let s = &context.scalar_values;
    let maximum = if s[20] > 0 {
        s[20]
    } else if s[21] > 0 {
        s[21]
    } else {
        s[22]
    };
    let maximum = i32::try_from(maximum).map_err(|_| {
        format!("its infrastructure maximum {maximum} is outside the native i32 field")
    })?;
    let level = if context.flags[10] == 0 {
        0
    } else {
        i32::from(i16::try_from(s[24]).map_err(|_| {
            format!(
                "its infrastructure level {} is outside the native i16 field",
                s[24]
            )
        })?)
    };
    Ok((i64::from(maximum.wrapping_sub(level))
        .wrapping_mul(s[23])
        .wrapping_mul(ONE)
        / ONE)
        .max(-95_000))
}

fn transport(inputs: &Inputs<'_>) -> Speed {
    let context = &inputs.row.reinforcement;
    inputs.exact_keys(7, &[0x29], true)?;
    inputs.exact_keys(8, &[0x29], true)?;
    inputs.exact_keys(9, &[0x1a7], true)?;
    if let Some(term) = inputs
        .row
        .reinforcement_modifiers
        .iter()
        .find(|term| term.scope < 7)
    {
        return Err(format!(
            "its transport modifiers record the land term {:#x} in scope {}",
            term.key, term.scope
        ));
    }
    let value = ONE.wrapping_add(inputs.optional(7, 0x29)?);
    let source = inputs.term(8, 0x29).ok();
    let reduction_term = inputs.term(9, 0x1a7).ok();
    if source.is_some() != reduction_term.is_some() {
        return Err(format!(
            "its transport records {} of the two associated modifier scopes 8 and 9",
            if source.is_some() {
                "scope 8 without 9"
            } else {
                "scope 9 without 8"
            }
        ));
    }
    if source.is_none()
        && (!inputs.row.reinforcement_transport.is_empty()
            || context.scalar_values[25..=29]
                .iter()
                .any(|&value| value != 0))
    {
        // These operands are captured only after a static definition exists.
        // Losing both associated modifier scopes is still an incomplete graph.
        return Err(
            "its transport operands are recorded without the associated modifier scopes 8 and 9"
                .into(),
        );
    }
    if context.ids[5] & (1 << 4) == 0 {
        return if source.is_none() {
            Ok(value)
        } else {
            Err(
                "its transport modifier scopes are recorded without the feature bit (ids[5] bit 4)"
                    .into(),
            )
        };
    }
    let Some(source) = source else {
        return Ok(value);
    };
    let s = &context.scalar_values;
    let category_mask = u32::try_from(s[29])
        .map_err(|_| format!("its transport category mask {} is outside u32", s[29]))?;
    let mut total = 0_i64;
    let mut weighted = 0_i64;
    for term in &inputs.row.reinforcement_transport {
        total = total.wrapping_add(term.regional_value);
        weighted = weighted.wrapping_add(mul(
            term.regional_value,
            term.own_intel.wrapping_sub(term.enemy_intel),
        ));
    }
    let average = divided("regional intel average", ratio(weighted, total, 0))?;
    // Native ordered clamps preserve even a negative definition maximum.
    let density = if total < 0 {
        0
    } else if total > s[25] {
        s[25]
    } else {
        total
    };
    let mut amount = divided("transport density", ratio(density, s[25], EXCEPTIONAL))?;
    if amount != 0 {
        let mut reduction = inputs.value(9, 0x1a7)?;
        let difference = average.wrapping_sub(s[26]);
        if difference > 0 {
            let range = s[27].wrapping_sub(s[26]);
            let capped = difference.min(range);
            let scaled = mul(capped, s[28]);
            reduction = reduction.wrapping_add(divided("intel range", ratio(scaled, range, 0))?);
        }
        if reduction != 0 {
            amount = mul(ONE.wrapping_sub(reduction), amount).clamp(0, ONE);
        }
    }
    if amount > 0 && category_mask != 0 && source.present != 0 {
        // 140E908E0 composes the raw static entry before 141956170 asks the
        // ordinary metadata-gated getter; clamping before multiplication differs.
        Ok(value.wrapping_add(inputs.effective(source, mul(amount, source.raw))))
    } else {
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::rows::{
        LandReinforcementAcclimatization, LandReinforcementSupplyTerm,
        LandReinforcementTransportTerm,
    };

    fn terms(scope: u8, keys: &[u32]) -> Vec<LandReinforcementModifier> {
        keys.iter()
            .map(|&key| LandReinforcementModifier {
                scope,
                key,
                ..Default::default()
            })
            .collect()
    }

    fn normal() -> LandCombatDivisionRow {
        let mut row = LandCombatDivisionRow::default();
        row.reinforcement.ids[0] = 1;
        row.reinforcement.flags[2] = 1;
        row.reinforcement_modifiers = terms(0, &[0x2b, 0x1e9, 0x5e, 0x17f, 0x286]);
        row
    }

    fn set(row: &mut LandCombatDivisionRow, scope: u8, key: u32, value: i64) {
        let term = row
            .reinforcement_modifiers
            .iter_mut()
            .find(|term| term.scope == scope && term.key == key)
            .unwrap();
        term.raw = value;
        term.present = 1;
    }

    #[test]
    fn absent_optional_sources_are_zero_but_partial_scopes_are_unavailable() {
        let mut row = normal();
        assert_eq!(modifier(&row), Ok(ONE));
        row.reinforcement_modifiers.extend(terms(3, &[0x2b]));
        assert_eq!(
            modifier(&row),
            Err(
                "modifier scope 3 records the terms [0x2b], not exactly [0x2b, 0x1e9] or none"
                    .into()
            )
        );
        assert!(modifier(&LandCombatDivisionRow::default()).is_err());
    }

    #[test]
    fn feature_gating_and_metadata_clamps_apply_before_land_addition() {
        let mut row = normal();
        set(&mut row, 0, 0x2b, 150_000);
        let term = &mut row.reinforcement_modifiers[0];
        term.metadata_flags = 0x10;
        term.feature_selector = 8;
        assert_eq!(modifier(&row), Ok(ONE));
        row.reinforcement.ids[6] = 1;
        row.reinforcement.ids[5] = 8;
        assert_eq!(modifier(&row), Ok(2 * ONE));
        row.reinforcement.ids[6] = 2;
        assert!(modifier(&row).is_err());
    }

    #[test]
    fn weather_preserves_first_maximum_and_scales_each_modifier_separately() {
        let mut row = normal();
        row.reinforcement_modifiers.extend(terms(3, &[0x2b, 0x1e9]));
        row.reinforcement.flags[4] = 1;
        row.reinforcement.scalar_values[0] = ONE;
        row.reinforcement.scalar_values[1] = ONE / 2;
        row.reinforcement_acclimatization = vec![
            LandReinforcementAcclimatization {
                weight: ONE,
                active: 0,
            },
            LandReinforcementAcclimatization {
                weight: ONE,
                active: 1,
            },
        ];
        set(&mut row, 3, 0x2b, -10_000);
        assert_eq!(modifier(&row), Ok(90_000));
        row.reinforcement_acclimatization.swap(0, 1);
        assert_eq!(modifier(&row), Ok(95_000));
        row.reinforcement.flags[5] = 1;
        set(&mut row, 3, 0x2b, 1);
        set(&mut row, 3, 0x1e9, 1);
        // Each 0.5*0.00001 truncates to zero; combining first would yield1.
        assert_eq!(modifier(&row), Ok(ONE));
    }

    #[test]
    fn supply_days_are_not_clamped_and_special_share_adjusts_the_factor() {
        let mut row = normal();
        row.reinforcement.flags[2] = 0;
        row.reinforcement.flags[11] = 1;
        row.reinforcement.flags[8] = 1;
        let s = &mut row.reinforcement.scalar_values;
        s[2] = ONE;
        s[4] = ONE / 2;
        s[5] = 10;
        s[6] = 24;
        s[7] = 5;
        s[8] = -50_000;
        s[9] = -10_000;
        row.reinforcement_supply = vec![
            LandReinforcementSupplyTerm {
                subunit: 1,
                count: 2,
                special: 0,
            },
            LandReinforcementSupplyTerm {
                subunit: 2,
                count: 2,
                special: 1,
            },
        ];
        set(&mut row, 0, 0x5e, -20_000);
        set(&mut row, 0, 0x17f, -40_000);
        assert_eq!(modifier(&row), Ok(60_000));
        // Exceptional grace division gives +0xffffffff, then clamps supplied1.
        row.reinforcement.scalar_values[6] = 0;
        assert_eq!(modifier(&row), Ok(90_000));
        row.reinforcement.flags[11] = 0;
        assert!(modifier(&row).is_err());
    }

    #[test]
    fn infrastructure_uses_healthy_level_and_retreat_takes_precedence() {
        let mut row = normal();
        row.reinforcement.ids[3] = 9;
        row.reinforcement.flags[9] = 1;
        row.reinforcement.flags[10] = 1;
        let s = &mut row.reinforcement.scalar_values;
        s[20] = 10;
        s[23] = -10_000;
        s[24] = 8;
        assert_eq!(modifier(&row), Ok(80_000));
        row.reinforcement.flags[0] = 1;
        row.reinforcement.flags[1] = 1;
        row.reinforcement.scalar_values[11] = 30_000;
        row.reinforcement.scalar_values[12] = 80_000;
        set(&mut row, 0, 0x286, 20_000);
        assert_eq!(modifier(&row), Ok(130_000));
    }

    #[test]
    fn air_defense_can_cancel_but_cannot_reverse_the_air_penalty() {
        let mut row = normal();
        row.reinforcement.flags[7] = 1;
        let s = &mut row.reinforcement.scalar_values;
        s[13] = ONE;
        s[14] = ONE;
        s[15] = ONE;
        s[16] = ONE;
        s[17] = -50_000;
        s[18] = ONE;
        s[19] = 20_000;
        assert_eq!(modifier(&row), Ok(85_000));
        row.reinforcement.scalar_values[19] = ONE;
        assert_eq!(modifier(&row), Ok(ONE));
    }

    #[test]
    fn transport_scales_raw_static_before_clamping_and_bypasses_land_inputs() {
        let mut row = LandCombatDivisionRow::default();
        row.reinforcement.ids[0] = 1;
        row.reinforcement.ids[5] = 1 << 4;
        row.reinforcement.flags[11] = 2;
        row.reinforcement_modifiers.extend(terms(8, &[0x29]));
        row.reinforcement_modifiers.extend(terms(9, &[0x1a7]));
        set(&mut row, 8, 0x29, 200_000);
        row.reinforcement_modifiers[0].metadata_flags = 0x10;
        let s = &mut row.reinforcement.scalar_values;
        s[25] = ONE;
        s[27] = ONE;
        s[29] = 1;
        row.reinforcement_transport
            .push(LandReinforcementTransportTerm {
                country: 1,
                regional_value: 50_000,
                own_intel: 0,
                enemy_intel: 0,
            });
        assert_eq!(modifier(&row), Ok(200_000));
        row.reinforcement_modifiers.pop();
        assert!(modifier(&row).is_err());
        row.reinforcement_modifiers.clear();
        assert!(modifier(&row).is_err());
    }
}
