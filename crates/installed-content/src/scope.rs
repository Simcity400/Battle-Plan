//! Script scope targets as the game parses them.
//!
//! A trigger or effect key that names a scope (`GER = { ... }`,
//! `ROOT.owner = { ... }`, `var:x = { ... }`, a state id) is read by the event
//! target constructor 14146B860 (called from CEffect::ReadEffect 1405345D0,
//! CTrigger::LoadKey 1405434F0 and the context trigger and effect), which
//! parses the key token with 14146DB00 (eventtarget.cpp). [`parse`] follows it
//! step by step; the order matters, since each step returns on a match:
//!
//! 1. ParseTokenInt32 (1424B0080) of the text is nonzero: a state id, the
//!    decimal prefix only (`5.owner` is state 5).
//! 2. The text starts with `var:` (1427BEBB4): a scoped variable
//!    (CScopedVariable::AssignTextWithDefaultScope 140539AF0).
//! 3. Three bytes long: a country specifier (140BA4F60 -> 140BA53F0: the
//!    country database's tag, FindCountryIndexByTag 14070F050, or the tag
//!    alias database 143316AA8), kept when it names one (140BA6220).
//!    Otherwise, a name the character database (143316A88, 1406B0030) holds.
//! 4. The token is one of the scope keywords 14146DA20 accepts (`root`,
//!    `this`, `prev`, `from`, `owner`, `controller`, `capital_scope`,
//!    `overlord`, `occupied`, `character`, `faction_leader`, `operation`,
//!    `opponent`) or a number token (id 0xC) whose value is zero.
//! 5. The text starts with `event_target:` (1427BE908, 13 bytes, compared
//!    exactly): the named event target, and after the first `.` a further
//!    target parsed the same way (the rest's number token is retyped 0xF).
//! 6. `prefix:value` whose prefix is a lexer keyword (14146E4E0 through
//!    1424A92B0): `mio:`, `sp:` and any other keyword prefix are kept;
//!    `scope:` only with `faction`.
//! 7. `head.rest`, where `head` is a scope keyword of step 4 (looked up
//!    without case, 14022EAB0): `head`, then `rest` parsed the same way.
//!
//! Anything else is kept as [`Scope::Unknown`]: the constructor leaves the
//! target invalid (0x4B8A) and names nothing.
//!
//! Keywords fold as the lexer folds them (1424A7A90); a quoted script token
//! is id 0xF and is never a keyword (step 4), but its text still passes
//! through steps 1-3 and 5-7, which read the text.

use crate::fixed5::decimal_i32;

/// The scope keywords 14146DA20 accepts, by their lexer ids.
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
    const ALL: [(&'static str, Self); 13] = [
        ("root", Self::Root),
        ("this", Self::This),
        ("prev", Self::Prev),
        ("from", Self::From),
        ("owner", Self::Owner),
        ("controller", Self::Controller),
        ("capital_scope", Self::CapitalScope),
        ("overlord", Self::Overlord),
        ("occupied", Self::Occupied),
        ("character", Self::Character),
        ("faction_leader", Self::FactionLeader),
        ("operation", Self::Operation),
        ("opponent", Self::Opponent),
    ];

    /// The keyword a text names, compared without case as the lexer's
    /// lookup does (1424A7A90, 14022EAB0).
    pub fn named(text: &[u8]) -> Option<Self> {
        Self::ALL
            .iter()
            .find(|(name, _)| text.eq_ignore_ascii_case(name.as_bytes()))
            .map(|(_, keyword)| *keyword)
    }

    /// The keyword's lexer name.
    pub fn name(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(_, keyword)| *keyword == self)
            .map(|(name, _)| *name)
            .unwrap_or_default()
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
    /// Whether a three-byte text names a country tag or tag alias
    /// (140BA53F0 then 140BA6220).
    fn is_country(&self, tag: &str) -> bool;
    /// Whether a name is a character's token (1406B0030).
    fn is_character(&self, token: &str) -> bool;
    /// Whether a text is a lexer keyword, which a `prefix:` must be
    /// (1424A92B0). The registry's names are content-independent; a caller
    /// without them answers for the prefixes it knows.
    fn is_keyword(&self, text: &str) -> bool;
}

const VARIABLE: &str = "var:";
const EVENT_TARGET: &str = "event_target:";

/// Parses one scope token as 14146DB00 does. `quoted` is the token's quoted
/// flag (a quoted token is never a keyword, step 4).
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
            // The rest's number token is retyped 0xF (1424B0110).
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

/// A scope a CEventScope may carry, as CTrigger::scope_compatible
/// (140542CA0) tests it, with the bit of a trigger class's accepted-scope
/// mask (the class's vtable +0x70) that asks for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ScopeKind {
    /// 0x2: a state, the scope's state id (+0xA8) >= 1.
    State,
    /// 0x4: a country, the scope's tag (+0x8) >= 1.
    Country,
    /// 0x8: a character (character_resolver 140529310).
    Character,
    /// 0x10: a combat, the combat pointer (+0x60) set.
    Combat,
    /// 0x20: the reference at +0x68/+0x6C (scope_mask_20_resolver 140529E30,
    /// class not established).
    Reference,
    /// 0x40: a strategic region, the pointer at +0x48 set.
    StrategicRegion,
    /// 0x80: an operation (1405294A0).
    Operation,
    /// 0x100: a unit (1405294C0).
    Unit,
    /// 0x200: an industrial organisation (140529390).
    IndustrialOrganisation,
    /// 0x400: a purchase contract (140529420).
    PurchaseContract,
    /// 0x800: a raid instance (140529460).
    RaidInstance,
    /// 0x1000: a special project (140529480).
    Project,
    /// 0x2000: a faction (140529350).
    Faction,
}

impl ScopeKind {
    /// Every kind, in the order 140542CA0 tests them.
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

/// A trigger class's accepted scopes: the mask its vtable +0x70 returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TriggerScopes(pub u32);

impl TriggerScopes {
    /// CTrigger::scope_compatible (140542CA0): a zero mask accepts any
    /// scope; otherwise any one accepted kind the scope carries suffices.
    /// Bits the function does not test (0x1 and above 0x2000) accept
    /// nothing. `carries` says whether the evaluated scope holds a kind; a
    /// caller that cannot tell refuses before asking.
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

    /// 140542CA0: mask zero accepts anything; any accepted kind carried
    /// suffices; an untested bit accepts nothing.
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

    /// 14146DB00's steps in order, each returning on a match.
    #[test]
    fn scope_tokens_parse_as_the_event_target_reader_does() {
        assert_eq!(scope("123"), Scope::State(123));
        // The decimal prefix wins before anything else (ParseTokenInt32).
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
