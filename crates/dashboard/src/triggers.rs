//! The one trigger evaluator: script triggers decided from one recorded
//! hour, for every consumer (raids, the combat tactics, scripted
//! localisation, adjacency rules).
//!
//! A trigger body is read as its content's reader reads it
//! ([`installed_content::triggers`]); each child is dispatched as CTrigger::LoadKey
//! (1405434F0) dispatches it ([`installed_content::trigger_keys::TriggerNames::dispatch`]):
//! a coded trigger by its static keyword, a scope of the key's text
//! (14146DB00, resolved over the hour by [`crate::scope::RecordedScopes`]), a
//! content count trigger, a scripted trigger, or no child. A coded trigger is
//! checked as CTrigger::Check (140540570) checks it: its class's accepted
//! scopes first ([`installed_content::trigger_keys::trigger_class`], 140542CA0),
//! false in a scope it does not accept.
//!
//! Each input is read from the table the game itself reads, joined through
//! `CountryLinks`. A trigger with no recorded input, or one this module does
//! not reproduce, is [`Eval::Unknown`] with its reason, so a caller withholds
//! what it decides instead of guessing. `NOT` is a NOR, as in the game.

use std::collections::BTreeSet;

use installed_content::parser::Item;
use replay::Hour;

mod aliases;
mod coded;
mod eval;
mod load;
mod operands;
#[cfg(test)]
mod tests;
mod world;

pub use eval::{Combat, Evaluator};
pub use operands::{Selected, Selector, compare, required, scope_selector, shown, trigger_boolean};
pub use world::{
    ContentFacts, LeaderRoles, NULL_STATE, PartyLeader, PartyLeaders, RecordedTemplate, World,
    required_invasion_hours,
};

/// Marks an undecided reason as a refused token, which the viewer's raid
/// report counts apart.
pub const REFUSED: &str = "refused token: ";

/// `reason` marked as a refused token.
pub fn refused(reason: String) -> String {
    format!("{REFUSED}{reason}")
}

/// Whether a war row is still running at `hour`.
///
/// The engine writes a relation's `end_date` only when its raw start hour is
/// below its raw end hour (`crates/engine/src/crelation.rs:46`, serializer
/// `crates/recorder/src/oracle.rs::compare_war_relations` verifies against the
/// save. A row with `end_hour <= start_hour` therefore has no end at all and is
/// ongoing whatever the hour; a row that does carry an end is ongoing only
/// before it. The recorded "no end" value is the engine's null date, hour
/// 43_808_760, not `i32::MAX`, so no magic number appears here.
pub fn ongoing(start_hour: i32, end_hour: i32, hour: Hour) -> bool {
    end_hour <= start_hour || hour < end_hour
}

/// A key as the lexer identifies it: only ASCII A-Z fold (the token hash
/// 1424A5EE0 and `_strnicmp` 142549958).
pub fn fold_token(key: &str) -> String {
    key.to_ascii_lowercase()
}

/// A three-valued result; `Unknown` names what could not be decided.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Eval {
    True,
    False,
    Unknown(BTreeSet<String>),
}

impl Eval {
    pub fn unknown(reason: impl Into<String>) -> Self {
        Self::Unknown(BTreeSet::from([reason.into()]))
    }

    pub fn from_bool(value: bool) -> Self {
        if value { Self::True } else { Self::False }
    }

    /// A proved false wins over an unknown.
    pub fn and(self, other: impl FnOnce() -> Self) -> Self {
        match self {
            Self::False => Self::False,
            Self::True => other(),
            Self::Unknown(mut reasons) => match other() {
                Self::False => Self::False,
                Self::True => Self::Unknown(reasons),
                Self::Unknown(more) => {
                    reasons.extend(more);
                    Self::Unknown(reasons)
                }
            },
        }
    }

    /// A proved true wins over an unknown.
    pub fn or(self, other: impl FnOnce() -> Self) -> Self {
        match self {
            Self::True => Self::True,
            Self::False => other(),
            Self::Unknown(mut reasons) => match other() {
                Self::True => Self::True,
                Self::False => Self::Unknown(reasons),
                Self::Unknown(more) => {
                    reasons.extend(more);
                    Self::Unknown(reasons)
                }
            },
        }
    }

