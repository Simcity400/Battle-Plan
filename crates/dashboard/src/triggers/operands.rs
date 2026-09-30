//! How the triggers' value readers read an operand token.

use installed_content::{
    fixed5::{IntOperand, decimal_fixed5, decimal_i32, int_operand, script_fixed5},
    parser::{Item, Operator, Value, c_string},
};

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
/// so the text to its first NUL. Any other value, a quoted or empty one, a
/// block's `{` and another spelling of yes or no included, fails that read,
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
    Ok(item.value.scalar().map(c_string) == Some(b"yes"))
}

/// A comparison trigger's operand, fixed5. CIntTrigger::vf4 (14138D4E0)
/// reads a number token through `%i` and any other, a quoted one included,
/// as a scoped variable whose constant is ParseFixed5String's
/// (installed_content::fixed5::int_operand); CIntTrigger::Evaluate (14138CC70)
/// compares the scoped value as `(int)(value / 100000)`. CValueTrigger's
/// reader (`fixed`, 14138D630) takes every token as a scoped variable
/// (140539370) and compares the fixed5 value (14138D120).
pub fn required(bytes: &[u8], quoted: bool, fixed: bool) -> Result<i64, String> {
    match int_operand(bytes, quoted || fixed) {
        IntOperand::Integer(value) => Ok(i64::from(value) * ONE),
        IntOperand::Constant(value) if fixed => Ok(value),
        IntOperand::Constant(value) => Ok(i64::from((value / ONE) as i32) * ONE),
        IntOperand::Unreadable => {
            Err("the operand is a number `%i` (1424B02C0) cannot scan".into())
        }
        IntOperand::Empty => Err(
            "the operand is empty: the scoped-variable reader (140539B50) logs it and sets nothing"
                .into(),
        ),
        IntOperand::Scoped => Err(
            "the operand names a variable or scope the game evaluates at run time (140539370)"
                .into(),
        ),
    }
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

/// A CMTTHModifier's `factor` or `add` as it reads each occurrence in order
/// (140545820 -> CScopedVariable::vf3 140539440 -> 140539B50, the
/// scoped-variable reader CValueTrigger's operands share): a constant
/// replaces the value, empty text is logged and sets nothing, and any other
/// text is a variable or scope CMeanTimeToHappen::Evaluate (1405455A0)
/// evaluates at run time (CScopedVariable::Evaluate 1405382B0), refused by
/// name. `default` is the modifier's own.
pub fn mtth_operand(what: &str, items: &[Item], key: &str, default: i64) -> Result<i64, String> {
    let mut value = Ok(default);
    for item in items.iter().filter(|item| item.keyword_is(key)) {
        let text = item.value.scalar().unwrap_or_default();
        if text.is_empty() {
            continue;
        }
        value = required(text, true, true).map_err(|why| {
            format!(
                "{what} modifier {key} = {}: {why}",
                String::from_utf8_lossy(text)
            )
        });
    }
    value
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
