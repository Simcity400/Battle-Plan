//! Temp triggers execute over the shared TLS store, not persistent variables.

use super::*;
use installed_content::parser::Value as ParsedValue;

enum Right<'v> {
    Scalar(Box<ScopedVariable>),
    Expression(&'v ParsedValue),
}

// AssignScopedVariableFromTokenText@140539370 parses into the existing
// operand. Empty text changes nothing; a repeated nonliteral can keep fields.
fn assign(evaluator: &Evaluator, operand: &mut ScopedVariable, text: &[u8]) -> Result<(), String> {
    if text.is_empty() {
        return Ok(());
    }
    let next = evaluator.operand(text)?;
    if operand.parsed && next.literal.is_none() {
        return Err(super::super::refused(format!(
            "repeated variable operand {} keeps fields from its earlier read (140539B50), which is not reproduced",
            String::from_utf8_lossy(text)
        )));
    }
    *operand = next;
    Ok(())
}

fn zero() -> ScopedVariable {
    // Constructor@14014AAC0: literal zero, but parsed (+8) is clear.
    ScopedVariable {
        literal: Some(0),
        ..ScopedVariable::default()
    }
}

impl Evaluator<'_, '_> {
    pub(in crate::triggers) fn temp_operation(
        &self,
        name: &str,
        item: &Item,
        scopes: Scopes,
        uncertain: bool,
    ) -> Option<Result<(), String>> {
        if !matches!(
            name,
            "set_temp_variable"
                | "add_to_temp_variable"
                | "subtract_from_temp_variable"
                | "multiply_temp_variable"
                | "divide_temp_variable"
                | "clamp_temp_variable"
                | "round_temp_variable"
                | "modulo_temp_variable"
        ) {
            return None;
        }
        if uncertain {
            let why = super::super::refused(format!(
                "{name}: its execution depends on an unknown earlier condition"
            ));
            self.taint_temps(why);
            // These setters return true even when their execution is optional.
            return Some(Ok(()));
        }
        let result = self.set_temp(name, item, scopes);
        // A failed key read may name any destination. Do not retain old temps.
        if let Err(why) = &result {
            self.taint_temps(format!("trigger {name}: {why}"));
        }
        Some(result)
    }