    pub fn negate(self) -> Self {
        match self {
            Self::True => Self::False,
            Self::False => Self::True,
            unknown => unknown,
        }
    }

    /// The value, or every reason it is undecided, joined.
    pub fn decided(self) -> Result<bool, String> {
        match self {
            Self::True => Ok(true),
            Self::False => Ok(false),
            Self::Unknown(reasons) => Err(reasons.into_iter().collect::<Vec<_>>().join("; ")),
        }
    }
}

/// What a scope holds (CEventScope's THIS, ROOT, PREV or FROM).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Scope {
    /// A canonical country slot.
    Country(u32),
    State(u32),
    Province(u32),
    /// A full unit leader reference, whose character the scope carries.
    Leader(u64),
    /// A character by its recorded id.
    Character(u32),
    /// A combat side's scope: its first primary country with the combat
    /// ([`Combat`]).
    Combat,
}

/// The scopes a trigger is evaluated in. A raid's
/// (`InitializeRaidEventScope@141580600`) has the raiding country as ROOT,
/// the target country as FROM and the target's province, state and leader
/// as variables.
#[derive(Clone, Copy, Debug)]
pub struct Scopes {
    pub root: Scope,
    pub this: Scope,
    pub prev: Option<Scope>,
    pub from: Option<Scope>,
    pub target_state: Option<u32>,
    pub target_province: Option<u32>,
    pub target_leader: Option<u64>,
    /// Links whose scope the caller cannot describe (scripted text's
    /// unknown links): a condition that reads one is unknown, one that does
    /// not is evaluated.
    pub unknown: UnknownLinks,
    /// THIS was set with clearing (14052F3F0), so it carries no character
    /// (+0x50) from the scope it was entered from.
    pub bare: bool,
}

/// Which of ROOT, FROM and PREV a caller cannot describe.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UnknownLinks {
    pub root: bool,
    pub from: bool,
    pub prev: bool,
}

impl Scopes {
    /// A bare scope with itself as ROOT and nothing else.
    pub fn of(scope: Scope) -> Self {
        Self {
            root: scope,
            this: scope,
            prev: None,
            from: None,
            target_state: None,
            target_province: None,
            target_leader: None,
            unknown: UnknownLinks::default(),
            bare: false,
        }
    }

    /// The combat scope a tactic's triggers are checked in: its constructor
    /// (1405286D0) makes THIS, ROOT, FROM and PREV the side.
    pub fn combat() -> Self {
        Self {
            prev: Some(Scope::Combat),
            from: Some(Scope::Combat),
            ..Self::of(Scope::Combat)
        }
    }

    /// A new THIS set with clearing, with PREV the calling scope and ROOT
    /// and FROM carried: a collection trigger's child scope (a copy of the
    /// caller whose item its setter installs with clearing: 140472450's
    /// SetState, 14043B650's set_country, each with 1 -> 14052F3F0).
    pub fn with_this(self, this: Scope) -> Self {
        Self {
            prev: Some(self.this),
            this,
            unknown: UnknownLinks {
                prev: false,
                ..self.unknown
            },
            bare: true,
            ..self
        }
    }

    /// The scopes of a target entered from these
    /// (event_target_functions::resolve 14146BB30): ROOT kept, PREV this
    /// scope, and FROM this scope's, except that ROOT, FROM and PREV copy
    /// their own scope's members and links (copy_members 1405296C0), so a
    /// target through one has that scope's FROM, which only the combat
    /// scope (whose links are itself, 1405286D0) holds here.
    ///
    /// Whether THIS carries what it was entered from follows the path:
    /// every setter 14146BB30 calls clears (set_country, SetState,
    /// SetCharacter and the others, each with 1 -> 14052F3F0), while THIS
    /// copies the calling scope (copy_members of +0x18/+0x20/+0x28 or of the
    /// scope itself), and a character target without a scope country leaves
    /// the copy_construct of the calling scope.
    pub fn entered(self, target: &installed_content::scope::Scope, this: Scope) -> Self {
        let mut scopes = self.with_this(this);
        if relinks(target) && self.root != Scope::Combat {
            scopes.unknown.from = true;
        }
        scopes.bare = match carried(target) {
            Carried::Kept => self.bare,
            Carried::Retained => false,
            Carried::Cleared => {
                !matches!(target, installed_content::scope::Scope::Character(_)) || this != self.this
            }
        };
        scopes
    }

