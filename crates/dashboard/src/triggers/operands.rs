//! How the triggers' value readers read an operand token.

use installed_content::{
    fixed5::{IntOperand, decimal_fixed5, decimal_i32, int_operand, script_fixed5},
    parser::{Item, Operator, Value, read_bool},
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

/// A scope selector ParseScopeSelector (14053E330) finds by the value's
/// keyword ID, as CTrigger::ResolveCountryScope (140542000) reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selector {
    /// `this` 0x29C3, `root` 0x2C94, `prev` 0x2C95, `from` 0x298F: that
    /// scope.
    This,
    Root,
    Prev,
    From,
    /// `owner` 0x283E (bit 0x10), `controller` 0x283F (0x40) and `occupied`
    /// 0x3DB1 (0x20): the state of THIS.
    StateOwner,
    StateController,
    StateOccupier,
    /// `capital` 0x284B and `random` 0x27BB, whose bits 140542000 does not
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

    /// The selector a bare value names, found without case (1424A7A90).
    pub fn named(text: &[u8]) -> Option<Self> {
        Self::NAMES
            .iter()
            .find(|(name, _)| text.eq_ignore_ascii_case(name.as_bytes()))
            .map(|(_, selector)| *selector)
    }
}

/// What ParseScopeSelector (14053E330) finds in a value token, before a
/// trigger's own reading of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selected {
    /// A selector keyword, by the token's keyword ID: never a quoted token
    /// (0xF) or a number (0xC) (1424A7A90, [`Item::value_keyword`]).
    Keyword(Selector),
    /// `event_target:` compared with the text's start (strncmp 14253B770,
    /// which stops at a NUL), whatever the token kind.
    EventTarget,
}

impl Selected {
    pub fn describe(self) -> &'static str {
        match self {
            Self::Keyword(_) => "a scope selector keyword (14053E330)",
            Self::EventTarget => "an event target (14053E330)",
        }
    }
}

/// The one reading of a value token as a scope selector (14053E330), which
/// the bool triggers (14138D3D0) and the country operand (141394CA0) share.
pub fn scope_selector(item: &Item) -> Option<Selected> {
    if let Some(keyword) = item.value_keyword()
        && let Some(selector) = Selector::named(keyword.as_bytes())
    {
        return Some(Selected::Keyword(selector));
    }
    item.value
        .scalar()?
        .starts_with(b"event_target:")
        .then_some(Selected::EventTarget)
}

/// A CBoolTrigger operand (14138D3D0, the reader CScriptedTrigger shares):
/// +0x58 is cleared, then the value token is read as a scope selector
/// ([`scope_selector`]); only if it is none, by read_bool_token
/// (1424B06E0), which compares `yes` and `no` with their terminating NUL,
/// so the text to its first NUL, quoted or bare. Any other value, an empty
/// one, a block's `{` or another spelling of yes or no, fails that read,
/// so +0x58 stays false and the trigger tests `no`
/// (CSimpleAssignTrigger::vf5 141395A40 only logs it). A selector is
/// refused: what the trigger then tests is not reproduced.
pub fn trigger_boolean(item: &Item) -> Result<bool, String> {
    if let Some(selector) = scope_selector(item) {
        return Err(format!(
            "the operand {} is {}, which the trigger's reader (14138D3D0) takes instead of yes or no; what the trigger then tests is not reproduced",
            shown(&item.value),
            selector.describe()
        ));
    }
    // read_bool_token (1424B06E0) compares bytes; a failed read leaves `no`.
    Ok(item.value.scalar().and_then(read_bool).unwrap_or(false))
}

/// CIntTrigger::Evaluate (14138CC70) preserves equality; CValueTrigger's
/// (14138D120, `fixed`) selects `<` only for that token and otherwise takes
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
    /// A comparison trigger's operand, fixed5, in `scopes`. CIntTrigger::vf4
    /// (14138D4E0) reads a number token through `%i` and any other, a quoted
    /// one included, as a scoped variable (a constant through
    /// ParseFixed5String, installed_content::fixed5::int_operand);
    /// CIntTrigger::Evaluate (14138CC70) compares the scoped value as
    /// `(int)(value / 100000)`. CValueTrigger's reader (`fixed`, 14138D630)
    /// takes every token as a scoped variable (140539370) and compares the
    /// fixed5 value (14138D120). A scoped variable is evaluated in the
    /// trigger's scope (CScopedVariable::Evaluate 1405382B0).
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
            IntOperand::Unreadable => {
                Err("the operand is a number `%i` (1424B02C0) cannot scan".into())
            }
            IntOperand::Empty => Err(
                "the operand is empty: the scoped-variable reader (140539B50) logs it and sets nothing"
                    .into(),
            ),
            IntOperand::Scoped => self.scoped_value(bytes, scopes).map(scoped),
        }
    }

    /// A CMTTHModifier's `factor` or `add` as it reads each occurrence in
    /// order (140545820 -> CScopedVariable::vf3 140539440 -> 140539B50, the
    /// scoped-variable reader CValueTrigger's operands share): empty text is
    /// logged and sets nothing, any other text is parsed into the one
    /// operand, which CMeanTimeToHappen::Evaluate (1405455A0) evaluates in
    /// the mean time's scope. `default` is the modifier's own.
    ///
    /// A later parse does not reset every field an earlier one set (the temp
    /// flag, the scope chain), so only a last literal, which Evaluate
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
                "{key} is assigned {} times, and a later assignment into the one operand keeps fields an earlier one set (140539B50), which is not reproduced",
                texts.len()
            ))));
        }
        self.scoped_value(text, scopes).map_err(within)
    }
}

/// A CMeanTimeToHappen's base value as its reader keeps it: 100000 from its
/// constructor (140544B60), then each field in order (140545850): `factor`
/// through ReadFixed5 (1424ABB10, `%lli`), `base` as a decimal (1424AFF10),
/// and `days`, `months` and `years` through ParseTokenInt32 (1424B0080)
/// times 1, 30 and 365 in 32-bit arithmetic. A `factor` that does not scan
/// is reported (1424ACEE0) and leaves the value as it was.
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
