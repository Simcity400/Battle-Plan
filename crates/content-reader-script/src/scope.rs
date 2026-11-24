
use content_reader_parser::fixed5::decimal_i32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[derive(replay_spool::Spooled)]
pub enum Keyword {
    Root,
    This,
    Prev,
    From,
    Owner,
    Controller,
    CapitalScope,
    Overlord,
    Occupied,
    Character,
    FactionLeader,
    Operation,
    Opponent,
}

impl Keyword {
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

#[derive(Clone, Debug, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub enum Scope {
    State(i32),
    Variable(String),
    Country(String),
    Character(String),
    Keyword(Keyword),
    ZeroNumber(String),
    EventTarget {
        name: String,
        then: Option<Box<Scope>>,
    },
    Prefixed { prefix: String, value: String },
    Chain { head: Keyword, then: Box<Scope> },
    Unknown(String),
}

pub trait Names {
    fn is_country(&self, tag: &str) -> bool;
    fn is_character(&self, token: &str) -> bool;
    fn is_keyword(&self, text: &str) -> bool;
}

const VARIABLE: &str = "var:";
const EVENT_TARGET: &str = "event_target:";

pub fn parse(text: &str, quoted: bool, names: &impl Names) -> Scope {
    let number = content_reader_parser::parser::number_token(text.as_bytes(), quoted);
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ScopeKind {
    State,
    Country,
    Character,
    Combat,
    Reference,
    StrategicRegion,
    Operation,
    Unit,
    IndustrialOrganisation,
    PurchaseContract,
    RaidInstance,
    Project,
    Faction,
}

impl ScopeKind {
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

    pub const fn bit(self) -> u32 {
        1 << (self as u32 + 1)
    }

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TriggerScopes(pub u32);

impl TriggerScopes {
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

    #[test]
    fn scope_tokens_parse_as_the_event_target_reader_does() {
        assert_eq!(scope("123"), Scope::State(123));
        assert_eq!(scope("5.owner"), Scope::State(5));
        assert_eq!(scope("-4"), Scope::State(-4));
        assert_eq!(scope("0"), Scope::ZeroNumber("0".into()));
        assert_eq!(scope("var:my_target"), Scope::Variable("my_target".into()));
        assert_eq!(scope("GER"), Scope::Country("GER".into()));
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
        assert_eq!(parse("ROOT", true, &Content), Scope::Unknown("ROOT".into()));
        assert_eq!(
            scope("event_target:hq.owner"),
            Scope::EventTarget {
                name: "hq".into(),
                then: Some(Box::new(Scope::Keyword(Keyword::Owner)))
            }
        );
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
        assert_eq!(scope("GER.owner"), Scope::Unknown("GER.owner".into()));
    }
}
