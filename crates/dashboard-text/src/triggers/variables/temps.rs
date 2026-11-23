use crate::triggers::Failure;

use super::*;
use content_reader_parser::parser::Value as ParsedValue;

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

pub(in crate::triggers) fn assign(
    evaluator: &Evaluator,
    operand: &mut ScopedVariable,
    text: &[u8],
) -> Result<(), Failure> {
    if text.is_empty() {
        return Ok(());
    }
    let next = evaluator.operand(text)?;
    if operand.parsed {
        if next.literal.is_some() {
            operand.literal = next.literal;
            return Ok(());
        }
        return Err(crate::triggers::refusal::refused(format!(
            "repeated variable operand {} keeps fields from its earlier read, which is not reproduced",
            String::from_utf8_lossy(text)
        )));
    }
    *operand = next;
    Ok(())
}

pub(in crate::triggers) fn zero() -> ScopedVariable {
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
    ) -> Option<Result<(), Failure>> {
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
        if uncertain && !self.in_colour_callback() {
            let why = crate::triggers::refusal::refused(format!(
                "{name}: its execution depends on an unknown earlier condition"
            ));
            self.taint_temps(why);
            return Some(Ok(()));
        }
        let result = if name == "set_temp_variable_to_random" {
            self.description_random_temp(item, scopes)
        } else {
            self.variable_operands(name, item, false)
                .and_then(|operands| self.set_temp(name, &operands, scopes, false))
        };
        if !self.in_colour_callback()
            && let Err(why) = &result
        {
            self.taint_temps(why.clone().context(|why| format!("trigger {name}: {why}")));
        }
        Some(result)
    }

    fn variable_operands<'v>(
        &self,
        name: &str,
        item: &'v Item,
        effect: bool,
    ) -> Result<Operands<'v>, Failure> {
        let mut var = zero();
        let mut right = Right::Scalar(Box::new(zero()));
        let mut min = zero();
        let mut max = zero();
        let mut if_zero = zero();
        if name == "round_temp_variable" {
            assign(self, &mut var, item.value.scalar().unwrap_or_default())?;
        } else {
            for field in item.value.items().unwrap_or_default() {
                let key = field.keyword().map(|key| key.folded());
                let scalar = field.value.scalar().unwrap_or_default();
                if effect && name == "divide_temp_variable" && key.as_deref() == Some(b"if_zero") {
                    assign(self, &mut if_zero, scalar)?;
                    continue;
                }
                if name == "clamp_temp_variable" {
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
                if field.value.items().is_some() && name != "modulo_temp_variable" {
                    right = Right::Expression(&field.value);
                } else {
                    match &mut right {
                        Right::Scalar(operand) => assign(self, operand, scalar)?,
                        Right::Expression(_) => {
                            return Err(crate::triggers::refusal::refused(
                                "scalar RHS after an expression has an incompatible value form",
                            ));
                        }
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
    ) -> Result<Vec<u8>, Failure> {
        if var.literal.is_some() || var.token.is_some() || var.random || var.ai_random {
            return Err(crate::triggers::refusal::refused(
                "temp destination is read-only; mutable lookup asserts",
            ));
        }
        let key = self.key(var, scopes, index)?;
        let initial = match var.default.as_deref() {
            Some(default) => self
                .evaluate_operand(default, scopes, None)
                .and_then(|v| self.raw(v))?,
            None => 0,
        };
        let current = crate::triggers::refusal::deferred(|| {
            self.temp(&key, scopes).and_then(|v| match v {
                Some(v) => self.raw(v),
                None => Ok(initial),
            })
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
    ) -> Result<(), Failure> {
        let Operands {
            var,
            right,
            min,
            max,
            if_zero,
        } = operands;
        let key = self.mutable_temp(var, scopes, None)?;
        let current = || {
            self.temp(&key, scopes)
                .and_then(|value| self.raw(value.unwrap_or(Value::Raw(0))))
        };
        let value = match name {
            "round_temp_variable" => rounded(current()?),
            "clamp_temp_variable" => {
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
                    return Ok(());
                }
                i64::from(numerator % divisor) * ONE
            }
            _ => {
                let rhs = self.variable_right(right, scopes)?;
                match name {
                    "set_temp_variable" => rhs,
                    "add_to_temp_variable" => current()?.wrapping_add(rhs),
                    "subtract_from_temp_variable" => current()?.wrapping_sub(rhs),
                    "multiply_temp_variable" => current()?.wrapping_mul(rhs) / ONE,
                    "divide_temp_variable" => {
                        let fallback = if effect {
                            Some(self.temp_fallback(if_zero, scopes)?)
                        } else {
                            None
                        };
                        if rhs == 0 && !effect {
                            return Ok(());
                        }
                        let value = current()?;
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

    fn variable_right(&self, right: &Right<'_>, scopes: Scopes) -> Result<i64, Failure> {
        match right {
            Right::Scalar(operand) if !operand.parsed => Ok(0),
            Right::Scalar(operand) => self
                .evaluate_operand(operand, scopes, None)
                .and_then(|v| self.raw(v)),
            Right::Expression(value) => self.temp_expression(value, scopes),
        }
    }

    fn temp_fallback(&self, operand: &ScopedVariable, scopes: Scopes) -> Result<i64, Failure> {
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

    pub fn description_variable(
        &self,
        name: &str,
        item: &Item,
        scopes: Scopes,
        temporary: bool,
        tooltip: bool,
    ) -> Result<Option<(i64, i64)>, Failure> {
        let operands = self
            .variable_operands(name, item, true)
            .inspect_err(|why| {
                if temporary {
                    self.taint_temps(why.clone().context(|why| format!("effect {name}: {why}")));
                }
            })?;
        if temporary && let Err(why) = self.set_temp(name, &operands, scopes, true) {
            self.taint_temps(why.clone().context(|why| format!("effect {name}: {why}")));
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

    pub fn description_temp_destination_fields(
        &self,
        fields: &[Item],
        name: &str,
        default: &[u8],
        scopes: Scopes,
        index: Option<&ScopedVariable>,
    ) -> Result<Vec<u8>, Failure> {
        let mut operand = zero();
        for field in fields.iter().filter(|field| field.keyword_is(name)) {
            assign(self, &mut operand, field.value.scalar().unwrap_or_default())?;
        }
        if !operand.parsed {
            assign(self, &mut operand, default)?;
        }
        self.mutable_temp(&operand, scopes, index)
    }

    pub fn store_description_temp(&self, key: Vec<u8>, value: i64) {
        self.note_variable_effect();
        self.local_temps
            .borrow_mut()
            .insert(key, Ok(Value::Raw(value)));
    }

    pub fn encode_description_scope(&self, scope: Scope) -> Result<i64, Failure> {
        self.encode(scope)
    }

    pub fn refuse_description_writer(&self, reason: String) {
        self.taint_temps(reason.into());
    }

    pub fn description_random_temp(
        &self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<(), Failure> {
        let mut min = ScopedVariable::literal(0);
        let mut max = ScopedVariable::literal(ONE);
        let mut var = zero();
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
        let integer = content_reader_parser::parser::read_bool_field(
            item.value.items().unwrap_or_default(),
            "integer",
            false,
        );
        let value = if integer {
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

    pub fn description_supply_temp(
        &self,
        item: &Item,
        scopes: Scopes,
        minimum_trains: impl FnOnce() -> Result<i64, Failure>,
    ) -> Result<(), Failure> {
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
        let need = content_reader_parser::parser::read_bool_field(
            item.value.items().unwrap_or_default(),
            "need",
            false,
        );
        if !matches!(kind.as_deref(), Some("truck" | "train")) {
            return Ok(());
        }
        if kind.as_deref() == Some("train") && !need {
            return Err("get_supply_vehicles_temp actual train count is not recorded".into());
        }
        let country = match scopes.this {
            Scope::Country(country) | Scope::CountryLeader { country, .. } => country,
            Scope::Combat => self.combat_country()?,
            _ => {
                return Err("get_supply_vehicles_temp needs the caller's country".into());
            }
        };
        let raw = self
            .world()
            .raw_country(country)
            .ok_or("supply-temp country is not recorded")?;
        let row = super::super::world::table::<replay_data::rows::tables::TrainInventory>(
            self.world().tables(),
            replay_data::rows::TableId::TrainInventory.full_mask(),
        )?
        .get(u128::from(raw))
        .map_err(|why| {
            super::super::refusal::input_error(replay_data::rows::TableId::TrainInventory, why)
        })?
        .ok_or_else(|| -> Failure {
                format!("train inventory lacks supply-temp country {raw}").into()
            })?;
        let value = match (kind.as_deref(), need) {
            (Some("truck"), false) => i64::from(*super::super::refusal::optional(
                &row.allocated_trucks,
                replay_data::rows::TableId::TrainInventory,
                "allocated_trucks",
                Some(Store::Country(country)),
            )?)
            .wrapping_mul(ONE),
            (Some("truck"), true) => {
                let raw = *super::super::refusal::optional(
                    &row.required_trucks_raw,
                    replay_data::rows::TableId::TrainInventory,
                    "required_trucks_raw",
                    Some(Store::Country(country)),
                )?;
                let rounded = if raw % ONE == 0 {
                    raw
                } else {
                    (raw / ONE + i64::from(raw > 0)).wrapping_mul(ONE)
                };
                i64::from((rounded / ONE) as i32).wrapping_mul(ONE)
            }
            (Some("train"), true) => {
                let count = *super::super::refusal::optional(
                    &row.required_trains,
                    replay_data::rows::TableId::TrainInventory,
                    "required_trains",
                    Some(Store::Country(country)),
                )?;
                let raw = i64::from(count).wrapping_mul(ONE);
                if raw > minimum_trains()? { raw } else { 0 }
            }
            _ => unreachable!(),
        };
        self.store_description_temp(destination, value);
        Ok(())
    }

    fn temp_expression(&self, expression: &ParsedValue, scopes: Scopes) -> Result<i64, Failure> {
        let Some(fields) = expression.items() else {
            return self.scoped_value(expression.scalar().unwrap_or_default(), scopes);
        };
        let Some(first) = fields.first().filter(|field| field.keyword_is("value")) else {
            return Err(crate::triggers::refusal::refused(
                "expression without its initial value",
            ));
        };
        let mut value = self.temp_expression(&first.value, scopes)?;
        for field in &fields[1..] {
            let key = field.keyword().map(|key| key.folded()).unwrap_or_default();
            if matches!(key.as_slice(), b"round" | b"not") && field.value.scalar() != Some(b"yes") {
                return Err(crate::triggers::refusal::refused(format!(
                    "expression unary {} requires yes",
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
            if key == b"clamp" {
                let bounds = field.value.items().ok_or("expression clamp needs bounds")?;
                let bound = |name: &str| {
                    let field = bounds
                        .iter()
                        .find(|field| field.keyword_is(name))
                        .ok_or_else(|| -> Failure {
                            format!("expression clamp lacks {name}").into()
                        })?;
                    self.temp_expression(&field.value, scopes)
                };
                let minimum = bound("min")?;
                let maximum = bound("max")?;
                value = if value < minimum {
                    minimum
                } else if value > maximum {
                    maximum
                } else {
                    value
                };
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
                return Err(crate::triggers::refusal::refused(format!(
                    "expression operator {} is not reproduced",
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
    fixed_round(value).wrapping_mul(ONE)
}

fn modulo_integer(value: i64) -> i32 {
    let whole = value / ONE;
    let floored = if value % ONE == 0 {
        value
    } else {
        (whole + (value >> 63)).wrapping_mul(ONE)
    };
    (floored / ONE) as i32
}

fn quotient(value: i64, rhs: i64) -> Result<i64, Failure> {
    if matches!(value, i64::MIN | i64::MAX) || value == i64::MIN + 1 {
        return Ok(0xffff_ffff);
    }
    value
        .wrapping_mul(ONE)
        .checked_div(rhs)
        .ok_or_else(|| -> Failure {
            crate::triggers::refusal::refused("fixed5 divide faults on signed quotient overflow")
        })
}
