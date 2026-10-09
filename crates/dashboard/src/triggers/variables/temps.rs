//! Temp triggers execute over the shared TLS store, not persistent variables.

use super::*;
use installed_content::parser::Value as ParsedValue;

enum Right<'v> {
    Scalar(Box<ScopedVariable>),
    Expression(&'v ParsedValue),
}

struct Operands<'v> {
    var: ScopedVariable,
    right: Right<'v>,
    min: ScopedVariable,
    max: ScopedVariable,
    if_zero: ScopedVariable,
}

// AssignScopedVariableFromTokenText@140539370 parses into the existing
// operand. Empty text changes nothing; a repeated nonliteral can keep fields.
pub(super) fn assign(
    evaluator: &Evaluator,
    operand: &mut ScopedVariable,
    text: &[u8],
) -> Result<(), String> {
    if text.is_empty() {
        return Ok(());
    }
    let next = evaluator.operand(text)?;
    if operand.parsed {
        if next.literal.is_some() {
            // Numeric/constant paths@14053A999/14053A7A3 change +9/+0x40,
            // keeping the name/default/indices read by explicit TLS@1405389B0.
            operand.literal = next.literal;
            return Ok(());
        }
        return Err(super::super::refused(format!(
            "repeated variable operand {} keeps fields from its earlier read (140539B50), which is not reproduced",
            String::from_utf8_lossy(text)
        )));
    }
    *operand = next;
    Ok(())
}

