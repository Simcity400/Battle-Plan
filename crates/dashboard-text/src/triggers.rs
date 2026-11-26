
use std::collections::BTreeSet;

use content_reader_parser::parser::Item;
use replay_data::Hour;

mod aliases;
mod carriers;
mod coded;
mod collections;
pub mod description;
mod eval;
mod flags;
pub mod event_scopes;
pub mod inputs;
pub use event_scopes::EventGraphGuard;
pub(crate) use event_scopes::event_tooltip_target;
mod load;
mod map_scripts;
mod operands;
mod organisations;
pub use organisations::{OrganisationCondition, Undecided};
pub mod refusal;
pub use refusal::Failure;
pub mod saved_event_targets;
mod scientists;
pub mod scripted_localisation;
#[cfg(test)]
mod tests;
mod variables;
pub mod world;

pub use dashboard_query::surrender::SurrenderLimits;
pub use eval::{Combat, Evaluator, VariableSession};
pub use operands::{Selected, Selector, compare, scope_selector, shown, trigger_boolean};
pub use variables::{Store, Temps};
pub(crate) use world::{AuthoredStates, GeographyInputs, PreparedContent};
pub use world::{
    AcceptedProjects, ContentFacts, LeaderRoles, NULL_STATE, PartyLeader, PartyLeaders, ProjectReads, ProjectScope, RecordedTemplate, World,
    required_invasion_hours,
};
pub use world::{LeaderVisibility, core_gap, fixed_round};

pub const REFUSED: &str = "refused token: ";

pub const RECORDED_TABLES: &[replay_data::rows::TableId] = &[
    replay_data::rows::TableId::ActiveIdeas,
    replay_data::rows::TableId::AdvisorTokens,
    replay_data::rows::TableId::ArmyOrderPlans,
    replay_data::rows::TableId::ArmyOrders,
    replay_data::rows::TableId::Autonomy,
    replay_data::rows::TableId::Capitulation,
    replay_data::rows::TableId::CharacterOperatives,
    replay_data::rows::TableId::CharacterRoleTraits,
    replay_data::rows::TableId::Characters,
    replay_data::rows::TableId::Clock,
    replay_data::rows::TableId::CompletedFocuses,
    replay_data::rows::TableId::Countries,
    replay_data::rows::TableId::CountryAppearance,
    replay_data::rows::TableId::CountryBuildingResources,
    replay_data::rows::TableId::CountryCharacters,
    replay_data::rows::TableId::CountryCivilWars,
    replay_data::rows::TableId::CountryCommandPower,
    replay_data::rows::TableId::CountryCounterInputs,
    replay_data::rows::TableId::CountryFactories,
    replay_data::rows::TableId::CountryGameScoreInputs,
    replay_data::rows::TableId::CountryLeaderRoles,
    replay_data::rows::TableId::CountryLinks,
    replay_data::rows::TableId::CountryResources,
    replay_data::rows::TableId::CountryStabilityInputs,
    replay_data::rows::TableId::CountryRetiredCharacters,
    replay_data::rows::TableId::CountryWars,
    replay_data::rows::TableId::CountryWargoals,
    replay_data::rows::TableId::WargoalTargets,
    replay_data::rows::TableId::Divisions,
    replay_data::rows::TableId::DivisionStatus,
    replay_data::rows::TableId::DivisionTemplateIcons,
    replay_data::rows::TableId::DivisionTemplates,
    replay_data::rows::TableId::DynamicModifiers,
    replay_data::rows::TableId::FactionAdvisors,
    replay_data::rows::TableId::FactionMembers,
    replay_data::rows::TableId::Factions,
    replay_data::rows::TableId::Focus,
    replay_data::rows::TableId::GlobalSavedEventTargets,
    replay_data::rows::TableId::AdvisorRoles,
    replay_data::rows::TableId::HiredAdvisors,
    replay_data::rows::TableId::HumanControllers,
    replay_data::rows::TableId::IndustrialOrganisationDetails,
    replay_data::rows::TableId::IndustrialOrganisations,
    replay_data::rows::TableId::IntelligenceAgencies,
    replay_data::rows::TableId::InvasionPreparations,
    replay_data::rows::TableId::LandTheatres,
    replay_data::rows::TableId::LexicalTokens,
    replay_data::rows::TableId::LocalPlayer,
    replay_data::rows::TableId::ModifierDefinitions,
    replay_data::rows::TableId::Nukes,
    replay_data::rows::TableId::OrdersGroupPlans,
    replay_data::rows::TableId::OrdersGroups,
    replay_data::rows::TableId::PartyLeaders,
    replay_data::rows::TableId::PoliticalParties,
    replay_data::rows::TableId::ScientistSkills,
    replay_data::rows::TableId::Scientists,
    replay_data::rows::TableId::Politics,
    replay_data::rows::TableId::Players,
    replay_data::rows::TableId::PowerBalanceMembers,
    replay_data::rows::TableId::PowerBalanceModifierBreakdowns,
    replay_data::rows::TableId::PowerBalances,
    replay_data::rows::TableId::Projects,
    replay_data::rows::TableId::ProvinceBuildings,
    replay_data::rows::TableId::ProvinceNames,
    replay_data::rows::TableId::Provinces,
    replay_data::rows::TableId::RaidTooltipOperands,
    replay_data::rows::TableId::RaidCountryInputs,
    replay_data::rows::TableId::Raids,
    replay_data::rows::TableId::RelationOpinions,
    replay_data::rows::TableId::ResearchTechnologies,
    replay_data::rows::TableId::ResearchModifiers,
    replay_data::rows::TableId::ReservedCountryNaming,
    replay_data::rows::TableId::ResourceDefinitions,
    replay_data::rows::TableId::ScriptFlags,
    replay_data::rows::TableId::ScriptVariables,
    replay_data::rows::TableId::ScriptVariableNames,
    replay_data::rows::TableId::ScriptVariableValues,
    replay_data::rows::TableId::SimulationFeatures,
    replay_data::rows::TableId::SliceReads,
    replay_data::rows::TableId::StateBaseResources,
    replay_data::rows::TableId::StateBuildings,
    replay_data::rows::TableId::StateCores,
    replay_data::rows::TableId::StateDynamicModifiers,
    replay_data::rows::TableId::StateResourceInputs,
    replay_data::rows::TableId::StateResources,
    replay_data::rows::TableId::StateResistance,
    replay_data::rows::TableId::States,
    replay_data::rows::TableId::Subjects,
    replay_data::rows::TableId::TechnologyDefinitions,
    replay_data::rows::TableId::TechSharingGroups,
    replay_data::rows::TableId::TechSharingMembers,
    replay_data::rows::TableId::UnitLeaderTraits,
    replay_data::rows::TableId::UnitLeaders,
    replay_data::rows::TableId::WarRelations,
];

