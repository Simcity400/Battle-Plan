//! How the triggers' value readers read an operand token.

use installed_content_core::{
    fixed5::IntOperand, fixed5::decimal_fixed5, fixed5::decimal_i32, fixed5::int_operand,
    fixed5::script_fixed5, parser::Item, parser::Operator, parser::Value, parser::read_bool,
};

use super::{Scopes, eval::Evaluator};
use crate::state_resources::ONE;

/// How a value reads in a refusal: its text, or that it is a block.
pub fn shown(value: &Value) -> String {
    match value.scalar() {
        Some(text) => String::from_utf8_lossy(text).into_owned(),
        None => "{ ... }".into(),
    }
}

/// A scope selector found by the value's keyword ID,
/// as country operands read it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selector {
    /// `this`, `root`, `prev`, `from`: that
    /// scope.
    This,
    Root,
    Prev,
    From,
    /// `owner`, `controller` and `occupied`:
    /// the state of THIS.
    StateOwner,
    StateController,
    StateOccupier,
    /// `capital` and `random`, which country selection does not
    /// test: link 0.
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

    /// The selector a bare value names, found without case.
    pub fn named(text: &[u8]) -> Option<Self> {
        Self::NAMES
            .iter()
            .find(|(name, _)| text.eq_ignore_ascii_case(name.as_bytes()))
            .map(|(_, selector)| *selector)
    }
}

/// What scope selection finds in a value token, before a
/// trigger's own reading of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selected<'a> {
    /// A selector keyword, by the token's keyword ID: never a quoted token
    /// or a number ([`Item::value_keyword`]).
    Keyword(Selector),
    /// `event_target:` compared with the text's start, stopping at a NUL,
    /// whatever the token kind.
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

/// The one reading of a value token as a scope selector, which
/// bool triggers and country operands share.
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

/// A boolean trigger operand, also used by scripted triggers:
/// the expected value is cleared, then the token is read as a scope selector
/// ([`scope_selector`]); only if it is none, as a boolean
/// comparing `yes` and `no` with their terminating NUL,
/// so the text to its first NUL, quoted or bare. Any other value, an empty
/// one, a block's `{` or another spelling of yes or no, fails that read,
/// so the expected value stays false and the trigger tests `no`;
/// assignment only logs the failure. A selector is
/// refused: what the trigger then tests is not reproduced.
pub fn trigger_boolean(item: &Item) -> Result<bool, String> {
    if let Some(selector) = scope_selector(item) {
        return Err(format!(
            "the operand {} is {}, which the trigger's reader takes instead of yes or no; what the trigger then tests is not reproduced",
            shown(&item.value),
            selector.describe()
        ));
    }
    // Boolean reading compares bytes; a failed read leaves `no`.
    Ok(item.value.scalar().and_then(read_bool).unwrap_or(false))
}

/// Integer comparisons preserve equality; fixed comparisons
/// select `<` only for that token and otherwise take
/// the strict `>` arm.
pub fn compare(actual: i64, required: i64, comparison: Operator, fixed: bool) -> bool {
    match comparison {
        Operator::Less => actual < required,
        Operator::Greater => actual > required,
        Operator::Equals if fixed => actual > required,
        Operator::Equals => actual == required,
    }
}

impl Evaluator<'_, '_> {
    /// A comparison trigger's operand, fixed5, in `scopes`. Integer comparison
    /// reads a number token through `%i` and any other, a quoted
    /// one included, as a scoped variable (a constant through
    /// installed_content_core::fixed5::int_operand);
    /// it compares the scoped value as
    /// `(int)(value / 100000)`. Fixed comparison
    /// takes every token as a scoped variable and compares the
    /// fixed5 value. A scoped variable is evaluated in the
    /// trigger's scope.
    pub fn required(
        &self,
        bytes: &[u8],
        quoted: bool,
        fixed: bool,
        scopes: Scopes,
    ) -> Result<i64, String> {
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

    /// A mean-time modifier's `factor` or `add` reads each occurrence in
    /// order, using the scoped-variable reader shared by fixed comparison
    /// operands: empty text is
    /// logged and sets nothing, any other text is parsed into the one
    /// operand, evaluated in
    /// the mean time's scope. `default` is the modifier's own.
    ///
    /// A later parse does not reset every field an earlier one set (the temp
    /// flag, the scope chain), so only a last literal, which evaluation
    /// returns before reading any other field, is exact after another
    /// assignment; any other repeated assignment is refused by name.
    pub(super) fn mtth_operand(
        &self,
        what: &str,
        items: &[Item],
        key: &str,
        default: i64,
        scopes: Scopes,
    ) -> Result<i64, String> {
        let texts: Vec<&[u8]> = items
            .iter()
            .filter(|item| item.keyword_is(key))
            .filter_map(|item| item.value.scalar())
            .filter(|text| !text.is_empty())
            .collect();
        let Some(&text) = texts.last() else {
            return Ok(default);
        };
        let within = |why: String| {
            format!(
                "{what} modifier {key} = {}: {why}",
                String::from_utf8_lossy(text)
            )
        };
        if texts.len() > 1 && self.operand(text).map_err(within)?.literal.is_none() {
            return Err(within(super::refused(format!(
                "{key} is assigned {} times, and a later assignment into the one operand keeps fields an earlier one set, which is not reproduced",
                texts.len()
            ))));
        }
        self.scoped_value(text, scopes).map_err(within)
    }
}

/// A mean-time base value: 100000 initially,
/// then each field in order: `factor`
/// as fixed5 (`%lli`), `base` as a decimal,
/// and `days`, `months` and `years` as decimal integers
/// times 1, 30 and 365 in 32-bit arithmetic. A `factor` that does not scan
/// is reported and leaves the value as it was.
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