    fn set_temp(&self, name: &str, item: &Item, scopes: Scopes) -> Result<(), String> {
        let mut var = zero();
        let mut right = Right::Scalar(Box::new(zero()));
        let mut min = zero();
        let mut max = zero();
        if name == "round_temp_variable" {
            // The scalar reader@141385570 already consumed this value.
            assign(self, &mut var, item.value.scalar().unwrap_or_default())?;
        } else {
            // Consume the native reader's Items; no token/block reader here.
            // CVariableTrigger@14043B2C0 differs from the effect divide reader:
            // `if_zero` is shorthand here, not an extra divide field.
            for field in item.value.items().unwrap_or_default() {
                let key = field.keyword().map(|key| key.folded());
                let scalar = field.value.scalar().unwrap_or_default();
                if name == "clamp_temp_variable" {
                    // CClampVariableTrigger::load_key@140437640.
                    match key.as_deref() {
                        Some(b"var") => assign(self, &mut var, scalar)?,
                        Some(b"min") => assign(self, &mut min, scalar)?,
                        Some(b"max") => assign(self, &mut max, scalar)?,
                        _ => {}
                    }
                    continue;
                }
                match key.as_deref() {
                    Some(b"tooltip") if name != "modulo_temp_variable" => continue,
                    Some(b"var") => {
                        assign(self, &mut var, scalar)?;
                        continue;
                    }
                    Some(b"value") => {}
                    _ => assign(self, &mut var, field.key.as_deref().unwrap_or_default())?,
                }
                // Modulo's 140480E00 / 140481020 reader takes one token,
                // including shorthand `tooltip`; it has no CExpression.
                if field.value.items().is_some() && name != "modulo_temp_variable" {
                    right = Right::Expression(&field.value);
                } else {
                    match &mut right {
                        Right::Scalar(operand) => assign(self, operand, scalar)?,
                        Right::Expression(_) => return Err(super::super::refused(
                            "scalar RHS after a CExpression calls bad_variant_access (1403BA3C0)"
                                .into(),
                        )),
                    }
                }
            }
        }
        // GetMutableValue@140535A10 receives TLS explicitly: scope prefixes
        // never select a persistent destination. It inserts BEFORE the RHS.
        if var.literal.is_some() || var.token.is_some() || var.random || var.ai_random {
            return Err(super::super::refused(
                "temp destination is read-only; GetMutableValue asserts (140535A10)".into(),
            ));
        }
        let key = self.key(&var, scopes, None)?;
        // Even an existing key evaluates its initial/default operand first.
        let initial = match var.default.as_deref() {
            Some(default) => self
                .evaluate_operand(default, scopes, None)
                .and_then(|v| self.raw(v))?,
            None => 0,
        };
        let current = self.temp(&key, scopes).and_then(|v| match v {
            Some(v) => self.raw(v),
            None => Ok(initial),
        });
        self.local_temps
            .borrow_mut()
            .insert(key.clone(), current.map(Value::Raw));
        // RHS evaluation can itself reach temp-writing alias conditions.
        let current = || {
            self.temp(&key, scopes)
                .and_then(|value| self.raw(value.unwrap_or(Value::Raw(initial))))
        };
        let value = match name {
            "round_temp_variable" => rounded(current()?), // 1403DC6A0 -> 1424D87B0
            "clamp_temp_variable" => {
                // 1403CF5F0 reads min then max before clamping max then min.
                let lower = if min.parsed {
                    Some(
                        self.evaluate_operand(&min, scopes, None)
                            .and_then(|v| self.raw(v))?,
                    )
                } else {
                    None
                };
                let upper = if max.parsed {
                    Some(
                        self.evaluate_operand(&max, scopes, None)
                            .and_then(|v| self.raw(v))?,
                    )
                } else {
                    None
                };
                let value = current()?;
                let value = upper.map_or(value, |upper| value.min(upper));
                lower.map_or(value, |lower| value.max(lower))
            }
            "modulo_temp_variable" => {
                // ApplyScriptVariableModulo@14047B280 caches the floored,
                // narrowed target BEFORE evaluating/rounding the divisor.
                let numerator = modulo_integer(current()?);
                if numerator < 0 {
                    return Ok(());
                }
                let Right::Scalar(operand) = right else {
                    unreachable!()
                };
                let divisor = if operand.parsed {
                    self.evaluate_operand(&operand, scopes, None)
                        .and_then(|v| self.raw(v))?
                } else {
                    0
                };
                let divisor = fixed_round(divisor) as i32;
                if divisor <= 0 {
                    // A RHS alias may have changed the target. No store here.
                    return Ok(());
                }
                i64::from(numerator % divisor) * ONE
            }
            _ => {
                let rhs = match right {
                    // The constructor's unparsed operand is literal zero.
                    Right::Scalar(operand) if !operand.parsed => 0,
                    Right::Scalar(operand) => self
                        .evaluate_operand(&operand, scopes, None)
                        .and_then(|v| self.raw(v))?,
                    Right::Expression(value) => self.temp_expression(value, scopes)?,
                };
                match name {
                    "set_temp_variable" => rhs,                             // 1403DC700
                    "add_to_temp_variable" => current()?.wrapping_add(rhs), // 1403CE2A0
                    "subtract_from_temp_variable" => current()?.wrapping_sub(rhs), // 1403DCD50
                    "multiply_temp_variable" => current()?.wrapping_mul(rhs) / ONE, // 1403DB1C0
                    "divide_temp_variable" => {
                        let value = current()?;
                        // 1403D0360 logs zero and leaves the inserted value.
                        if rhs == 0 {
                            value
                        } else {
                            quotient(value, rhs)?
                        }
                    }
                    _ => unreachable!(),
                }
            }
        };
        self.local_temps
            .borrow_mut()
            .insert(key, Ok(Value::Raw(value)));
        Ok(())
    }