pub fn refused(reason: String) -> String {
    format!("{REFUSED}{reason}")
}

pub fn ongoing(start_hour: i32, end_hour: i32, hour: Hour) -> bool {
    end_hour <= start_hour || hour < end_hour
}

pub fn fold_token(key: &str) -> String {
    key.to_ascii_lowercase()
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub enum Eval {
    True,
    False,
    Unknown(BTreeSet<Failure>),
}

impl Eval {
    pub fn unknown(reason: impl Into<Failure>) -> Self {
        let reason = reason.into();
        reason.consumed();
        Self::Unknown(BTreeSet::from([reason]))
    }

    pub fn from_bool(value: bool) -> Self {
        if value { Self::True } else { Self::False }
    }

    pub fn and(self, other: impl FnOnce() -> Self) -> Self {
        if let Self::Unknown(reasons) = &self {
            for failure in reasons {
                failure.consumed();
            }
        }
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

    pub fn or(self, other: impl FnOnce() -> Self) -> Self {
        if let Self::Unknown(reasons) = &self {
            for failure in reasons {
                failure.consumed();
            }
        }
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
        if let Self::Unknown(reasons) = &self {
            for failure in reasons {
                failure.consumed();
            }
        }
        match self {
            Self::True => Self::False,
            Self::False => Self::True,
            unknown => unknown,
        }
    }

    pub fn decided(self) -> Result<bool, String> {
        self.decided_input().map_err(|why| why.to_string())
    }

    pub(crate) fn decided_input(self) -> Result<bool, Failure> {
        if let Self::Unknown(reasons) = &self {
            for failure in reasons {
                failure.consumed();
            }
        }
        match self {
            Self::True => Ok(true),
            Self::False => Ok(false),
            Self::Unknown(reasons) => Err(Failure::merge(reasons)),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub enum Scope {
    Country(u32),
    State(u32),
    Province(u32),
    Leader(u64),
    CountryLeader {
        country: u32,
        leader: u64,
    },
    Character(u32),
    IndustrialOrganisation(u64),
    Project {
        owner: u32,
        id: u32,
    },
    Raid(u64),
    Unit(u64),
    Combat,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scopes {
    pub root: Scope,
    pub this: Scope,
    pub prev: Option<Scope>,
    pub from: Option<Scope>,
    pub target_state: Option<u32>,
    pub target_province: Option<u32>,
    pub target_leader: Option<u64>,
    pub unknown: UnknownLinks,
    pub bare: bool,
    pub cursor: Option<usize>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UnknownLinks {
    pub root: bool,
    pub from: bool,
    pub prev: bool,
}

impl Scopes {
    pub fn empty() -> Self {
        let empty = Scope::State(NULL_STATE);
        Self {
            prev: Some(empty),
            from: Some(empty),
            ..Self::of(empty)
        }
    }

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
            cursor: None,
        }
    }

    pub fn combat() -> Self {
        Self {
            prev: Some(Scope::Combat),
            from: Some(Scope::Combat),
            ..Self::of(Scope::Combat)
        }
    }

    pub fn with_this(self, this: Scope) -> Self {
        Self {
            prev: Some(self.this),
            this,
            unknown: UnknownLinks {
                prev: false,
                ..self.unknown
            },
            bare: true,
            cursor: None,
            ..self
        }
    }

    pub fn entered(self, target: &content_reader_script::scope::Scope, this: Scope) -> Self {
        let mut scopes = self.with_this(this);
        if relinks(target) && self.root != Scope::Combat {
            scopes.unknown.from = true;
        }
        scopes.bare = match carried(target) {
            Carried::Kept => self.bare,
            Carried::Retained => false,
            Carried::Cleared => {
                !matches!(target, content_reader_script::scope::Scope::Character(_)) || this != self.this
            }
        };
        scopes
    }

    pub fn link(&self, keyword: content_reader_script::scope::Keyword) -> Result<Option<Scope>, Failure> {
        use content_reader_script::scope::Keyword;
        let (scope, unknown) = match keyword {
            Keyword::Root => (Some(self.root), self.unknown.root),
            Keyword::From => (self.from, self.unknown.from),
            _ => (self.prev, self.unknown.prev),
        };
        if unknown {
            let name = keyword.name().to_ascii_uppercase();
            return Err(format!("{name}: this event scope's {name} is not known here").into());
        }
        Ok(scope)
    }
}

enum Carried {
    Kept,
    Retained,
    Cleared,
}

fn carried(target: &content_reader_script::scope::Scope) -> Carried {
    use content_reader_script::scope::{Keyword, Scope as Parsed};
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
            None | Some(Carried::Kept) => Carried::Cleared,
            Some(other) => other,
        },
        _ => Carried::Cleared,
    }
}

fn relinks(target: &content_reader_script::scope::Scope) -> bool {
    use content_reader_script::scope::{Keyword, Scope as Parsed};
    let link = |keyword: &Keyword| matches!(keyword, Keyword::Root | Keyword::From | Keyword::Prev);
    match target {
        Parsed::Keyword(keyword) => link(keyword),
        Parsed::Chain { head, then } => link(head) || relinks(then),
        _ => false,
    }
}

impl World<'_> {
    pub fn has_children(&self, triggers: &[Item]) -> Result<bool, String> {
        Evaluator::new(self, None)
            .has_children(triggers)
            .map_err(|why| why.to_string())
    }

    pub fn evaluate(&self, triggers: &[Item], scopes: Scopes) -> Eval {
        Evaluator::new(self, None).evaluate(triggers, scopes)
    }

    pub fn country_of(&self, item: &Item, scopes: Scopes) -> Result<Option<u32>, String> {
        Evaluator::new(self, None)
            .country_operand(item, scopes)
            .map_err(|why| why.to_string())
    }

    pub fn resolve_tag(&self, tag: &str, scopes: Scopes) -> Result<Option<u32>, String> {
        Evaluator::new(self, None)
            .resolve_tag(tag, scopes)
            .map_err(|why| why.to_string())
    }
}
