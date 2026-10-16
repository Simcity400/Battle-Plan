//! Script scope targets use ordered recognition. A trigger or effect key can name a
//! country, a state, a chained scope or a scoped variable. [`parse`] checks these
//! steps, returning on the first match:
//!
//! 1. A nonzero decimal prefix names a state (`5.owner` is state 5).
//! 2. `var:` introduces a scoped variable.
//! 3. Three-byte text can name a country tag or alias. Other text can name a character
//!    token.
//! 4. A scope keyword (`root`, `this`, `prev`, `from`, `owner`, `controller`,
//!    `capital_scope`, `overlord`, `occupied`, `character`, `faction_leader`, `operation`,
//!    `opponent`) or a zero-valued number token is retained.
//! 5. Exact `event_target:` introduces a named target and an optional further target
//!    after the first dot, parsed without number-token classification.
//! 6. A keyword prefix in `prefix:value` is retained, including `mio:` and `sp:`;
//!    `scope:` accepts only `faction`.
//! 7. `head.rest` accepts a scope-keyword head without case and parses the rest
//!    recursively.
//!
//! Unmatched text stays [`Scope::Unknown`], naming no valid target. Quoted text cannot
//! match the keyword step but still participates in text-based steps.

use crate::fixed5::decimal_i32;

/// The accepted scope keywords.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Keyword {
    /// `root` (0x2C94).
    Root,
    /// `this` (0x29C3).
    This,
    /// `prev` (0x2C95).
    Prev,
    /// `from` (0x298F).
    From,
    /// `owner` (0x283E).
    Owner,
    /// `controller` (0x283F).
    Controller,
    /// `capital_scope` (0x29FE).
    CapitalScope,
    /// `overlord` (0x2B7F).
    Overlord,
    /// `occupied` (0x3DB1).
    Occupied,
    /// `character` (0x4C16).
    Character,
    /// `faction_leader` (0x3C40).
    FactionLeader,
    /// `operation` (0x2F1B).
    Operation,
    /// `opponent` (0x3514).
    Opponent,
}

impl Keyword {
    /// Recognises a scope selector with the lexer's ASCII case fold.
    pub fn named(text: &[u8]) -> Option<Self> {
        match text.to_ascii_lowercase().as_slice() {
            b"root" => Some(Self::Root),
            b"this" => Some(Self::This),
            b"prev" => Some(Self::Prev),
            b"from" => Some(Self::From),
            b"owner" => Some(Self::Owner),
            b"controller" => Some(Self::Controller),
            b"capital_scope" => Some(Self::CapitalScope),
            b"overlord" => Some(Self::Overlord),
            b"occupied" => Some(Self::Occupied),
            b"character" => Some(Self::Character),
            b"faction_leader" => Some(Self::FactionLeader),
            b"operation" => Some(Self::Operation),
            b"opponent" => Some(Self::Opponent),
            _ => None,
        }
    }

    /// The keyword's lexer name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Root => "root",
            Self::This => "this",
            Self::Prev => "prev",
            Self::From => "from",
            Self::Owner => "owner",
            Self::Controller => "controller",
            Self::CapitalScope => "capital_scope",
            Self::Overlord => "overlord",
            Self::Occupied => "occupied",
            Self::Character => "character",
            Self::FactionLeader => "faction_leader",
            Self::Operation => "operation",
            Self::Opponent => "opponent",
        }
    }
}

/// One parsed scope target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Step 1: a nonzero decimal prefix, a state id.
    State(i32),
    /// Step 2: `var:<name>`, a scoped variable.
    Variable(String),
    /// Step 3: a three-byte country tag the content names.
    Country(String),
    /// Step 3: a name the character database holds.
    Character(String),
    /// Step 4: a scope keyword.
    Keyword(Keyword),
    /// Step 4: a number token whose value is zero (`0`, `0.5`, `-`).
    ZeroNumber(String),
    /// Step 5: `event_target:<name>`, then an optional further target.
    EventTarget {
        name: String,
        then: Option<Box<Scope>>,
    },
    /// Step 6: `<prefix>:<value>` with a keyword prefix (`mio`, `sp`,
    /// `scope:faction`, ...), the prefix folded.
    Prefixed { prefix: String, value: String },
    /// Step 7: a scope keyword, then a further target.
    Chain { head: Keyword, then: Box<Scope> },
    /// None of the steps: the target stays invalid (0x4B8A).
    Unknown(String),
}

/// Content facts steps 3 and 6 look up.
pub trait Names {
    /// Whether three-byte text names a country tag or tag alias.
    fn is_country(&self, tag: &str) -> bool;
    /// Whether a name is a character token.
    fn is_character(&self, token: &str) -> bool;
    /// Whether text is a lexer keyword, as a `prefix:` requires. Registry names are
    /// content-independent; callers without them answer for known prefixes.
    fn is_keyword(&self, text: &str) -> bool;
}