    /// Core CExpression instructions@1414052C0, from the shared math reader.
    /// Unported operators are refused where reached, including lazy operators.
    fn temp_expression(&self, expression: &ParsedValue, scopes: Scopes) -> Result<i64, String> {
        let Some(fields) = expression.items() else {
            return self.scoped_value(expression.scalar().unwrap_or_default(), scopes);
        };
        let Some(first) = fields.first().filter(|field| field.keyword_is("value")) else {
            return Err(super::super::refused(
                "CExpression without its initial value (141406A00)".into(),
            ));
        };
        let mut value = self.temp_expression(&first.value, scopes)?;
        for field in &fields[1..] {
            let key = field.keyword().map(|key| key.folded()).unwrap_or_default();
            if matches!(key.as_slice(), b"round" | b"not") && field.value.scalar() != Some(b"yes") {
                return Err(super::super::refused(format!(
                    "CExpression unary {} requires yes (1414072C0)",
                    String::from_utf8_lossy(&key)
                )));
            }
            if key == b"round" {
                value = rounded(value);
                continue;
            }
            if key == b"not" {
                value = if value == 0 { ONE } else { 0 };
                continue;
            }
            if !matches!(
                key.as_slice(),
                b"add"
                    | b"subtract"
                    | b"multiply"
                    | b"divide"
                    | b"min"
                    | b"max"
                    | b"greater_than"
                    | b"less_than"
                    | b"greater_than_or_equals"
                    | b"less_than_or_equals"
                    | b"equals"
                    | b"not_equals"
            ) {
                return Err(super::super::refused(format!(
                    "CExpression operator {} (1414052C0) is not reproduced",
                    String::from_utf8_lossy(&key)
                )));
            }
            let rhs = self.temp_expression(&field.value, scopes)?;
            value = match key.as_slice() {
                b"add" => value.wrapping_add(rhs),
                b"subtract" => value.wrapping_sub(rhs),
                b"multiply" => value.wrapping_mul(rhs) / ONE,
                b"divide" => {
                    if rhs == 0 {
                        0xffff_ffff
                    } else {
                        quotient(value, rhs)?
                    }
                }
                b"min" => value.min(rhs),
                b"max" => value.max(rhs),
                b"greater_than" => i64::from(value > rhs) * ONE,
                b"less_than" => i64::from(value < rhs) * ONE,
                b"greater_than_or_equals" => i64::from(value >= rhs) * ONE,
                b"less_than_or_equals" => i64::from(value <= rhs) * ONE,
                b"equals" => i64::from(value == rhs) * ONE,
                b"not_equals" => i64::from(value != rhs) * ONE,
                _ => unreachable!(),
            };
        }
        Ok(value)
    }
}

fn rounded(value: i64) -> i64 {
    // 1424D87B0: the shared fixed_round result, returned as fixed5.
    fixed_round(value).wrapping_mul(ONE)
}

fn modulo_integer(value: i64) -> i32 {
    // 14047B280 floors in fixed5, wraps that product, divides, then narrows.
    let whole = value / ONE;
    let floored = if value % ONE == 0 {
        value
    } else {
        (whole + (value >> 63)).wrapping_mul(ONE)
    };
    (floored / ONE) as i32
}

fn quotient(value: i64, rhs: i64) -> Result<i64, String> {
    // Divide@1403D0360 / CExpression@1414052C0 sentinel guard.
    if matches!(value, i64::MIN | i64::MAX) || value == i64::MIN + 1 {
        return Ok(0xffff_ffff);
    }
    value.wrapping_mul(ONE).checked_div(rhs).ok_or_else(|| {
        super::super::refused(
            "fixed5 divide faults on native signed quotient overflow (1403D0360)".into(),
        )
    })
}
