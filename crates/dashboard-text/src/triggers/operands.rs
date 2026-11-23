use crate::triggers::Failure;

use content_reader_parser::{fixed5::{IntOperand, decimal_fixed5, decimal_i32, int_operand, script_fixed5}, parser::{Item, Operator, Value, read_bool}};

use super::{Scopes, eval::Evaluator};
use crate::state_resources::ONE;

pub fn shown(value: &Value) -> String {
    match value.scalar() {
        Some(text) => String::from_utf8_lossy(text).into_owned(),
        None => "{ ... }".into(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selector {
    This,
    Root,
    Prev,
    From,
    StateOwner,
    StateController,
    StateOccupier,
    None,
}

impl Selector {
    const NAMES: [(&'static str, Selector); 9] = [
        ("this", Selector::This),
        ("root", Selector::Root),
        ("prev", Selector::Prev),
        ("from", Selector::From),
        ("owner", Selector::StateOwner),
        ("controller", Selector::StateController),
        ("occupied", Selector::StateOccupier),
        ("capital", Selector::None),
        ("random", Selector::None),
    ];

    pub fn named(text: &[u8]) -> Option<Self> {
        Self::NAMES
            .iter()
            .find(|(name, _)| text.eq_ignore_ascii_case(name.as_bytes()))
            .map(|(_, selector)| *selector)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selected<'a> {
    Keyword(Selector),
    EventTarget(&'a [u8]),
}

impl Selected<'_> {
    pub fn describe(self) -> &'static str {
        match self {
            Self::Keyword(_) => "a scope selector keyword",
            Self::EventTarget(_) => "an event target",
        }
    }
}

pub fn scope_selector(item: &Item) -> Option<Selected<'_>> {
    if let Some(keyword) = item.value_keyword()
        && let Some(selector) = Selector::named(keyword.as_bytes())
    {
        return Some(Selected::Keyword(selector));
    }
    item.value
        .scalar()?
        .strip_prefix(b"event_target:")
        .map(Selected::EventTarget)
}

pub fn trigger_boolean(item: &Item) -> Result<bool, Failure> {
    if let Some(selector) = scope_selector(item) {
        return Err(format!(
            "the operand {} is {}, which the trigger's reader takes instead of yes or no; what the trigger then tests is not reproduced",
            shown(&item.value),
            selector.describe()
        ).into());
    }
    Ok(item.value.scalar().and_then(read_bool).unwrap_or(false))
}

pub fn compare(actual: i64, required: i64, comparison: Operator, fixed: bool) -> bool {
    match comparison {
        Operator::Less => actual < required,
        Operator::Greater => actual > required,
        Operator::Equals if fixed => actual > required,
        Operator::Equals => actual == required,
    }
}

impl Evaluator<'_, '_> {
    pub fn required(
        &self,
        bytes: &[u8],
        quoted: bool,
        fixed: bool,
        scopes: Scopes,
    ) -> Result<i64, Failure> {
        let scoped = |value: i64| {
            if fixed {
                value
            } else {
                i64::from((value / ONE) as i32) * ONE
            }
        };
        match int_operand(bytes, quoted || fixed) {
            IntOperand::Integer(value) => Ok(i64::from(value) * ONE),
            IntOperand::Constant(value) => Ok(scoped(value)),
            IntOperand::Unreadable => Err("the operand is a number `%i` cannot scan".into()),
            IntOperand::Empty => Err(
                "the operand is empty: the scoped-variable reader logs it and sets nothing".into(),
            ),
            IntOperand::Scoped => self.scoped_value(bytes, scopes).map(scoped),
        }
    }

    pub(super) fn mtth_operand(
        &self,
        what: &str,
        items: &[Item],
        key: &str,
        default: i64,
        scopes: Scopes,
    ) -> Result<i64, Failure> {
        let texts: Vec<&[u8]> = items
            .iter()
            .filter(|item| item.keyword_is(key))
            .filter_map(|item| item.value.scalar())
            .filter(|text| !text.is_empty())
            .collect();
        let Some(&text) = texts.last() else {
            return Ok(default);
        };
        let within = |why: Failure| {
            why.context(|why| {
                format!(
                    "{what} modifier {key} = {}: {why}",
                    String::from_utf8_lossy(text)
                )
            })
        };
        if texts.len() > 1 && self.operand(text).map_err(within)?.literal.is_none() {
            return Err(within(crate::triggers::refusal::refused(format!(
                "{key} is assigned {} times, and a later assignment into the one operand keeps fields an earlier one set, which is not reproduced",
                texts.len()
            ))));
        }
        self.scoped_value(text, scopes).map_err(within)
    }
}

pub fn mtth_base(items: &[Item]) -> i64 {
    let mut base = ONE;
    for item in items {
        let (Some(keyword), Some(text)) = (item.keyword(), item.value.scalar()) else {
            continue;
        };
        base = match keyword.folded().as_slice() {
            b"factor" => script_fixed5(text).unwrap_or(base),
            b"base" => decimal_fixed5(text),
            unit @ (b"days" | b"months" | b"years") => {
                let count = decimal_i32(text);
                let days = match unit {
                    b"days" => count,
                    b"months" => count.wrapping_mul(30),
                    _ => count.wrapping_mul(365),
                };
                i64::from(days) * ONE
            }
            _ => continue,
        };
    }
    base
}