const VARIABLE: &str = "var:";
const EVENT_TARGET: &str = "event_target:";

/// Parse one scope token. `quoted` prevents keyword recognition in step 4.
pub fn parse(text: &str, quoted: bool, names: &impl Names) -> Scope {
    let number = crate::parser::number_token(text.as_bytes(), quoted);
    parse_token(text, number, quoted, names)
}

fn parse_token(text: &str, number: bool, quoted: bool, names: &impl Names) -> Scope {
    let state = decimal_i32(text.as_bytes());
    if state != 0 {
        return Scope::State(state);
    }
    if let Some(name) = text.strip_prefix(VARIABLE) {
        return Scope::Variable(name.to_owned());
    }
    if text.len() == 3 {
        if names.is_country(text) {
            return Scope::Country(text.to_owned());
        }
    } else if names.is_character(text) {
        return Scope::Character(text.to_owned());
    }
    if number {
        return Scope::ZeroNumber(text.to_owned());
    }
    if !quoted && let Some(keyword) = Keyword::named(text.as_bytes()) {
        return Scope::Keyword(keyword);
    }
    if let Some(rest) = text.strip_prefix(EVENT_TARGET) {
        let (name, then) = match rest.split_once('.') {
            Some((name, then)) => (name, Some(then)),
            None => (rest, None),
        };
        return Scope::EventTarget {
            name: name.to_owned(),
            // The rest is parsed without number-token classification.
            then: then.map(|then| Box::new(parse_token(then, false, false, names))),
        };
    }
    if let Some((prefix, value)) = text.split_once(':')
        && names.is_keyword(prefix)
    {
        let prefix = prefix.to_ascii_lowercase();
        if prefix != "scope" || value.eq_ignore_ascii_case("faction") {
            return Scope::Prefixed {
                prefix,
                value: value.to_owned(),
            };
        }
        return Scope::Unknown(text.to_owned());
    }
    if let Some((head, rest)) = text.split_once('.')
        && let Some(head) = Keyword::named(head.as_bytes())
    {
        // As after `event_target:`, the rest's number token is retyped 0xF.
        return Scope::Chain {
            head,
            then: Box::new(parse_token(rest, false, false, names)),
        };
    }
    Scope::Unknown(text.to_owned())
}

impl std::fmt::Display for Scope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::State(id) => write!(f, "{id}"),
            Self::Variable(name) => write!(f, "{VARIABLE}{name}"),
            Self::Country(text) | Self::Character(text) | Self::ZeroNumber(text) => {
                f.write_str(text)
            }
            Self::Unknown(text) => f.write_str(text),
            Self::Keyword(keyword) => f.write_str(keyword.name()),
            Self::EventTarget { name, then } => {
                write!(f, "{EVENT_TARGET}{name}")?;
                match then {
                    Some(then) => write!(f, ".{then}"),
                    None => Ok(()),
                }
            }
            Self::Prefixed { prefix, value } => write!(f, "{prefix}:{value}"),
            Self::Chain { head, then } => write!(f, "{}.{then}", head.name()),
        }
    }
}

/// An evaluated scope kind and its bit in a trigger's accepted-scope mask.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ScopeKind {
    /// 0x2: a state whose id is positive.
    State,
    /// 0x4: a country whose tag id is positive.
    Country,
    /// 0x8: a character.
    Character,
    /// 0x10: a combat reference is set.
    Combat,
    /// 0x20: an unresolved reference kind.
    Reference,
    /// 0x40: a strategic-region reference is set.
    StrategicRegion,
    /// 0x80: an operation.
    Operation,
    /// 0x100: a unit.
    Unit,
    /// 0x200: an industrial organisation.
    IndustrialOrganisation,
    /// 0x400: a purchase contract.
    PurchaseContract,
    /// 0x800: a raid instance.
    RaidInstance,
    /// 0x1000: a special project.
    Project,
    /// 0x2000: a faction.
    Faction,
}

impl ScopeKind {
    /// Every scope kind in compatibility-check order.
    pub const ALL: [Self; 13] = [
        Self::State,
        Self::Country,
        Self::Character,
        Self::Combat,
        Self::Reference,
        Self::StrategicRegion,
        Self::Operation,
        Self::Unit,
        Self::IndustrialOrganisation,
        Self::PurchaseContract,
        Self::RaidInstance,
        Self::Project,
        Self::Faction,
    ];

    /// The kind's bit in an accepted-scope mask.
    pub const fn bit(self) -> u32 {
        1 << (self as u32 + 1)
    }

    /// The mask accepting every native scope kind.
    pub const fn all_bits() -> u32 {
        let mut bits = 0;
        let mut index = 0;
        while index < Self::ALL.len() {
            bits |= Self::ALL[index].bit();
            index += 1;
        }
        bits
    }
}

/// A trigger class's accepted-scope mask.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TriggerScopes(pub u32);

