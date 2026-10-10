//! Sum mapped modifiers in authored order; the warning displays the
//! complement. Counterplay uses slot 3, skips slot 6, and uses the smaller
//! output endpoint.

use crate::combat::text::localise;
use installed_content::localisation::Argument as Arg;
use installed_content::{
    localisation::Localisation,
    raids::tooltips::{Formula, Modifier},
};

/// Use two distinct truncating divisions, then clamp endpoints. Combining
/// the divisions changes the last decimal places.
pub fn mapped(m: &Modifier, raw: i64) -> Result<i64, String> {
    let denominator = m.reference.wrapping_sub(m.start_reference);
    let numerator = raw
        .wrapping_sub(m.start_reference)
        .wrapping_mul(m.weight.wrapping_sub(m.start_weight))
        / 100_000;
    let quotient = if denominator == 0 || !(-i64::MAX + 1..=i64::MAX - 1).contains(&numerator) {
        0xffff_ffff
    } else {
        numerator
            .wrapping_mul(100_000)
            .checked_div(denominator)
            .ok_or("native success mapping would overflow signed division")?
    };
    Ok(quotient
        .wrapping_add(m.start_weight)
        .clamp(m.start_weight.min(m.weight), m.start_weight.max(m.weight)))
}

pub struct Input {
    pub raw: Result<i64, String>,
    pub skip: Result<bool, String>,
    pub defender_can_affect: Result<bool, String>,
}

pub struct Evaluation {
    pub success: Result<i64, String>,
    pub counterplay: Vec<Result<String, String>>,
}

pub fn evaluate(
    loc: &Localisation,
    formula: &Formula,
    thresholds: Result<[i64; 2], String>,
    input: impl Fn(&Modifier) -> Input,
) -> Evaluation {
    let mut sum = Ok(formula.base);
    let mut counterplay = Vec::new();
    for modifier in &formula.modifiers {
        let input = input(modifier);
        let value = match &input.skip {
            Ok(true) => Ok(0),
            Ok(false) => input.raw.clone().and_then(|raw| mapped(modifier, raw)),
            Err(reason) => Err(reason.clone()),
        };
        sum = match (sum, value.clone()) {
            (Ok(sum), Ok(value)) => Ok(sum.wrapping_add(value)),
            (Err(reason), _) => Err(reason),
            (_, Err(reason)) => Err(format!("success modifier {}: {reason}", modifier.name)),
        };
        if matches!(input.defender_can_affect, Ok(false)) || matches!(input.skip, Ok(true)) {
            continue;
        }
        let line = (|| {
            input.defender_can_affect?;
            input.skip?;
            let value = value?;
            if value == modifier.weight.min(modifier.start_weight) {
                return Ok(None);
            }
            let raw = input.raw?;
            let thresholds = thresholds.clone()?;
            counterplay_text(loc, modifier, raw, value, thresholds).map(Some)
        })();
        match line {
            Ok(Some(line)) => counterplay.push(Ok(line)),
            Ok(None) => {}
            Err(reason) => counterplay.push(Err(format!(
                "success_modifier_{}_counterplay: {reason}",
                modifier.name
            ))),
        }
    }
    Evaluation {
        success: sum.map(|sum| 100_000 - sum.clamp(0, 100_000)),
        counterplay,
    }
}

fn effect(loc: &Localisation, value: i64, thresholds: [i64; 2]) -> Result<String, String> {
    // Thresholds are percentage points; pass the original fixed5 fraction
    // as the argument, separate from the scaled comparison operand.
    let percent = value.wrapping_mul(10_000_000) / 100_000;
    let key = if percent < thresholds[0] {
        "RAID_SUCCESS_MODIFIER_VALUE_BAD"
    } else if percent < thresholds[1] {
        "RAID_SUCCESS_MODIFIER_VALUE_NEUTRAL"
    } else {
        "RAID_SUCCESS_MODIFIER_VALUE_GOOD"
    };
    // Bind VAL including its trailing NUL.
    localise(loc, key, &[("VAL", Arg::Fixed5(value))])
}