    /// ROOT, FROM or PREV, or why the caller cannot say which scope it is.
    pub fn link(&self, keyword: installed_content::scope::Keyword) -> Result<Option<Scope>, String> {
        use installed_content::scope::Keyword;
        let (scope, unknown) = match keyword {
            Keyword::Root => (Some(self.root), self.unknown.root),
            Keyword::From => (self.from, self.unknown.from),
            _ => (self.prev, self.unknown.prev),
        };
        if unknown {
            let name = keyword.name().to_ascii_uppercase();
            return Err(format!(
                "{name}: this event scope's {name} is not known here"
            ));
        }
        Ok(scope)
    }
}

/// What a target's last step leaves THIS carrying (14146BB30).
enum Carried {
    /// THIS: a copy of the calling scope, as it carries.
    Kept,
    /// ROOT, FROM, PREV or a saved event target: a copy of that scope's
    /// members, which the caller cannot describe.
    Retained,
    /// A setter with clearing.
    Cleared,
}

fn carried(target: &installed_content::scope::Scope) -> Carried {
    use installed_content::scope::{Keyword, Scope as Parsed};
    let keyword = |keyword: &Keyword| match keyword {
        Keyword::This => Carried::Kept,
        Keyword::Root | Keyword::From | Keyword::Prev => Carried::Retained,
        _ => Carried::Cleared,
    };
    match target {
        Parsed::Keyword(head) => keyword(head),
        Parsed::Chain { head, then } => match carried(then) {
            Carried::Kept => keyword(head),
            other => other,
        },
        Parsed::EventTarget { then, .. } => match then.as_deref().map(carried) {
            None | Some(Carried::Kept) => Carried::Retained,
            Some(other) => other,
        },
        _ => Carried::Cleared,
    }
}

/// Whether a target passes through ROOT, FROM or PREV, whose scope's links
/// it takes (14146BB30).
fn relinks(target: &installed_content::scope::Scope) -> bool {
    use installed_content::scope::{Keyword, Scope as Parsed};
    let link = |keyword: &Keyword| matches!(keyword, Keyword::Root | Keyword::From | Keyword::Prev);
    match target {
        Parsed::Keyword(keyword) => link(keyword),
        Parsed::Chain { head, then } => link(head) || relinks(then),
        _ => false,
    }
}

impl World<'_> {
    /// Whether a trigger body holds a child CTrigger::LoadKey (1405434F0)
    /// adds ([`Evaluator::has_children`]).
    pub fn has_children(&self, triggers: &[Item]) -> Result<bool, String> {
        Evaluator::new(self, None).has_children(triggers)
    }

    /// A trigger body in `scopes`, with no combat.
    pub fn evaluate(&self, triggers: &[Item], scopes: Scopes) -> Eval {
        Evaluator::new(self, None).evaluate(triggers, scopes)
    }

    /// A country operand as the country triggers read it
    /// ([`Evaluator::country_operand`]).
    pub fn country_of(&self, item: &Item, scopes: Scopes) -> Result<Option<u32>, String> {
        Evaluator::new(self, None).country_operand(item, scopes)
    }

    /// A country tag or tag alias as ResolveScriptCountryTag (140BA5D10)
    /// resolves it in `scopes`, an alias through AliasResolve (1407141F0);
    /// `None` is no country.
    pub fn resolve_tag(&self, tag: &str, scopes: Scopes) -> Result<Option<u32>, String> {
        Evaluator::new(self, None).resolve_tag(tag, scopes)
    }

    /// Whether a unit leader is visible to its own country
    /// ([`Evaluator::leader_visible_from`]).
    pub fn leader_visible_from(
        &self,
        row: &replay::rows::UnitLeaderRow,
        template: Result<Option<RecordedTemplate>, String>,
    ) -> Eval {
        Evaluator::new(self, None).leader_visible_from(row, template)
    }
}