impl TriggerScopes {
    /// A zero mask accepts every scope. Otherwise any accepted kind carried by the
    /// scope suffices. Untested bits (0x1 and above 0x2000) accept nothing. `carries`
    /// reports the evaluated scope's kinds; a caller that cannot determine them refuses
    /// before asking.
    pub fn accepts(self, carries: impl Fn(ScopeKind) -> bool) -> bool {
        self.0 == 0
            || ScopeKind::ALL
                .iter()
                .any(|kind| self.0 & kind.bit() != 0 && carries(*kind))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mask zero accepts anything; any accepted kind carried suffices. Untested bits
    /// accept nothing.
    #[test]
    fn trigger_scopes_accept_as_the_game_checks_them() {
        assert_eq!(ScopeKind::State.bit(), 0x2);
        assert_eq!(ScopeKind::Country.bit(), 0x4);
        assert_eq!(ScopeKind::Faction.bit(), 0x2000);
        assert_eq!(ScopeKind::all_bits(), 0x3FFE);
        let country_only = |kind| kind == ScopeKind::Country;
        assert!(TriggerScopes(0).accepts(|_| false));
        assert!(TriggerScopes(0x4).accepts(country_only));
        assert!(!TriggerScopes(0x2).accepts(country_only));
        assert!(TriggerScopes(0x2 | 0x4).accepts(country_only));
        assert!(!TriggerScopes(0x1).accepts(|_| true));
    }

    struct Content;
    impl Names for Content {
        fn is_country(&self, tag: &str) -> bool {
            matches!(tag, "GER" | "SOV")
        }
        fn is_character(&self, token: &str) -> bool {
            token == "GER_erwin_rommel"
        }
        fn is_keyword(&self, text: &str) -> bool {
            matches!(
                text.to_ascii_lowercase().as_str(),
                "mio" | "sp" | "scope" | "country"
            )
        }
    }

    fn scope(text: &str) -> Scope {
        parse(text, false, &Content)
    }

    /// Scope-recognition steps return on their first match.
    #[test]
    fn scope_tokens_parse_as_the_event_target_reader_does() {
        assert_eq!(scope("123"), Scope::State(123));
        // The decimal prefix wins before any other recognition.
        assert_eq!(scope("5.owner"), Scope::State(5));
        assert_eq!(scope("-4"), Scope::State(-4));
        assert_eq!(scope("0"), Scope::ZeroNumber("0".into()));
        assert_eq!(scope("var:my_target"), Scope::Variable("my_target".into()));
        assert_eq!(scope("GER"), Scope::Country("GER".into()));
        // Three bytes that name no country fall through to the rest.
        assert_eq!(scope("XYZ"), Scope::Unknown("XYZ".into()));
        assert_eq!(
            scope("GER_erwin_rommel"),
            Scope::Character("GER_erwin_rommel".into())
        );
        assert_eq!(scope("ROOT"), Scope::Keyword(Keyword::Root));
        assert_eq!(
            scope("Capital_Scope"),
            Scope::Keyword(Keyword::CapitalScope)
        );
        // A quoted token is id 0xF, never a keyword.
        assert_eq!(parse("ROOT", true, &Content), Scope::Unknown("ROOT".into()));
        assert_eq!(
            scope("event_target:hq.owner"),
            Scope::EventTarget {
                name: "hq".into(),
                then: Some(Box::new(Scope::Keyword(Keyword::Owner)))
            }
        );
        // `event_target:` is compared exactly (strncmp).
        assert_eq!(
            scope("EVENT_TARGET:hq"),
            Scope::Unknown("EVENT_TARGET:hq".into())
        );
        assert_eq!(
            scope("mio:GER_tank"),
            Scope::Prefixed {
                prefix: "mio".into(),
                value: "GER_tank".into()
            }
        );
        assert!(matches!(scope("scope:faction"), Scope::Prefixed { .. }));
        assert_eq!(scope("scope:other"), Scope::Unknown("scope:other".into()));
        assert_eq!(scope("nokeyword:x"), Scope::Unknown("nokeyword:x".into()));
        assert_eq!(
            scope("ROOT.owner.capital_scope"),
            Scope::Chain {
                head: Keyword::Root,
                then: Box::new(Scope::Chain {
                    head: Keyword::Owner,
                    then: Box::new(Scope::Keyword(Keyword::CapitalScope))
                })
            }
        );
        assert_eq!(
            scope("ROOT.owner.capital_scope").to_string(),
            "root.owner.capital_scope"
        );
        assert_eq!(
            scope("PREV.GER"),
            Scope::Chain {
                head: Keyword::Prev,
                then: Box::new(Scope::Country("GER".into()))
            }
        );
        // Only a keyword heads a chain: a tag does not.
        assert_eq!(scope("GER.owner"), Scope::Unknown("GER.owner".into()));
    }
}