fn counterplay_text(
    loc: &Localisation,
    m: &Modifier,
    raw: i64,
    value: i64,
    thresholds: [i64; 2],
) -> Result<String, String> {
    let prefix = format!("success_modifier_{}", m.name);
    let choose = |preferred: &str, fallback: &str| {
        let key = format!("{prefix}{preferred}");
        if loc.get(&key).is_some() {
            key
        } else {
            format!("{prefix}{fallback}")
        }
    };
    let key = choose("_counterplay", "_improvement");
    // Log a missing improvement key and return empty text.
    let mut out = if loc.get(&key).is_some() {
        localise(loc, &key, &[])?
    } else {
        String::new()
    };
    let negative = |value: i64| value.wrapping_mul(-100_000) / 100_000;
    let maximum = negative(m.weight.min(m.start_weight));
    let value = negative(value);
    // Use the raw endpoint belonging to the smaller mapped value.
    let endpoint = if m.weight <= m.start_weight {
        m.reference
    } else {
        m.start_reference
    };
    let source_key = choose("_source_counterplay", "_source");
    let source = if loc.get(&source_key).is_some() {
        localise(
            loc,
            &source_key,
            &[("VAL", Arg::Fixed5(raw)), ("MAX", Arg::Fixed5(endpoint))],
        )?
    } else {
        String::new()
    };
    let value = effect(loc, value, thresholds)?;
    let maximum = effect(loc, maximum, thresholds)?;
    // Bind VALUE (the source key uses VAL), MAX and SOURCE.
    let mut args = vec![("VALUE", Arg::Text(&value)), ("MAX", Arg::Text(&maximum))];
    let key = if source.is_empty() {
        "tooltip_raid_modifier_effect_improvement_no_source"
    } else {
        args.push(("SOURCE", Arg::Text(&source)));
        "tooltip_raid_modifier_effect_improvement"
    };
    let detail = localise(loc, key, &args)?;
    if !detail.is_empty() {
        super::ensure_newlines(&mut out, 1);
        out.push_str(&detail);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn modifier() -> Modifier {
        Modifier {
            name: "synthetic".into(),
            weight: 30_000,
            reference: 100_000,
            start_weight: -20_000,
            start_reference: 0,
            custom: Vec::new(),
        }
    }

    #[test]
    fn mapping_uses_both_divisions_reversed_endpoints_and_zero_divisor_sentinel() {
        let mut m = modifier();
        assert_eq!(mapped(&m, -1), Ok(-20_000));
        assert_eq!(mapped(&m, 50_000), Ok(5_000));
        assert_eq!(mapped(&m, 100_001), Ok(30_000));
        m.reference = 30_000;
        assert_eq!(mapped(&m, 1), Ok(-20_000)); // first division truncates to zero.
        m.reference = 0;
        assert_eq!(mapped(&m, 1), Ok(30_000));
        m.reference = -100_000;
        assert_eq!(mapped(&m, -50_000), Ok(5_000));
    }

    #[test]
    fn warning_complements_after_clamp_and_skips_only_applicable_modifiers() {
        let f = Formula {
            base: 90_000,
            modifiers: vec![modifier()],
        };
        let read = |skip| Input {
            raw: Ok(100_000),
            skip: Ok(skip),
            defender_can_affect: Ok(false),
        };
        assert_eq!(
            evaluate(&Localisation::default(), &f, Ok([0, 0]), |_| read(false)).success,
            Ok(0)
        );
        assert_eq!(
            evaluate(&Localisation::default(), &f, Ok([0, 0]), |_| read(true)).success,
            Ok(10_000)
        );
        let f = Formula {
            base: -100_000,
            modifiers: Vec::new(),
        };
        assert_eq!(
            evaluate(&Localisation::default(), &f, Ok([0, 0]), |_| unreachable!()).success,
            Ok(100_000)
        );
    }

    #[test]
    fn counterplay_filter_endpoint_and_missing_input_are_separate() {
        let f = Formula {
            base: 0,
            modifiers: vec![modifier(), modifier()],
        };
        let got = evaluate(&Localisation::default(), &f, Ok([0, 0]), |_| Input {
            raw: Err("absent".into()),
            skip: Ok(false),
            defender_can_affect: Ok(true),
        });
        assert!(got.success.is_err());
        assert_eq!(got.counterplay.len(), 2);
        let got = evaluate(&Localisation::default(), &f, Ok([0, 0]), |_| Input {
            raw: Ok(0),
            skip: Ok(false),
            defender_can_affect: Ok(true),
        });
        assert!(got.counterplay.is_empty());
    }

    #[test]
    fn counterplay_detail_takes_value_max_and_source_while_the_source_takes_val() {
        let loc: Localisation = [
            ("success_modifier_synthetic_counterplay", "counter"),
            ("success_modifier_synthetic_source", "$VAL|0$ of $MAX|0$"),
            ("RAID_SUCCESS_MODIFIER_VALUE_GOOD", "$VAL|0%$"),
            (
                "tooltip_raid_modifier_effect_improvement",
                "($VALUE$/$MAX$ from $SOURCE$)",
            ),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect();
        let f = Formula {
            base: 0,
            modifiers: vec![modifier()],
        };
        let got = evaluate(&loc, &f, Ok([i64::MIN; 2]), |_| Input {
            raw: Ok(50_000),
            skip: Ok(false),
            defender_can_affect: Ok(true),
        });
        assert_eq!(
            got.counterplay,
            [Ok("counter\n(-5%/20% from 0 of 0)".into())]
        );
    }
}