pub(super) fn zero() -> ScopedVariable {
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
                | "set_temp_variable_to_random"
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
        let result = if name == "set_temp_variable_to_random" {
            self.description_random_temp(item, scopes)
        } else {
            self.variable_operands(name, item, false)
                .and_then(|operands| self.set_temp(name, &operands, scopes, false))
        };
        // A failed key read may name any destination. Do not retain old temps.
        if let Err(why) = &result {
            self.taint_temps(format!("trigger {name}: {why}"));
        }
        Some(result)
    }

    fn variable_operands<'v>(
        &self,
        name: &str,
        item: &'v Item,
        effect: bool,
    ) -> Result<Operands<'v>, String> {
        let mut var = zero();
        let mut right = Right::Scalar(Box::new(zero()));
        let mut min = zero();
        let mut max = zero();
        let mut if_zero = zero();
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
                if effect && name == "divide_temp_variable" && key.as_deref() == Some(b"if_zero") {
                    // CDivideTempVariableEffect reader@1403B2580 owns +0x220.
                    assign(self, &mut if_zero, scalar)?;
                    continue;
                }
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
        Ok(Operands {
            var,
            right,
            min,
            max,
            if_zero,
        })
    }

    fn mutable_temp(
        &self,
        var: &ScopedVariable,
        scopes: Scopes,
        index: Option<&ScopedVariable>,
    ) -> Result<Vec<u8>, String> {
        // GetMutableValue@140535A10 receives TLS explicitly: scope prefixes
        // never select a persistent destination. It inserts BEFORE the RHS.
        if var.literal.is_some() || var.token.is_some() || var.random || var.ai_random {
            return Err(super::super::refused(
                "temp destination is read-only; GetMutableValue asserts (140535A10)".into(),
            ));
        }
        let key = self.key(var, scopes, index)?;
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
        if !self.local_temps.borrow().contains_key(&key) {
            self.note_variable_effect();
        }
        self.local_temps
            .borrow_mut()
            .insert(key.clone(), current.map(Value::Raw));
        Ok(key)
    }

    fn set_temp(
        &self,
        name: &str,
        operands: &Operands<'_>,
        scopes: Scopes,
        effect: bool,
    ) -> Result<(), String> {
        let Operands {
            var,
            right,
            min,
            max,
            if_zero,
        } = operands;
        let key = self.mutable_temp(var, scopes, None)?;
        // RHS evaluation can itself reach temp-writing alias conditions.
        let current = || {
            self.temp(&key, scopes)
                .and_then(|value| self.raw(value.unwrap_or(Value::Raw(0))))
        };
        let value = match name {
            "round_temp_variable" => rounded(current()?), // 1403DC6A0 -> 1424D87B0
            "clamp_temp_variable" => {
                // Trigger/effect@1403CF5F0/14034EDD0: min then max; clamp max then min.
                let lower = if min.parsed {
                    Some(
                        self.evaluate_operand(min, scopes, None)
                            .and_then(|v| self.raw(v))?,
                    )
                } else {
                    None
                };
                let upper = if max.parsed {
                    Some(
                        self.evaluate_operand(max, scopes, None)
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
                    self.evaluate_operand(operand, scopes, None)
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
                let rhs = self.variable_right(right, scopes)?;
                match name {
                    "set_temp_variable" => rhs, // Trigger/effect@1403DC700/140364070.
                    "add_to_temp_variable" => current()?.wrapping_add(rhs), // 1403CE2A0/14034C050.
                    "subtract_from_temp_variable" => current()?.wrapping_sub(rhs), // 1403DCD50/140366420.
                    "multiply_temp_variable" => current()?.wrapping_mul(rhs) / ONE, // 1403DB1C0/14035B2B0.
                    "divide_temp_variable" => {
                        // Effect@1403554E0 reads if_zero through explicit TLS
                        // (1405389B0), even when the divisor is nonzero.
                        let fallback = if effect {
                            Some(self.temp_fallback(if_zero, scopes)?)
                        } else {
                            None
                        };
                        if rhs == 0 && !effect {
                            return Ok(()); // 1403D0360: trigger logs zero without a store.
                        }
                        let value = current()?;
                        // 1403D0360 logs zero and leaves the inserted value.
                        if rhs == 0 {
                            fallback.unwrap_or(value)
                        } else {
                            quotient(value, rhs)?
                        }
                    }
                    _ => unreachable!(),
                }
            }
        };
        self.store_description_temp(key, value);
        Ok(())
    }

    fn variable_right(&self, right: &Right<'_>, scopes: Scopes) -> Result<i64, String> {
        match right {
            Right::Scalar(operand) if !operand.parsed => Ok(0),
            Right::Scalar(operand) => self
                .evaluate_operand(operand, scopes, None)
                .and_then(|v| self.raw(v)),
            Right::Expression(value) => self.temp_expression(value, scopes),
        }
    }

    fn temp_fallback(&self, operand: &ScopedVariable, scopes: Scopes) -> Result<i64, String> {
        // 1405389B0 builds the key and evaluates the default, then looks up
        // only the supplied TLS store. It does not Evaluate a scalar literal.
        let key = self.key(operand, scopes, None)?;
        let default = operand
            .default
            .as_deref()
            .map(|default| {
                self.evaluate_operand(default, scopes, None)
                    .and_then(|v| self.raw(v))
            })
            .transpose()?
            .unwrap_or(0);
        self.temp(&key, scopes)
            .and_then(|v| self.raw(v.unwrap_or(Value::Raw(default))))
    }

    /// 140379020 executes temporary scalars before 14039A7D0 evaluates
    /// RIGHT then LEFT. Persistent scalars describe without executing.
    pub(crate) fn description_variable(
        &self,
        name: &str,
        item: &Item,
        scopes: Scopes,
        temporary: bool,
        tooltip: bool,
    ) -> Result<Option<(i64, i64)>, String> {
        let operands = self.variable_operands(name, item, true).map_err(|why| {
            if temporary {
                self.taint_temps(format!("effect {name}: {why}"));
            }
            why
        })?;
        if temporary && let Err(why) = self.set_temp(name, &operands, scopes, true) {
            self.taint_temps(format!("effect {name}: {why}"));
            return Err(why);
        }
        if !tooltip {
            return Ok(None);
        }
        let right = self.variable_right(&operands.right, scopes)?;
        let left = self
            .evaluate_operand(&operands.var, scopes, None)
            .and_then(|v| self.raw(v))?;
        Ok(Some((left, right)))
    }

    /// Scorer configuration readers@14047C500/14047C6C0 retain parsed
    /// operands across repeated fields and assign a default only if unparsed.
    /// GetMutableValue@140535A10 inserts TLS; the separate index overrides
    /// authored indices, including `num` for sorted lengths (14047AD60).
    pub(crate) fn description_temp_destination_fields(
        &self,
        fields: &[Item],
        name: &str,
        default: &[u8],
        scopes: Scopes,
        index: Option<&ScopedVariable>,
    ) -> Result<Vec<u8>, String> {
        let mut operand = zero();
        for field in fields.iter().filter(|field| field.keyword_is(name)) {
            assign(self, &mut operand, field.value.scalar().unwrap_or_default())?;
        }
        if !operand.parsed {
            assign(self, &mut operand, default)?;
        }
        self.mutable_temp(&operand, scopes, index)
    }

    pub(crate) fn store_description_temp(&self, key: Vec<u8>, value: i64) {
        self.note_variable_effect();
        self.local_temps
            .borrow_mut()
            .insert(key, Ok(Value::Raw(value)));
    }

    pub(crate) fn encode_description_scope(&self, scope: Scope) -> Result<i64, String> {
        self.encode(scope)
    }

    /// A failed mutating description must not fabricate later temp values.
    pub(crate) fn refuse_description_writer(&self, reason: String) {
        self.taint_temps(reason);
    }

    /// 14047BF80 -> 14047B530 evaluates min then max and returns before
    /// destination/RNG for a reversed range. Constructor@140479C50: 0..1.
    pub(crate) fn description_random_temp(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<(), String> {
        let mut min = ScopedVariable::literal(0);
        let mut max = ScopedVariable::literal(ONE);
        let mut var = zero();
        // Defaults are literal but unparsed, so the first authored read replaces them.
        min.parsed = false;
        max.parsed = false;
        for field in item.value.items().unwrap_or_default() {
            if field.keyword_is("min") {
                assign(self, &mut min, field.value.scalar().unwrap_or_default())?;
            } else if field.keyword_is("max") {
                assign(self, &mut max, field.value.scalar().unwrap_or_default())?;
            } else if field.keyword_is("var") {
                assign(self, &mut var, field.value.scalar().unwrap_or_default())?;
            }
        }
        let min = self
            .evaluate_operand(&min, scopes, None)
            .and_then(|v| self.raw(v))?;
        let max = self
            .evaluate_operand(&max, scopes, None)
            .and_then(|v| self.raw(v))?;
        if max < min {
            return Ok(());
        }
        let destination = self.mutable_temp(&var, scopes, None)?;
        let random = self.next_variable_random(scopes)?;
        let integer = installed_content::parser::read_bool_field(
            item.value.items().unwrap_or_default(),
            "integer",
            false,
        );
        let value = if integer {
            // 14047B530 floors/narrows each endpoint, then the scaled delta.
            let lower = modulo_integer(min);
            let upper = modulo_integer(max);
            let delta = i64::from(upper.wrapping_sub(lower))
                .wrapping_mul(ONE)
                .wrapping_mul(random)
                / ONE;
            i64::from(lower.wrapping_add(modulo_integer(delta))).wrapping_mul(ONE)
        } else {
            min.wrapping_add(max.wrapping_sub(min).wrapping_mul(random) / ONE)
        };
        self.store_description_temp(destination, value);
        Ok(())
    }

    /// 14047BEE0 -> 14047B0B0; reader@140480FD0. Only recognized bare
    /// truck/train tokens read a total; all other types store zero.
    pub(crate) fn description_supply_temp(
        &self,
        item: &Item,
        scopes: Scopes,
        minimum_trains: impl FnOnce() -> Result<i64, String>,
    ) -> Result<(), String> {
        let mut var = zero();
        let mut kind = None;
        for field in item.value.items().unwrap_or_default() {
            if field.keyword_is("var") {
                assign(self, &mut var, field.value.scalar().unwrap_or_default())?;
            } else if field.keyword_is("type") {
                kind = field.value_keyword();
            }
        }
        let destination = self.mutable_temp(&var, scopes, None)?;
        self.store_description_temp(destination.clone(), 0);
        let need = installed_content::parser::read_bool_field(
            item.value.items().unwrap_or_default(),
            "need",
            false,
        );
        if !matches!(kind.as_deref(), Some("truck" | "train")) {
            return Ok(());
        }
        if kind.as_deref() == Some("train") && !need {
            return Err("get_supply_vehicles_temp actual trains CCountrySupplySystem+0x110 i32 are not recorded (14047B0B0)".into());
        }
        let country = match scopes.this {
            Scope::Country(country) | Scope::CountryLeader { country, .. } => country,
            Scope::Combat => self.combat_country()?,
            _ => {
                return Err(
                    "get_supply_vehicles_temp needs the caller's country (14047B0B0)".into(),
                );
            }
        };
        let raw = self
            .world()
            .raw_country(country)
            .ok_or("supply-temp country is not recorded")?;
        let row = self
            .world()
            .frame()
            .complete::<replay::rows::tables::TrainInventory>()
            .map_err(|why| why.to_string())?
            .iter()
            .find(|row| row.country == raw)
            .ok_or_else(|| format!("TrainInventory lacks supply-temp country {raw}"))?;
        let value = match (kind.as_deref(), need) {
            (Some("truck"), false) => i64::from(
                row.allocated_trucks
                    .require("TrainInventory.allocated_trucks (CCountrySupplySystem+0x138 i32)")?
                    .ok_or("TrainInventory.allocated_trucks capture unavailable")?,
            )
            .wrapping_mul(ONE),
            (Some("truck"), true) => {
                let raw = row
                    .required_trucks_raw
                    .require(
                        "TrainInventory.required_trucks_raw (CSupplyCalculationData+0x130 i64)",
                    )?
                    .ok_or("TrainInventory.required_trucks_raw capture unavailable")?;
                // 14047C4A0 ceilings fixed5 (wrapping product); 14047B0B0
                // narrows the resulting whole count to i32 before scaling it.
                let rounded = if raw % ONE == 0 {
                    raw
                } else {
                    (raw / ONE + i64::from(raw > 0)).wrapping_mul(ONE)
                };
                i64::from((rounded / ONE) as i32).wrapping_mul(ONE)
            }
            (Some("train"), true) => {
                let count = row
                    .required_trains
                    .require("TrainInventory.required_trains (CSupplyCalculationData+0x140 i32)")?
                    .ok_or("TrainInventory.required_trains capture unavailable")?;
                let raw = i64::from(count).wrapping_mul(ONE);
                // 14047B0B0: strictly greater than the content define, else zero.
                if raw > minimum_trains()? { raw } else { 0 }
            }
            _ => unreachable!(),
        };
        self.store_description_temp(destination, value);
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
