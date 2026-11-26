use crate::triggers::Failure;
use crate::triggers::refusal::refused;

use std::{
    borrow::Cow,
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, OnceLock},
};

use content_reader_key_names::{characters::{LeaderTemplate, ScientistVisibilityTemplate}, tag_aliases::TagAliases, unit_leader::Traits};
use content_reader_military::leader_traits::LeaderTraits;
use content_reader_politics::script_constants::Constants;
use content_reader_script::trigger_keys::TriggerNames;
use replay_data::{
    Hour, rows::CharacterRow, rows::CountryAppearanceRow, rows::CountryCounterInputRow,
    rows::CountryLinkRow, rows::CountryRow, rows::InvasionPreparationRow, rows::LexicalTokenRow,
    rows::ProvinceRow, rows::Recorded, rows::RowCodec, rows::ScriptVariablesRow, rows::StateRow,
    rows::TableSchema, rows::column, rows::mask, rows::tables,
};
use replay_frame::frame::{HourFrame, Leader, OperandStatus, Province, Rowset};
use replay_frame::{frame::DecodedFrame, frame::TableView, frame::ViewIdentity};

use super::{Eval, Scope, ongoing, variables::Store};
use crate::{
    character_scope,
    scope::RecordedScopes,
    state_resources::{Links, ONE, StateResources},
};

#[path = "world/literals.rs"]
mod literals;

type Index<T> = OnceLock<super::refusal::Resolved<Result<T, Failure>>>;

type OperandLoader<'a, T> = (
    &'a dyn PreparedContent,
    HourFrame<'a>,
    fn(&'a dyn PreparedContent, HourFrame<'a>) -> T,
);

pub struct PreparedOperand<'a, T> {
    value: OnceLock<T>,
    load: Option<OperandLoader<'a, T>>,
}

impl<'a, T> PreparedOperand<'a, T> {
    fn lazy(
        content: &'a dyn PreparedContent,
        frame: HourFrame<'a>,
        load: fn(&'a dyn PreparedContent, HourFrame<'a>) -> T,
    ) -> Self {
        Self {
            value: OnceLock::new(),
            load: Some((content, frame, load)),
        }
    }
}

impl<T> From<T> for PreparedOperand<'_, T> {
    fn from(value: T) -> Self {
        Self {
            value: OnceLock::from(value),
            load: None,
        }
    }
}

impl<T> std::ops::Deref for PreparedOperand<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.value.get_or_init(|| {
            let (content, frame, load) = self.load.expect("lazy operand loader");
            load(content, frame)
        })
    }
}

pub(crate) type AuthoredStates<'a> = (
    &'a BTreeMap<u32, Vec<u32>>,
    &'a BTreeSet<u32>,
    Result<(), &'a str>,
);
pub(crate) trait PreparedContent: Send + Sync {
    fn ideas(&self) -> Result<&content_reader_politics::ideas::IdeaContent, &str>;
    fn mio_traits(&self) -> Result<&content_reader_military::mio_traits::Catalog, &str>;
    fn equipment_groups(&self) -> Result<&content_reader_key_names::equipment::Groups, &str>;
    fn special_projects(&self) -> Result<&content_reader_military::special_projects::Catalog, &str>;
    fn authored_states(&self) -> AuthoredStates<'_>;
    fn continents(&self) -> Result<&[String], &str>;
    fn ideologies(&self) -> Result<&content_reader_key_names::ideologies::Ideologies, &str>;
    fn triggers(&self) -> Result<&TriggerNames, &str>;
    fn character_traits(&self) -> Result<(&Traits, &LeaderTraits), &str>;
    fn resources(&self) -> Result<(&content_reader_key_names::buildings::Buildings, [i64; 2]), &str>;
    fn leader_roles(&self) -> Result<&LeaderRoles, &str>;
    fn constants(&self) -> Result<&Constants, &str>;
    fn invasion_days(&self) -> Result<i32, &str>;
    fn geography(&self) -> GeographyInputs<'_>;
}

#[path = "world/folds.rs"]
mod folds;
use folds::Folds;
thread_local! {
    static FOLDS: RefCell<Vec<(ViewIdentity, Arc<Folds>)>> = const { RefCell::new(Vec::new()) };
    static VAR_NAMES: RefCell<Vec<(ViewIdentity, Arc<std::collections::HashMap<Vec<u8>, u32>>)>> =
        const { RefCell::new(Vec::new()) };
}

pub(in crate::triggers) enum Names {
    Own(Arc<Folds>),
}
type Linked<T> = (T, Option<Failure>);
#[derive(Debug)]
pub(super) struct PartialCollection<'a, T> {
    known: &'a [T],
    unresolved: Option<&'a Failure>,
}

impl<T: super::inputs::Fingerprint> super::inputs::Fingerprint for PartialCollection<'_, T> {
    fn fingerprint(&self, into: &mut std::hash::DefaultHasher) {
        self.known.fingerprint(into);
        self.unresolved.fingerprint(into);
    }
}

impl<T> PartialCollection<'_, T> {
    pub(super) fn any(&self, mut predicate: impl FnMut(&T) -> super::Eval) -> super::Eval {
        let mut result = super::Eval::False;
        for entry in self.known {
            result = result.or(|| predicate(entry));
            if result == super::Eval::True {
                return result;
            }
        }
        if let Some(reason) = self.unresolved {
            reason.reached();
            result = result.or(|| super::Eval::unknown(reason.clone()));
        }
        result
    }
}
#[derive(Debug)]
pub struct ReferencedRows<T> {
    rows: BTreeMap<u32, T>,
    table: replay_data::rows::TableId,
    unavailable: Option<Failure>,
}

impl<T> ReferencedRows<T> {
    fn enumerate(&self) {
        if let Some(failure) = &self.unavailable {
            failure.resolution_failed();
        }
    }

    pub fn iter(&self) -> std::collections::btree_map::Iter<'_, u32, T> {
        self.enumerate();
        self.rows.iter()
    }

    pub fn values(&self) -> std::collections::btree_map::Values<'_, u32, T> {
        self.enumerate();
        self.rows.values()
    }

    pub fn keys(&self) -> std::collections::btree_map::Keys<'_, u32, T> {
        self.enumerate();
        self.rows.keys()
    }

    pub fn len(&self) -> usize {
        self.enumerate();
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.enumerate();
        self.rows.is_empty()
    }

    pub fn get(&self, id: &u32) -> Option<&T> {
        super::refusal::reference(self.rows.get(id), *id != 0, || {
            self.unavailable.clone().unwrap_or_else(|| {
                super::refusal::missing_reference(
                    self.table,
                    (self.table == tables::States::ID).then_some(Store::State(*id)),
                )
            })
        })
    }

    pub fn contains_key(&self, id: &u32) -> bool {
        self.get(id).is_some()
    }
}

impl<T> std::ops::Deref for ReferencedRows<T> {
    type Target = BTreeMap<u32, T>;
    fn deref(&self) -> &Self::Target {
        &self.rows
    }
}

impl<'m, T> IntoIterator for &'m ReferencedRows<T> {
    type Item = (&'m u32, &'m T);
    type IntoIter = std::collections::btree_map::Iter<'m, u32, T>;
    fn into_iter(self) -> Self::IntoIter {
        self.enumerate();
        self.rows.iter()
    }
}

type Invasions<'a> = BTreeMap<u32, CountryInvasions<'a>>;

#[derive(Default)]
struct CountryInvasions<'a> {
    groups: Vec<u64>,
    orders: Index<Vec<Invasion<'a>>>,
}

#[derive(Clone, Copy, Debug, Hash)]
pub(super) struct Invasion<'a> {
    pub key: (u64, u32),
    pub active: u8,
    pub path: &'a [u32],
}
type CountryNames<'a> = BTreeSet<(u32, &'a [u8])>;

fn linked<T>(found: Option<T>, unresolved: &Option<Failure>) -> Result<Option<T>, Failure> {
    match (found, unresolved) {
        (Some(value), _) => Ok(Some(value)),
        (None, Some(reason)) => {
            reason.reached();
            Err(reason.clone())
        }
        (None, None) => Ok(None),
    }
}
struct CountryRows {
    runs: Vec<(u32, usize, usize)>,
    unlinked: Option<(u32, u32)>,
}

impl CountryRows {
    fn new(
        countries: impl Iterator<Item = Result<u32, Failure>>,
        linked: impl Fn(u32) -> bool,
    ) -> Result<Option<Self>, Failure> {
        let mut runs: Vec<(u32, usize, usize)> = Vec::new();
        for (at, country) in countries.enumerate() {
            let country = country?;
            match runs.last_mut() {
                Some(run) if run.0 == country => run.2 = at + 1,
                Some(run) if run.0 > country => return Ok(None),
                _ => runs.push((country, at, at + 1)),
            }
        }
        let mut unlinked = runs
            .iter()
            .map(|run| run.0)
            .filter(|&country| !linked(country));
        let first = unlinked.next();
        let unlinked = first.map(|first| (first, unlinked.last().unwrap_or(first)));
        Ok(Some(Self { runs, unlinked }))
    }

    fn of(&self, country: u32) -> std::ops::Range<usize> {
        self.runs
            .binary_search_by_key(&country, |run| run.0)
            .map_or(0..0, |at| self.runs[at].1..self.runs[at].2)
    }

    fn first_unlinked(&self) -> Option<(usize, u32)> {
        let (country, _) = self.unlinked?;
        Some((self.of(country).start, country))
    }
}

type Modifiers<'a> = BTreeMap<(u32, &'a [u8]), Vec<(u32, i32)>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct State {
    pub id: u32,
    pub owner: u32,
    pub controller: u32,
    pub continent: Recorded<Option<u32>>,
}

const LINK: u64 = mask::<CountryLinkRow>(&["canonical_slot"]);
const COUNTRY: u64 = mask::<CountryRow>(&["canonical_slot"]);
const TAG: usize = column::<CountryRow>("tag");
const PROVINCE: u64 = mask::<ProvinceRow>(&[
    "state",
    "owner",
    "controller",
    "victory_points",
    "controller_area",
]);
const STATE: u64 = mask::<StateRow>(&["owner", "controller", "continent"]);
const LITERAL: u64 = mask::<LexicalTokenRow>(&["literal"]);
const SPAN: u64 = mask::<replay_data::rows::WarRelationRow>(&["start_hour", "end_hour"]);
const FOCUS_TOKEN: usize = column::<replay_data::rows::CompletedFocusRow>("token");
const RULING: u64 = mask::<replay_data::rows::PoliticalPartyRow>(&["is_ruling"]);
const PARTY_TOKEN: usize = column::<replay_data::rows::PoliticalPartyRow>("token");
const PARTY_LEADER: u64 =
    mask::<replay_data::rows::PartyLeaderRow>(&["character", "leader_id", "role"]);
const COUNTRY_CHARACTER: u64 = mask::<replay_data::rows::CountryCharacterRow>(&["character"]);
const SCIENTIST: u64 =
    mask::<replay_data::rows::ScientistRow>(&["country", "ordinal", "skill_count", "active"]);
const SCIENTIST_SKILL: u64 = mask::<replay_data::rows::ScientistSkillRow>(&["token", "level"]);
const IDEA_TOKEN: usize = column::<replay_data::rows::ActiveIdeaRow>("token");
const LITERAL_COLUMN: usize = column::<LexicalTokenRow>("literal");

fn walked<T: TableSchema>(why: std::io::Error) -> Failure {
    super::refusal::input_error(T::ID, why)
}

fn each<'a, T: TableSchema>(
    rows: Rowset<'a, T::Row>,
) -> impl DoubleEndedIterator<Item = Result<Cow<'a, T::Row>, Failure>> + 'a {
    rows.iter().map(|row| row.map_err(walked::<T>))
}

fn states_of(inputs: &CountryCounterInputRow, controlled: bool) -> &Recorded<Option<Vec<u32>>> {
    if controlled {
        &inputs.controlled_states
    } else {
        &inputs.owned_states
    }
}

fn state_list(inputs: &CountryCounterInputRow, controlled: bool) -> &[u32] {
    match states_of(inputs, controlled) {
        Recorded::Value(Some(states)) => states.as_slice(),
        _ => &[],
    }
}

type OrganisationDetails<'a> = Cow<'a, replay_data::rows::IndustrialOrganisationDetailsRow>;

fn organisation_size(row: OrganisationDetails<'_>) -> i32 {
    row.state.size
}

fn organisation_unlocked(row: OrganisationDetails<'_>) -> Cow<'_, [u32]> {
    part(row, |row| row.unlocked.as_slice())
}

fn organisation_policies(row: OrganisationDetails<'_>) -> (Cow<'_, [u32]>, u32) {
    let policy = row.state.policy;
    (part(row, |row| row.allowed_policies.as_slice()), policy)
}

fn organisation_flags(row: OrganisationDetails<'_>) -> Cow<'_, [replay_data::rows::IndustrialOrganisationFlag]> {
    part(row, |row| row.flags.as_slice())
}

fn organisation_tasks(row: OrganisationDetails<'_>) -> Cow<'_, [replay_data::rows::IndustrialOrganisationTask]> {
    part(row, |row| row.tasks.as_slice())
}

fn part<'a, R: Clone, T: ToOwned + ?Sized>(
    row: Cow<'a, R>,
    field: impl for<'r> Fn(&'r R) -> &'r T,
) -> Cow<'a, T> {
    match row {
        Cow::Borrowed(row) => Cow::Borrowed(field(row)),
        Cow::Owned(row) => Cow::Owned(field(&row).to_owned()),
    }
}

fn only<'a, T: TableSchema>(
    rows: Rowset<'a, T::Row>,
) -> Result<Option<Cow<'a, T::Row>>, Failure> {
    if rows.len() != 1 {
        return Ok(None);
    }
    each::<T>(rows).next().transpose()
}

fn keyed<'a, T: TableSchema>(
    rows: &Rowset<'a, T::Row>,
    key: u128,
) -> Result<Option<Cow<'a, T::Row>>, Failure> {
    rows.get(key).map_err(walked::<T>)
}

struct PoliticalParties<'a> {
    ruling: BTreeMap<u32, u32>,
    unresolved: Option<Failure>,
    ids: BTreeMap<u32, &'a [u8]>,
    tokens: BTreeMap<Vec<u8>, u32>,
    members: BTreeSet<(u32, u32)>,
}

type CoreRows<'f> = (
    Rowset<'f, CountryLinkRow>,
    Rowset<'f, CountryRow>,
    Rowset<'f, ProvinceRow>,
    Rowset<'f, StateRow>,
);

fn core_reason(status: OperandStatus<'_>, table: replay_data::rows::TableId) -> Option<String> {
    let what = table.words();
    match status {
        OperandStatus::Complete | OperandStatus::WrongLayout => None,
        OperandStatus::Truncated(missing) | OperandStatus::TruncatedWrongLayout(missing) => {
            Some(format!("{what}: read only in part ({missing} rows missing)"))
        }
        OperandStatus::Unavailable(reason) => Some(format!("{what}: not available: {reason}")),
        OperandStatus::NotRecorded(mode) => {
            Some(format!("{what}: not recorded in a {mode} recording"))
        }
    }
}

fn core_tables(frame: HourFrame<'_>) -> (CoreRows<'_>, Vec<(&'static str, Failure)>) {
    fn rows<'f, T: TableSchema>(
        frame: HourFrame<'f>,
        mask: u64,
        name: &'static str,
        gaps: &mut Vec<(&'static str, Failure)>,
    ) -> Rowset<'f, T::Row> {
        if let Some(reason) = core_reason(frame.status(T::ID), T::ID) {
            gaps.push((name, Failure::input(T::ID, "", None, reason)));
        }
        frame.held::<T>(mask)
    }
    let mut gaps = Vec::new();
    let core = (
        rows::<tables::CountryLinks>(frame, LINK, "CountryLinks", &mut gaps),
        rows::<tables::Countries>(frame, COUNTRY, "Countries", &mut gaps),
        rows::<tables::Provinces>(frame, PROVINCE, "Provinces", &mut gaps),
        rows::<tables::States>(frame, STATE, "States", &mut gaps),
    );
    (core, gaps)
}

pub fn core_gap(frame: &dyn replay_frame::frame::RecordedSource) -> Option<String> {
    core_gap_of(|table| frame.status(table))
}

pub fn core_gap_in(frame: HourFrame<'_>) -> Option<String> {
    core_gap_of(|table| frame.status(table))
}

fn core_gap_of<'f>(
    status: impl Fn(replay_data::rows::TableId) -> OperandStatus<'f>,
) -> Option<String> {
    use replay_data::rows::TableId;
    let gaps = super::refusal::deferred(|| {
        [TableId::CountryLinks, TableId::Countries, TableId::Provinces, TableId::States]
        .into_iter()
        .filter_map(|table| {
            core_reason(status(table), table).map(|reason| Failure::input(table, "", None, reason))
        })
        .collect::<Vec<_>>()
    });
    gaps.into_iter().next().map(|why| {
        why.reached();
        why.to_string()
    })
}

type TemplateNames<'a> = (
    &'a content_reader_layers::content::diagnostics::Once<
        Result<content_reader_localisation::localisation::Localisation, String>,
    >,
    &'a content_reader_layers::ResolvedLayers,
    &'a str,
);

#[derive(Clone, Copy)]
pub struct GeographyInputs<'a> {
    pub neighbors:
        &'a content_reader_layers::content::diagnostics::Once<Result<BTreeMap<u32, Vec<u32>>, String>>,
    pub terrain: &'a content_reader_layers::content::diagnostics::Once<
        Option<content_reader_map::movement::MovementMap>,
    >,
    pub layers: &'a content_reader_layers::ResolvedLayers,
    pub state_provinces: &'a BTreeMap<u32, Vec<u32>>,
    pub state_error: Option<&'a str>,
    pub defines: &'a content_reader_defines::defines::Defines,
    pub terrain_definitions: Option<&'a content_reader_key_names::terrain::TerrainDefinitions>,
}

impl<'a> GeographyInputs<'a> {
    pub(crate) fn neighbors(self) -> Result<&'a BTreeMap<u32, Vec<u32>>, &'a str> {
        self.neighbors
            .get_or_init(|| match self.state_error {
                Some(why) => Err(why.to_owned()),
                None => content_reader_map::province_graph::ProvinceGraph::read(self.layers)
                    .and_then(|(graph, _)| graph.neighbor_states(self.state_provinces))
                    .map_err(|error| error.to_string()),
            })
            .as_ref()
            .map_err(String::as_str)
    }

    pub(crate) fn terrain(self) -> Option<&'a content_reader_map::movement::MovementMap> {
        self.terrain
            .get_or_init(|| {
                self.terrain_definitions.and_then(|terrain| {
                    content_reader_map::movement::MovementMap::read_with_terrain(
                        self.layers,
                        self.defines,
                        terrain,
                    )
                    .ok()
                })
            })
            .as_ref()
    }
}

pub struct World<'a> {
    frame: HourFrame<'a>,
    prepared_content: Option<&'a dyn PreparedContent>,
    hour: Hour,
    core_gaps: Vec<(&'static str, Failure)>,
    link_rows: Rowset<'a, CountryLinkRow>,
    links: OnceLock<BTreeMap<u32, u32>>,
    country_rows: Rowset<'a, CountryRow>,
    countries: Index<BTreeMap<u32, (u32, &'a [u8])>>,
    province_rows: Rowset<'a, ProvinceRow>,
    state_rows: Rowset<'a, StateRow>,
    provinces: OnceLock<ReferencedRows<Province>>,
    states: OnceLock<ReferencedRows<State>>,
    state_provinces: PreparedOperand<'a, &'a BTreeMap<u32, Vec<u32>>>,
    pub(super) state_content: PreparedOperand<'a, Result<(), &'a str>>,
    pub(super) continents: PreparedOperand<'a, Result<&'a [String], &'a str>>,
    pub(super) ideologies:
        PreparedOperand<'a, Result<&'a content_reader_key_names::ideologies::Ideologies, &'a str>>,
    pub(super) impassable: PreparedOperand<'a, &'a BTreeSet<u32>>,
    neighbor_states: Result<&'a BTreeMap<u32, Vec<u32>>, &'a str>,
    terrain: Option<&'a content_reader_map::movement::MovementMap>,
    lazy_geography: Option<GeographyInputs<'a>>,
    pub(super) dlcs: &'a BTreeSet<String>,
    pub(super) triggers: PreparedOperand<'a, Result<&'a TriggerNames, &'a str>>,
    pub(super) character_traits:
        PreparedOperand<'a, Result<(&'a Traits, &'a LeaderTraits), &'a str>>,
    pub(super) ideas: PreparedOperand<'a, Result<&'a content_reader_politics::ideas::IdeaContent, &'a str>>,
    pub(super) mio_traits:
        PreparedOperand<'a, Result<&'a content_reader_military::mio_traits::Catalog, &'a str>>,
    pub(super) equipment_groups:
        PreparedOperand<'a, Result<&'a content_reader_key_names::equipment::Groups, &'a str>>,
    pub(super) special_projects:
        PreparedOperand<'a, Result<&'a content_reader_military::special_projects::Catalog, &'a str>>,
    character_tables: OnceLock<character_scope::Tables<'a>>,
    aliases: &'a Arc<TagAliases>,
    recorded: OnceLock<RecordedScopes<'a>>,
    names: Index<Names>,
    variable_names: Index<Arc<std::collections::HashMap<Vec<u8>, u32>>>,
    literals: Index<literals::Lookup<'a>>,
    factions: Index<Linked<BTreeMap<u32, u64>>>,
    wars: Index<Linked<BTreeSet<(u32, u32)>>>,
    overlords: Index<Linked<BTreeMap<u32, u32>>>,
    focuses: Index<Linked<CountryNames<'a>>>,
    slot_links: OnceLock<BTreeMap<u32, Vec<u32>>>,
    focus_rows: Index<Option<CountryRows>>,
    party_rows: Index<Option<CountryRows>>,
    party_leader_rows: Index<Option<CountryRows>>,
    character_rows: Index<Option<CountryRows>>,
    idea_rows: Index<Option<CountryRows>>,
    techs: Index<Techs>,
    projects: Index<Linked<BTreeSet<(u32, u32)>>>,
    political_parties: Index<PoliticalParties<'a>>,
    politics_ruling: Index<Linked<BTreeMap<u32, Option<u32>>>>,
    politics_elections: Index<Linked<BTreeMap<u32, Recorded<bool>>>>,
    country_modifiers: Index<(Modifiers<'a>, BTreeMap<&'a [u8], u32>)>,
    state_modifiers: Index<Modifiers<'a>>,
    state_members: Index<BTreeMap<u32, Vec<u32>>>,
    controller_area_sizes: Index<BTreeMap<u32, i32>>,
    owned_controlled_states: Index<BTreeMap<u32, i32>>,
    opinions: Index<Linked<BTreeMap<(u32, u32), i32>>>,
    resource_amounts: PreparedOperand<'a, Result<StateResources<'a>, &'a str>>,
    invasions: Index<Linked<Invasions<'a>>>,
    script_constants: PreparedOperand<'a, Result<&'a Constants, &'a str>>,
    invasion_prepare_days: PreparedOperand<'a, Result<i32, &'a str>>,
    leader_roles: PreparedOperand<'a, Result<&'a LeaderRoles, &'a str>>,
    appearance: Index<Linked<BTreeMap<u32, u128>>>,
    template_names: Option<TemplateNames<'a>>,
    accepted_projects: Option<Arc<AcceptedProjects>>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AcceptedProjects {
    pub rows: BTreeMap<(u32, u32), replay_data::rows::ProjectRow>,
    pub owners: BTreeSet<u32>,
    pub slots: BTreeSet<u32>,
}

pub struct ContentFacts<'a> {
    pub state_provinces: &'a BTreeMap<u32, Vec<u32>>,
    pub state_content: Result<(), &'a str>,
    pub continents: Result<&'a [String], &'a str>,
    pub ideologies: Result<&'a content_reader_key_names::ideologies::Ideologies, &'a str>,
    pub impassable: &'a BTreeSet<u32>,
    pub neighbor_states: Result<&'a BTreeMap<u32, Vec<u32>>, &'a str>,
    pub terrain: Option<&'a content_reader_map::movement::MovementMap>,
    pub dlcs: &'a BTreeSet<String>,
    pub triggers: Result<&'a TriggerNames, &'a str>,
    pub character_traits: Result<(&'a Traits, &'a LeaderTraits), &'a str>,
    pub tag_aliases: &'a Arc<TagAliases>,
    pub buildings: Result<&'a content_reader_key_names::buildings::Buildings, &'a str>,
    pub resource_bonuses: Result<[i64; 2], &'a str>,
    pub leader_roles: Result<&'a LeaderRoles, &'a str>,
    pub invasion_prepare_days: Result<i32, &'a str>,
    pub script_constants: Result<&'a Constants, &'a str>,
}

impl<'a> ContentFacts<'a> {
    pub fn content_only(
        dlcs: &'a BTreeSet<String>,
        triggers: &'a TriggerNames,
        aliases: &'a Arc<TagAliases>,
    ) -> Self {
        static STATES: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
        static IMPASSABLE: BTreeSet<u32> = BTreeSet::new();
        static BUILDINGS: OnceLock<content_reader_key_names::buildings::Buildings> = OnceLock::new();
        Self {
            state_provinces: &STATES,
            state_content: Err("no recorded content geography is supplied"),
            continents: Err("no recorded continent content is supplied"),
            ideologies: Err("no recorded ideology content is supplied"),
            impassable: &IMPASSABLE,
            neighbor_states: Err("no recorded content geography is supplied"),
            terrain: None,
            dlcs,
            triggers: Ok(triggers),
            character_traits: Err("character traits are unavailable in this content-only check"),
            tag_aliases: aliases,
            buildings: Ok(BUILDINGS.get_or_init(Default::default)),
            resource_bonuses: Err("resource defines are unavailable in this content-only check"),
            leader_roles: Err("character templates are unavailable in this content-only check"),
            invasion_prepare_days: Err(
                "invasion defines are unavailable in this content-only check",
            ),
            script_constants: Err("script constants are unavailable in this content-only check"),
        }
    }
}

#[derive(Debug)]
pub struct LeaderRoles {
    pub(super) templates: Vec<LeaderTemplate>,
    pub(super) by_place: BTreeMap<(Vec<u8>, u32), Vec<usize>>,
    names: BTreeSet<Vec<u8>>,
    layers: Option<content_reader_layers::ResolvedLayers>,
    placed: std::sync::Mutex<Placed>,
}

type PlacedFile = Arc<Vec<ScientistVisibilityTemplate>>;

#[derive(Debug, Default)]
struct Placed {
    files: BTreeMap<Vec<u8>, Result<PlacedFile, String>>,
    origins: BTreeMap<(Vec<u8>, u32), Result<usize, String>>,
    bodies: Vec<(PlacedFile, usize)>,
}

pub struct RoleVisible<'a>(RoleBody<'a>);

enum RoleBody<'a> {
    Content(&'a content_reader_script::triggers::Triggers),
    Placed(PlacedFile, usize),
}

impl std::ops::Deref for RoleVisible<'_> {
    type Target = [content_reader_parser::parser::Item];
    fn deref(&self) -> &Self::Target {
        match &self.0 {
            RoleBody::Content(visible) => visible,
            RoleBody::Placed(file, at) => file[*at].triggers.as_deref().unwrap_or_default(),
        }
    }
}

impl LeaderRoles {
    pub fn read_through(templates: Vec<LeaderTemplate>, layers: content_reader_layers::ResolvedLayers) -> Self {
        Self { layers: Some(layers), ..Self::new(templates) }
    }

    fn placed(&self, file: &[u8], line: u32) -> Result<usize, String> {
        let mut placed = self.placed.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let key = (file.to_vec(), line);
        if let Some(found) = placed.origins.get(&key) {
            return found.clone();
        }
        let name = String::from_utf8_lossy(file).into_owned();
        let parsed = match placed.files.get(file) {
            Some(parsed) => parsed.clone(),
            None => {
                let parsed = self
                    .layers
                    .as_ref()
                    .ok_or_else(|| "the recorded mod stack's content is not open".to_owned())
                    .and_then(|layers| layers.read(&name).map_err(|error| format!("{name} cannot be read: {error}")))
                    .and_then(|bytes| {
                        content_reader_key_names::characters::parse_leader_visibility_templates(&bytes)
                            .map(Arc::new)
                            .map_err(|error| format!("{name} cannot be parsed: {error}"))
                    });
                placed.files.insert(file.to_vec(), parsed.clone());
                parsed
            }
        };
        let found = parsed.and_then(|parsed| {
            let mut at = parsed.iter().enumerate().filter(|(_, template)| template.line == line).map(|(at, _)| at);
            let at = match (at.next(), at.next()) {
                (Some(at), None) => at,
                (None, _) => return Err(format!("no unit leader role's visible opens at {name}:{line}")),
                (Some(_), Some(_)) => return Err(format!("several unit leader roles' visible open at {name}:{line}")),
            };
            parsed[at].triggers.as_ref().map_err(|why| format!("the unit leader role at {name}:{line} is refused: {why}"))?;
            placed.bodies.push((parsed.clone(), at));
            Ok(self.templates.len() + placed.bodies.len() - 1)
        });
        placed.origins.insert(key, found.clone());
        found
    }

    pub fn new(templates: Vec<LeaderTemplate>) -> Self {
        let mut by_place: BTreeMap<_, Vec<usize>> = BTreeMap::new();
        for (index, template) in templates.iter().enumerate() {
            if let Some(line) = template.line {
                by_place
                    .entry((template.path.as_bytes().to_vec(), line))
                    .or_default()
                    .push(index);
            }
        }
        let names = templates
            .iter()
            .map(|template| template.tag.as_bytes().to_ascii_lowercase())
            .collect();
        Self {
            templates,
            by_place,
            names,
            layers: None,
            placed: std::sync::Mutex::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordedTemplate {
    pub tag: u32,
    pub file: Vec<u8>,
    pub line: i32,
    pub generated: bool,
}

fn recorded_template(character: &CharacterRow) -> Option<RecordedTemplate> {
    (character.template_present != 0).then(|| RecordedTemplate {
        tag: character.template_tag,
        file: character.template_file.clone(),
        line: character.template_line,
        generated: character.template_generated != 0,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub enum LeaderVisibility {
    Decided(Eval),
    Unconditioned,
    Conditioned(usize),
}

pub const NULL_STATE: u32 = 0;

pub(super) const CHARACTER_FLAGS: u8 = 3;

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct PartyLeaders {
    pub ruling: bool,
    pub leaders: Vec<PartyLeader>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PartyLeader {
    pub character: Option<u32>,
    pub ordinal: u32,
    pub role: Option<i32>,
    pub id: i32,
}

pub(super) const LEADER: u64 = mask::<replay_data::rows::UnitLeaderRow>(Leader::FIELDS);

pub(super) fn lexical(frame: HourFrame<'_>) -> Result<Rowset<'_, LexicalTokenRow>, Failure> {
    table::<tables::LexicalTokens>(frame, LITERAL)
}

pub(super) fn literal<'a>(
    lexical: &Rowset<'a, LexicalTokenRow>,
    token: u32,
) -> Result<Option<&'a [u8]>, Failure> {
    lexical
        .text(u128::from(token), |row| &row.literal, LITERAL_COLUMN)
        .map_err(walked::<tables::LexicalTokens>)
}

pub(super) fn table<T: TableSchema>(
    frame: HourFrame<'_>,
    mask: u64,
) -> Result<Rowset<'_, T::Row>, Failure> {
    frame.complete::<T>(mask).map_err(|error| {
        let failure = Failure::input(T::ID, "", None, error.to_string());
        failure.resolution_failed();
        failure
    })
}

pub(super) fn characters(
    frame: HourFrame<'_>,
    mask: u64,
) -> Result<Rowset<'_, CharacterRow>, Failure> {
    frame
        .complete::<tables::Characters>(mask)
        .map_err(|error| super::refusal::input_error(tables::Characters::ID, error))
}

pub(super) fn provinces(frame: HourFrame<'_>) -> Result<Rowset<'_, ProvinceRow>, Failure> {
    frame
        .complete::<tables::Provinces>(PROVINCE)
        .map_err(|error| super::refusal::input_error(tables::Provinces::ID, error))
}

pub(super) fn province_values(
    frame: HourFrame<'_>,
) -> Result<impl Iterator<Item = Result<Province, Failure>> + '_, Failure> {
    Ok(each::<tables::Provinces>(provinces(frame)?).map(|row| row.map(|row| Province::from(&*row))))
}

pub(super) fn leaders(
    frame: HourFrame<'_>,
) -> Result<Rowset<'_, replay_data::rows::UnitLeaderRow>, Failure> {
    table::<tables::UnitLeaders>(frame, LEADER)
}

fn required_row<'a, T: TableSchema>(
    frame: HourFrame<'a>,
    mask: u64,
    scope: Option<Store>,
    matches: impl Fn(&T::Row) -> bool,
    message: String,
) -> Result<Cow<'a, T::Row>, Failure> {
    let complete = matches!(
        frame.status(T::ID),
        OperandStatus::Complete | OperandStatus::WrongLayout
    );
    for row in each::<T>(frame.held::<T>(mask)).rev() {
        let row = row?;
        if matches(&row) {
            return Ok(row);
        }
    }
    Err(Failure::input(T::ID, "", if complete { scope } else { None }, message))
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(any(test, feature = "test-support"), derive(replay_spool::Spooled))]
pub enum Query {
    TableStatus(replay_data::rows::TableId),
    TableFresh(replay_data::rows::TableId),
    CountrySlots,
    StateIds,
    StateContinentToken(u32),
    StateExists(u32),
    FocusTree(u32),
    CoreGap,
    TableGap(String),
    UnitStatus(u64),
    RaidWingName(u64),
    IndustrialOrganisation(u64),
    IndustrialOrganisationSize(u64),
    IndustrialOrganisationUnlocked(u64),
    IndustrialOrganisationPolicies(u64),
    IndustrialOrganisationFlags(u64),
    IndustrialOrganisationTasks(u64),
    Clock,
    Slot(u32),
    Tag(u32),
    StateRow(u32),
    ProvinceRow(u32),
    StateController(u32),
    ProvinceController(u32),
    Token(u32),
    TokenOf(Vec<u8>),
    Unlinked(String, u32),
    Faction(u32),
    SameFaction(u32, u32),
    AtWar(u32, u32),
    Overlord(u32),
    OriginalReference(u32),
    SameTag(i32, i32),
    HasFocus(u32, Vec<u8>),
    HasTech(u32, Vec<u8>),
    CompletedProject(u32, Vec<u8>),
    Government(u32),
    NamedIdeology(Vec<u8>),
    IdeologyToken(u32),
    RulingParty(u32),
    ScopedRulingIdeology(u32),
    HasElections(u32),
    Flag(u8, u32, Vec<u8>),
    HasCountryModifier(u32, Vec<u8>, Option<Scope>),
    HasStateModifier(u32, Vec<u8>, Option<Scope>),
    StateResource(u32, u32),
    StateResourceAmount(u32, u32, Option<u32>),
    BuildingLevel(bool, u32, u32),
    StateMembers,
    StateOrder(u32),
    Exists(u32),
    OwnedStateCount(u32),
    CountryCounterInputs(u32),
    CountryStateRoster(u32, bool),
    InvasionOrders(u32),
    IsAllyWith(u32, Option<u32>),
    Variable(Store, Vec<u8>),
    VariableSeed(Store),
    CharacterExists(u32),
    CharacterTag(u32, u32),
    CharacterSkill(u32),
    CharacterRetired(u32),
    CharacterHasTrait(u32, Vec<u8>),
    StateOfValue(i64),
    UnitLeader(u64),
    PartyLeaders(u32),
    CountryCharacters(u32),
    LeaderHasTrait(u32, String),
    CivilWarCountries(u32),
    AdvisorToken(Vec<u8>),
    FactionAdvisors(u64),
    HasCivilWar(u32),
    Enemies(u32),
    HasIdea(u32, String),
    ControlledStateCount(u32),
    Opinion(u32, Option<u32>),
    BuiltinVariable(Store, Vec<u8>),
    HasTemplate(u32, Vec<u8>),
    Character(u32),
    Appearance(u32),
    IsCoreOf(u32, Option<u32>),
    IsClaimedBy(u32, Option<u32>),
    RawCountry(u32),
    Nukes(u32),
    CountryScientists(u32),
    ScientistActive(u32),
    ScientistLevel(u32, u32),
}

impl World<'_> {
    pub fn reread(&self, query: &Query) -> u64 {
        super::inputs::resolved_digest(|| match query {
            Query::TableFresh(table) => super::inputs::digest(&self.table_fresh_unrecorded(*table)),
            Query::TableStatus(table) => {
                super::inputs::digest(&self.table_status_unrecorded(*table))
            }
            Query::CountrySlots => super::inputs::digest(&self.country_slots_unrecorded()),
            Query::StateIds => super::inputs::digest(&self.state_ids_unrecorded()),
            Query::StateContinentToken(state) => {
                super::inputs::digest(&self.state_continent_token_unrecorded(*state))
            }
            Query::StateExists(state) => {
                super::inputs::digest(&self.state_exists_unrecorded(*state))
            }
            Query::FocusTree(raw) => super::inputs::digest(&self.focus_tree_unrecorded(*raw)),
            Query::CoreGap => super::inputs::digest(&self.core_gap_unrecorded()),
            Query::TableGap(table) => super::inputs::digest(&self.table_gap_unrecorded(table)),
            Query::UnitStatus(reference) => {
                super::inputs::fingerprint(&self.unit_status_unrecorded(*reference))
            }
            Query::RaidWingName(reference) => {
                super::inputs::digest(&self.raid_wing_name_unrecorded(*reference))
            }
            Query::IndustrialOrganisation(reference) => {
                super::inputs::fingerprint(&self.industrial_organisation_unrecorded(*reference))
            }
            Query::IndustrialOrganisationSize(reference) => {
                super::inputs::fingerprint(&self.organisation_details(*reference).map(organisation_size))
            }
            Query::IndustrialOrganisationUnlocked(reference) => {
                super::inputs::fingerprint(&self.organisation_details(*reference).map(organisation_unlocked))
            }
            Query::IndustrialOrganisationPolicies(reference) => {
                super::inputs::fingerprint(&self.organisation_details(*reference).map(organisation_policies))
            }
            Query::IndustrialOrganisationFlags(reference) => {
                super::inputs::fingerprint(&self.organisation_details(*reference).map(organisation_flags))
            }
            Query::IndustrialOrganisationTasks(reference) => {
                super::inputs::fingerprint(&self.organisation_details(*reference).map(organisation_tasks))
            }
            Query::Clock => super::inputs::digest(&self.hour),
            Query::Slot(raw) => super::inputs::digest(&self.slot_unrecorded(*raw)),
            Query::Tag(slot) => super::inputs::digest(&self.tag_unrecorded(*slot)),
            Query::StateRow(state) => super::inputs::digest(&self.state_row_unrecorded(*state)),
            Query::ProvinceRow(province) => {
                super::inputs::digest(&self.province_row_unrecorded(*province))
            }
            Query::StateController(state) => {
                super::inputs::digest(&self.state_controller_unrecorded(*state))
            }
            Query::ProvinceController(province) => {
                super::inputs::digest(&self.province_controller_unrecorded(*province))
            }
            Query::Token(token) => super::inputs::digest(&self.token_unrecorded(*token)),
            Query::TokenOf(name) => super::inputs::digest(&self.token_of_unrecorded(name)),
            Query::Unlinked(table, _) => super::inputs::digest(&self.unlinked_unrecorded(table)),
            Query::Faction(slot) => super::inputs::digest(&self.faction_unrecorded(*slot)),
            Query::SameFaction(a, b) => {
                super::inputs::digest(&self.same_faction_unrecorded(*a, *b))
            }
            Query::AtWar(a, b) => super::inputs::digest(&self.at_war_unrecorded(*a, *b)),
            Query::Overlord(subject) => super::inputs::digest(&self.overlord_unrecorded(*subject)),
            Query::OriginalReference(slot) => {
                super::inputs::digest(&self.original_reference_unrecorded(*slot))
            }
            Query::SameTag(a, b) => super::inputs::digest(&self.same_tag_unrecorded(*a, *b)),
            Query::HasFocus(slot, name) => {
                super::inputs::digest(&self.has_focus_unrecorded(*slot, name))
            }
            Query::HasTech(slot, name) => {
                super::inputs::digest(&self.has_tech_unrecorded(*slot, name))
            }
            Query::CompletedProject(slot, name) => {
                super::inputs::digest(&self.completed_project_unrecorded(*slot, name))
            }
            Query::Government(slot) => super::inputs::digest(&self.government_unrecorded(*slot)),
            Query::NamedIdeology(token) => {
                super::inputs::digest(&self.named_ideology_unrecorded(token))
            }
            Query::IdeologyToken(id) => super::inputs::digest(&self.ideology_token_unrecorded(*id)),
            Query::RulingParty(slot) => super::inputs::digest(&self.ruling_party_unrecorded(*slot)),
            Query::ScopedRulingIdeology(slot) => {
                super::inputs::digest(&self.scoped_ruling_ideology_unrecorded(*slot))
            }
            Query::HasElections(slot) => {
                super::inputs::digest(&self.has_elections_unrecorded(*slot))
            }
            Query::Flag(kind, id, name) => {
                super::inputs::digest(&self.flag_unrecorded(*kind, *id, name))
            }
            Query::HasCountryModifier(slot, name, target) => {
                super::inputs::digest(&self.has_country_modifier_unrecorded(*slot, name, *target))
            }
            Query::HasStateModifier(state, name, target) => {
                super::inputs::digest(&self.has_state_modifier_unrecorded(*state, name, *target))
            }
            Query::StateResource(state, resource) => {
                super::inputs::digest(&self.state_resource_unrecorded(*state, *resource))
            }
            Query::StateResourceAmount(state, resource, controller) => super::inputs::digest(
                &self.state_resource_amount_unrecorded(*state, *resource, *controller),
            ),
            Query::BuildingLevel(province, location, building_type) => super::inputs::digest(
                &self.building_level_unrecorded(*province, *location, *building_type),
            ),
            Query::StateMembers => super::inputs::fingerprint(&self.state_members_unrecorded()),
            Query::StateOrder(state) => super::inputs::digest(&self.state_order_unrecorded(*state)),
            Query::Exists(slot) => super::inputs::digest(&self.exists_unrecorded(*slot)),
            Query::OwnedStateCount(slot) => {
                super::inputs::digest(&self.owned_state_count_unrecorded(*slot))
            }
            Query::CountryCounterInputs(slot) => {
                super::inputs::fingerprint(&self.country_counter_inputs_unrecorded(*slot))
            }
            Query::CountryStateRoster(slot, controlled) => {
                super::inputs::digest(&self.country_state_roster_unrecorded(*slot, *controlled))
            }
            Query::InvasionOrders(slot) => {
                super::inputs::fingerprint(&self.invasion_orders_unrecorded(*slot))
            }
            Query::IsAllyWith(slot, other) => {
                super::inputs::digest(&self.is_ally_with_unrecorded(*slot, *other))
            }
            Query::Variable(store, key) => {
                super::inputs::digest(&self.variable_unrecorded(*store, key))
            }
            Query::VariableSeed(store) => {
                super::inputs::digest(&self.variable_seed_unrecorded(*store))
            }
            Query::CharacterExists(id) => {
                super::inputs::digest(&self.character_exists_unrecorded(*id))
            }
            Query::CharacterTag(character, tagged) => {
                super::inputs::digest(&self.character_tag_unrecorded(*character, *tagged))
            }
            Query::CharacterSkill(character) => {
                super::inputs::digest(&self.character_skill_unrecorded(*character))
            }
            Query::CharacterRetired(character) => {
                super::inputs::digest(&self.character_retired_unrecorded(*character))
            }
            Query::CharacterHasTrait(character, name) => {
                super::inputs::digest(&self.character_has_trait_unrecorded(*character, name))
            }
            Query::StateOfValue(raw) => {
                super::inputs::digest(&self.state_of_value_unrecorded(*raw))
            }
            Query::UnitLeader(reference) => {
                super::inputs::fingerprint(&self.unit_leader_unrecorded(*reference))
            }
            Query::PartyLeaders(slot) => {
                super::inputs::fingerprint(&self.party_leaders_unrecorded(*slot))
            }
            Query::CountryCharacters(slot) => {
                super::inputs::digest(&self.country_characters_unrecorded(*slot))
            }
            Query::LeaderHasTrait(slot, name) => {
                super::inputs::digest(&self.leader_has_trait_unrecorded(*slot, name))
            }
            Query::CivilWarCountries(raw) => {
                super::inputs::digest(&self.civil_war_countries_unrecorded(*raw))
            }
            Query::AdvisorToken(name) => {
                super::inputs::digest(&self.advisor_token_unrecorded(name))
            }
            Query::FactionAdvisors(faction) => {
                super::inputs::digest(&self.faction_advisors_unrecorded(*faction))
            }
            Query::HasCivilWar(slot) => {
                super::inputs::digest(&self.has_civil_war_unrecorded(*slot))
            }
            Query::Enemies(slot) => super::inputs::digest(&self.enemies_unrecorded(*slot)),
            Query::HasIdea(slot, name) => {
                super::inputs::digest(&self.has_idea_unrecorded(*slot, name))
            }
            Query::ControlledStateCount(slot) => {
                super::inputs::digest(&self.controlled_state_count_unrecorded(*slot))
            }
            Query::Opinion(owner, target) => {
                super::inputs::digest(&self.opinion_unrecorded(*owner, *target))
            }
            Query::BuiltinVariable(store, name) => {
                super::inputs::digest(&self.builtin_variable_unrecorded(*store, name))
            }
            Query::HasTemplate(slot, wanted) => {
                super::inputs::digest(&self.has_template_unrecorded(*slot, wanted))
            }
            Query::Character(id) => super::inputs::fingerprint(&self.character_unrecorded(*id)),
            Query::Appearance(slot) => {
                super::inputs::fingerprint(&self.appearance_unrecorded(*slot))
            }
            Query::IsCoreOf(state, country) => {
                super::inputs::fingerprint(&self.is_core_of_unrecorded(*state, *country))
            }
            Query::IsClaimedBy(state, country) => {
                super::inputs::fingerprint(&self.is_claimed_by_unrecorded(*state, *country))
            }
            Query::RawCountry(slot) => super::inputs::digest(&self.raw_country_unrecorded(*slot)),
            Query::Nukes(slot) => super::inputs::digest(&self.nukes_unrecorded(*slot)),
            Query::CountryScientists(slot) => {
                super::inputs::digest(&self.country_scientists_unrecorded(*slot))
            }
            Query::ScientistActive(character) => {
                super::inputs::digest(&self.scientist_active_unrecorded(*character))
            }
            Query::ScientistLevel(character, specialization) => super::inputs::digest(
                &self.scientist_level_unrecorded(*character, *specialization),
            ),
        })
    }
}

impl<'a> World<'a> {
    pub(crate) fn table_fresh(&self, table: replay_data::rows::TableId) -> Result<(), Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::TableFresh(table)),
            || self.table_fresh_unrecorded(table),
        )
    }

    fn table_fresh_unrecorded(&self, table: replay_data::rows::TableId) -> Result<(), Failure> {
        if self.frame.sampled_at(table) == Some(self.frame.hour()) {
            Ok(())
        } else {
            Err(Failure::input(
                table,
                "acquisition",
                None,
                format!(
                    "{}: not read this hour, so the values carried from an earlier hour cannot answer this exactly",
                    table.words()
                ),
            ))
        }
    }

    pub(crate) fn table_status(&self, table: replay_data::rows::TableId) -> Result<(), Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::TableStatus(table)),
            || self.table_status_unrecorded(table),
        )
    }

    fn table_status_unrecorded(&self, id: replay_data::rows::TableId) -> Result<(), Failure> {
        use replay_data::rows::TableId;
        match id {
            TableId::Countries => table::<tables::Countries>(self.frame, 0).map(drop),
            TableId::CountryLinks => table::<tables::CountryLinks>(self.frame, 0).map(drop),
            TableId::States => table::<tables::States>(self.frame, 0).map(drop),
            _ => unreachable!("only the core tables' completeness is queried"),
        }
    }

    pub fn country_slots(&self) -> Result<Vec<u32>, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::CountrySlots),
            || self.country_slots_unrecorded(),
        )
    }

    fn country_slots_unrecorded(&self) -> Result<Vec<u32>, Failure> {
        let mut slots = each::<tables::Countries>(table::<tables::Countries>(self.frame, COUNTRY)?)
            .map(|row| row.map(|row| row.canonical_slot))
            .collect::<Result<Vec<u32>, Failure>>()?;
        slots.sort_unstable();
        Ok(slots)
    }

    pub(crate) fn state_ids(&self) -> Result<Vec<u32>, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::StateIds),
            || self.state_ids_unrecorded(),
        )
    }

    fn state_ids_unrecorded(&self) -> Result<Vec<u32>, Failure> {
        Ok(table::<tables::States>(self.frame, 0)?
            .keys()
            .map(|key| key as u32)
            .collect())
    }

    pub(crate) fn state_continent_token(&self, state: u32) -> Result<u32, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::StateContinentToken(state)),
            || self.state_continent_token_unrecorded(state),
        )
    }

    fn state_continent_token_unrecorded(&self, state: u32) -> Result<u32, Failure> {
        table::<tables::States>(self.frame, 0)?;
        let row = self
            .state_map()
            .get(&state)
            .ok_or_else(|| -> Failure { format!("state {state} is not recorded").into() })?;
        row.continent
            .require("continent of states")
            .map_err(|why| {
                Failure::input(
                    replay_data::rows::TableId::States,
                    "continent",
                    Some(Store::State(state)),
                    why,
                )
            })?
            .ok_or_else(|| {
                Failure::input(
                    replay_data::rows::TableId::States,
                    "continent",
                    Some(Store::State(state)),
                    "the native state-template continent capture failed",
                )
            })
    }

    pub(crate) fn state_exists(&self, id: u32) -> Result<bool, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::StateExists(id)),
            || self.state_exists_unrecorded(id),
        )
    }

    fn state_exists_unrecorded(&self, id: u32) -> Result<bool, Failure> {
        table::<tables::States>(self.frame, 0)?;
        Ok(self.state_map().contains_key(&id))
    }

    pub(crate) fn focus_tree(&self, raw: u32) -> Result<Option<Vec<u8>>, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::FocusTree(raw)),
            || self.focus_tree_unrecorded(raw),
        )
    }

    fn focus_tree_unrecorded(&self, raw: u32) -> Result<Option<Vec<u8>>, Failure> {
        const TREE: u64 = mask::<replay_data::rows::FocusRow>(&["tree"]);
        let rows = table::<tables::Focus>(self.frame, TREE)?;
        let row = keyed::<tables::Focus>(&rows, u128::from(raw))?
            .ok_or_else(|| -> Failure { format!("country {raw} has no Focus row").into() })?;
        Ok(row.into_owned().tree)
    }

    pub fn states(&self) -> &ReferencedRows<State> {
        super::inputs::table(self.frame, replay_data::rows::TableId::States);
        self.state_map()
    }

    fn core_unavailable(&self, name: &str) -> Option<Failure> {
        self.core_gaps
            .iter()
            .find(|(gap, _)| *gap == name)
            .map(|(_, why)| why.clone())
    }

    fn state_map(&self) -> &ReferencedRows<State> {
        self.states.get_or_init(|| {
            let _cpu = replay_frame::cpu::scope_phase("triggers:index");
            let rows = super::refusal::deferred(|| {
                each::<tables::States>(self.state_rows)
                    .map(|row| {
                        row.map(|row| {
                            (
                                row.id,
                                State {
                                    id: row.id,
                                    owner: row.owner,
                                    controller: row.controller,
                                    continent: row.continent,
                                },
                            )
                        })
                    })
                    .collect::<Result<BTreeMap<_, _>, Failure>>()
            });
            let (rows, failed) = match rows {
                Ok(rows) => (rows, None),
                Err(why) => (BTreeMap::new(), Some(why)),
            };
            ReferencedRows {
                rows,
                table: tables::States::ID,
                unavailable: self.core_unavailable("States").or(failed),
            }
        })
    }

    fn province_map(&self) -> &ReferencedRows<Province> {
        self.provinces.get_or_init(|| {
            let _cpu = replay_frame::cpu::scope_phase("triggers:index");
            let rows = super::refusal::deferred(|| {
                each::<tables::Provinces>(self.province_rows)
                    .map(|row| row.map(|row| (row.id, Province::from(&*row))))
                    .collect::<Result<BTreeMap<_, _>, Failure>>()
            });
            let (rows, failed) = match rows {
                Ok(rows) => (rows, None),
                Err(why) => (BTreeMap::new(), Some(why)),
            };
            ReferencedRows {
                rows,
                table: tables::Provinces::ID,
                unavailable: self.core_unavailable("Provinces").or(failed),
            }
        })
    }

    pub fn hour(&self) -> Hour {
        self.hour
    }

    pub(crate) fn clock(&self) -> Hour {
        super::inputs::query_resolved(|| super::inputs::Key::World(Query::Clock), || self.hour)
    }

    pub fn links(&self) -> &BTreeMap<u32, u32> {
        super::inputs::table(self.frame, replay_data::rows::TableId::CountryLinks);
        self.links.get_or_init(|| {
            let _cpu = replay_frame::cpu::scope_phase("triggers:index");
            let mut links = BTreeMap::new();
            for row in self.link_rows.iter() {
                match row {
                    Ok(row) => {
                        links.insert(row.id, row.canonical_slot);
                    }
                    Err(why) => walked::<tables::CountryLinks>(why).reached(),
                }
            }
            links
        })
    }

    fn linked(&self, raw: u32) -> Option<u32> {
        match self.link_rows.get(u128::from(raw)) {
            Ok(row) => row.map(|row| row.canonical_slot),
            Err(why) => {
                walked::<tables::CountryLinks>(why).reached();
                None
            }
        }
    }

    fn country(&self, slot: u32) -> Option<(u32, &'a [u8])> {
        let rows = self.country_rows;
        get(&self.countries, || {
            let mut countries = BTreeMap::new();
            for row in rows.with_text(|row| &row.tag, TAG) {
                let (row, tag) = row.map_err(walked::<tables::Countries>)?;
                countries.insert(row.canonical_slot, (row.id, tag));
            }
            Ok(countries)
        })
        .ok()?
        .get(&slot)
        .copied()
    }

    pub fn provinces(&self) -> &ReferencedRows<Province> {
        super::inputs::table(self.frame, replay_data::rows::TableId::Provinces);
        self.province_map()
    }

    pub(super) fn controller_area_sizes(&self) -> Result<&BTreeMap<u32, i32>, Failure> {
        super::inputs::table(self.frame, tables::Provinces::ID);
        get(&self.controller_area_sizes, || {
            let mut counts = BTreeMap::<u32, i32>::new();
            for province in province_values(self.frame)? {
                let area = province?
                    .controller_area
                    .require("controller area of provinces")
                    .map_err(|why| {
                        Failure::input(
                            replay_data::rows::TableId::Provinces,
                            "controller_area",
                            None,
                            why,
                        )
                    })?
                    .ok_or_else(|| {
                        Failure::input(
                            replay_data::rows::TableId::Provinces,
                            "controller_area",
                            None,
                            "controller-area capture unavailable",
                        )
                    })?;
                if area != 0 {
                    let count = counts.entry(area).or_default();
                    *count = count.wrapping_add(1);
                }
            }
            Ok(counts)
        })
    }

    pub fn state_provinces(&self) -> &BTreeMap<u32, Vec<u32>> {
        &self.state_provinces
    }

    pub(super) fn character_tables(&self) -> &character_scope::Tables<'a> {
        self.character_tables.get_or_init(|| {
            let _cpu = replay_frame::cpu::scope_phase("triggers:index");
            super::inputs::unrecorded(|| {
                super::refusal::deferred(|| character_scope::Tables::read_in(self.frame))
            })
        })
    }
}

impl<'a> World<'a> {
    pub fn has_dlc(&self, name: &str) -> bool {
        self.dlcs.contains(name)
    }

    pub fn unrecorded(
        dlcs: &'a BTreeSet<String>,
        triggers: &'a TriggerNames,
        aliases: &'a Arc<TagAliases>,
    ) -> Self {
        static FRAME: OnceLock<DecodedFrame> = OnceLock::new();
        let frame = FRAME.get_or_init(|| DecodedFrame {
            branch: 0,
            hour: 0,
            tables: std::array::from_fn(|_| {
                TableView::Unavailable {
                    reason: "no recording is supplied for this content-only check".into(),
                }
                .into()
            }),
            metrics: Default::default(),
        });
        Self::read(frame as &dyn replay_frame::frame::DecodedSource, ContentFacts::content_only(dlcs, triggers, aliases))
    }

    pub fn coastal(&self, province: u32) -> Result<bool, Failure> {
        let terrain = self.terrain().ok_or(
            "the content movement map is unavailable, so a province's coastal flag is unknown",
        )?;
        Ok(terrain
            .terrain(province)
            .is_some_and(|terrain| terrain.flags & 8 != 0))
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn with_geography(mut self, geography: GeographyInputs<'a>) -> Self {
        self.lazy_geography = Some(geography);
        self
    }

    pub fn neighbor_states(&self) -> Result<&'a BTreeMap<u32, Vec<u32>>, &'a str> {
        if let Some(content) = self.prepared_content {
            return content.geography().neighbors();
        }
        self.lazy_geography
            .map_or_else(|| self.neighbor_states, GeographyInputs::neighbors)
    }

    fn terrain(&self) -> Option<&'a content_reader_map::movement::MovementMap> {
        if let Some(content) = self.prepared_content {
            return content.geography().terrain();
        }
        self.lazy_geography
            .map_or_else(|| self.terrain, GeographyInputs::terrain)
    }

    pub fn new(
        frame: &'a dyn replay_frame::frame::DecodedSource,
        content: ContentFacts<'a>,
    ) -> Result<Self, Failure> {
        Self::new_in(HourFrame::Decoded(frame), content)
    }

    pub fn new_in(frame: HourFrame<'a>, content: ContentFacts<'a>) -> Result<Self, Failure> {
        let world = Self::read_in(frame, content);
        match world.core_gap() {
            Some(gap) => Err(gap),
            None => Ok(world),
        }
    }

    pub fn read(frame: &'a dyn replay_frame::frame::DecodedSource, content: ContentFacts<'a>) -> Self {
        Self::read_in(HourFrame::Decoded(frame), content)
    }

    pub fn read_in(frame: HourFrame<'a>, content: ContentFacts<'a>) -> Self {
        let ((links, countries, provinces, states), core_gaps) =
            super::refusal::deferred(|| core_tables(frame));
        Self {
            frame,
            prepared_content: None,
            hour: frame.hour(),
            core_gaps,
            link_rows: links,
            links: OnceLock::new(),
            country_rows: countries,
            countries: OnceLock::new(),
            province_rows: provinces,
            state_rows: states,
            provinces: OnceLock::new(),
            states: OnceLock::new(),
            state_provinces: content.state_provinces.into(),
            state_content: content.state_content.into(),
            continents: content.continents.into(),
            ideologies: content.ideologies.into(),
            impassable: content.impassable.into(),
            neighbor_states: content.neighbor_states,
            terrain: content.terrain,
            lazy_geography: None,
            dlcs: content.dlcs,
            triggers: content.triggers.into(),
            character_traits: content.character_traits.into(),
            character_tables: OnceLock::new(),
            ideas: Err("idea definitions are not supplied").into(),
            mio_traits: Err("industrial organisation definitions are not supplied").into(),
            equipment_groups: Err("equipment groups are not supplied").into(),
            special_projects: Err("special project definitions are not supplied").into(),
            aliases: content.tag_aliases,
            recorded: OnceLock::new(),
            names: OnceLock::new(),
            variable_names: OnceLock::new(),
            literals: OnceLock::new(),
            factions: OnceLock::new(),
            wars: OnceLock::new(),
            overlords: OnceLock::new(),
            focuses: OnceLock::new(),
            slot_links: OnceLock::new(),
            focus_rows: OnceLock::new(),
            party_rows: OnceLock::new(),
            party_leader_rows: OnceLock::new(),
            character_rows: OnceLock::new(),
            idea_rows: OnceLock::new(),
            techs: OnceLock::new(),
            projects: OnceLock::new(),
            political_parties: OnceLock::new(),
            politics_ruling: OnceLock::new(),
            politics_elections: OnceLock::new(),
            country_modifiers: OnceLock::new(),
            state_modifiers: OnceLock::new(),
            state_members: OnceLock::new(),
            controller_area_sizes: OnceLock::new(),
            owned_controlled_states: OnceLock::new(),
            opinions: OnceLock::new(),
            resource_amounts: content
                .buildings
                .and_then(|buildings| {
                    content
                        .resource_bonuses
                        .map(|bonuses| StateResources::new(frame, buildings, bonuses))
                })
                .into(),
            invasions: OnceLock::new(),
            script_constants: content.script_constants.into(),
            invasion_prepare_days: content.invasion_prepare_days.into(),
            leader_roles: content.leader_roles.into(),
            appearance: OnceLock::new(),
            template_names: None,
            accepted_projects: None,
        }
    }

    pub fn with_accepted_projects(mut self, accepted: Option<Arc<AcceptedProjects>>) -> Self {
        self.accepted_projects = accepted;
        self
    }

    pub(crate) fn accepted_projects(&self) -> Option<&AcceptedProjects> {
        self.accepted_projects.as_deref()
    }

    #[cfg(test)]
    pub(crate) fn with_organisation_content(
        mut self,
        catalog: &'a content_reader_military::mio_traits::Catalog,
        groups: &'a content_reader_key_names::equipment::Groups,
    ) -> Self {
        self.mio_traits = Ok(catalog).into();
        self.equipment_groups = Ok(groups).into();
        self
    }

    #[cfg(test)]
    pub(crate) fn with_special_projects(
        mut self,
        catalog: &'a content_reader_military::special_projects::Catalog,
    ) -> Self {
        self.special_projects = Ok(catalog).into();
        self
    }

    pub(crate) fn with_prepared_special_projects(mut self, content: &'a dyn PreparedContent) -> Self {
        self.special_projects =
            PreparedOperand::lazy(content, self.frame, |content, _| content.special_projects());
        self
    }

    pub(crate) fn with_prepared_content(mut self, content: &'a dyn PreparedContent) -> Self {
        self.prepared_content = Some(content);
        self.state_provinces = PreparedOperand::lazy(content, self.frame, |content, _| {
            content.authored_states().0
        });
        self.impassable = PreparedOperand::lazy(content, self.frame, |content, _| {
            content.authored_states().1
        });
        self.state_content = PreparedOperand::lazy(content, self.frame, |content, _| {
            content.authored_states().2
        });
        self.continents =
            PreparedOperand::lazy(content, self.frame, |content, _| content.continents());
        self.ideologies =
            PreparedOperand::lazy(content, self.frame, |content, _| content.ideologies());
        self.triggers = PreparedOperand::lazy(content, self.frame, |content, _| content.triggers());
        self.ideas = PreparedOperand::lazy(content, self.frame, |content, _| content.ideas());
        self.mio_traits =
            PreparedOperand::lazy(content, self.frame, |content, _| content.mio_traits());
        self.equipment_groups =
            PreparedOperand::lazy(content, self.frame, |content, _| content.equipment_groups());
        self = self.with_prepared_special_projects(content);
        self.character_traits =
            PreparedOperand::lazy(content, self.frame, |content, _| content.character_traits());
        self.leader_roles =
            PreparedOperand::lazy(content, self.frame, |content, _| content.leader_roles());
        self.script_constants =
            PreparedOperand::lazy(content, self.frame, |content, _| content.constants());
        self.invasion_prepare_days =
            PreparedOperand::lazy(content, self.frame, |content, _| content.invasion_days());
        self.resource_amounts = PreparedOperand::lazy(content, self.frame, |content, frame| {
            content
                .resources()
                .map(|(buildings, bonuses)| StateResources::new(frame, buildings, bonuses))
        });
        self
    }

    pub(super) fn character_templates(&self) -> Result<&'a BTreeSet<Vec<u8>>, &'a str> {
        self.leader_roles.map(|roles| &roles.names)
    }

    pub fn core_gap(&self) -> Option<Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::CoreGap),
            || self.core_gap_unrecorded(),
        )
    }

    fn core_gap_unrecorded(&self) -> Option<Failure> {
        self.core_gaps.first().map(|(_, why)| {
            why.reached();
            why.clone()
        })
    }

    pub(super) fn table_gap(&self, table: &str) -> Option<Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::TableGap(table.to_owned())),
            || self.table_gap_unrecorded(table),
        )
    }

    fn table_gap_unrecorded(&self, table: &str) -> Option<Failure> {
        self.core_gaps
            .iter()
            .find(|(name, _)| *name == table)
            .map(|(_, why)| {
                why.reached();
                why.clone()
            })
    }

    pub fn recorded(&self) -> &RecordedScopes<'a> {
        self.recorded.get_or_init(|| {
            let _cpu = replay_frame::cpu::scope_phase("triggers:index");
            super::inputs::unrecorded(|| {
                RecordedScopes::read_in(self.frame, Arc::clone(self.aliases))
            })
        })
    }

    pub fn tables(&self) -> HourFrame<'a> {
        self.frame
    }

    pub fn frame(&self) -> &'a dyn replay_frame::frame::DecodedSource {
        match self.frame {
            HourFrame::Decoded(frame) => frame,
        }
    }

    pub fn unit_status(
        &self,
        reference: u64,
    ) -> Result<Cow<'a, replay_data::rows::DivisionStatusRow>, Failure> {
        super::inputs::query_typed_resolved(
            || super::inputs::Key::World(Query::UnitStatus(reference)),
            || self.unit_status_unrecorded(reference),
        )
    }

    fn unit_status_unrecorded(
        &self,
        reference: u64,
    ) -> Result<Cow<'a, replay_data::rows::DivisionStatusRow>, Failure> {
        const NAME: u64 = mask::<replay_data::rows::DivisionStatusRow>(&[
            "name",
            "naming",
            "country_ordinal",
        ]);
        self.unit(reference)?;
        let rows = self
            .frame
            .complete::<tables::DivisionStatus>(NAME)
            .map_err(|why| {
                super::refusal::input_error(replay_data::rows::TableId::DivisionStatus, why)
            })?;
        keyed::<tables::DivisionStatus>(&rows, u128::from(reference))?
            .ok_or_else(|| -> Failure {
                format!("UNIT {reference:#x} name: cached army name is not recorded").into()
            })
    }

    pub fn raid_wing_name(&self, reference: u64) -> Result<Vec<u8>, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::RaidWingName(reference)),
            || self.raid_wing_name_unrecorded(reference),
        )
    }

    fn raid_wing_name_unrecorded(&self, reference: u64) -> Result<Vec<u8>, Failure> {
        crate::air_wing_names::raid_name(&self.frame, reference).map_err(Into::into)
    }

    pub fn industrial_organisation(
        &self,
        reference: u64,
    ) -> Result<Cow<'a, replay_data::rows::IndustrialOrganisationRow>, Failure> {
        super::inputs::query_typed_resolved(
            || super::inputs::Key::World(Query::IndustrialOrganisation(reference)),
            || self.industrial_organisation_unrecorded(reference),
        )
    }

    fn industrial_organisation_unrecorded(
        &self,
        reference: u64,
    ) -> Result<Cow<'a, replay_data::rows::IndustrialOrganisationRow>, Failure> {
        let rows = table::<tables::IndustrialOrganisations>(
            self.frame,
            tables::IndustrialOrganisations::ID.full_mask(),
        )?;
        keyed::<tables::IndustrialOrganisations>(&rows, u128::from(reference))?.ok_or_else(
            || -> Failure {
                super::refusal::missing_reference(tables::IndustrialOrganisations::ID, None)
            },
        )
    }

    pub(crate) fn industrial_organisation_size(&self, reference: u64) -> Result<i32, Failure> {
        super::inputs::query_typed_resolved(
            || super::inputs::Key::World(Query::IndustrialOrganisationSize(reference)),
            || self.organisation_details(reference).map(organisation_size),
        )
    }

    pub(crate) fn organisation_unlocked(&self, reference: u64) -> Result<Cow<'a, [u32]>, Failure> {
        super::inputs::query_typed_resolved(
            || super::inputs::Key::World(Query::IndustrialOrganisationUnlocked(reference)),
            || self.organisation_details(reference).map(organisation_unlocked),
        )
    }

    pub(crate) fn organisation_policies(
        &self,
        reference: u64,
    ) -> Result<(Cow<'a, [u32]>, u32), Failure> {
        super::inputs::query_typed_resolved(
            || super::inputs::Key::World(Query::IndustrialOrganisationPolicies(reference)),
            || self.organisation_details(reference).map(organisation_policies),
        )
    }

    pub(crate) fn organisation_flags(
        &self,
        reference: u64,
    ) -> Result<Cow<'a, [replay_data::rows::IndustrialOrganisationFlag]>, Failure> {
        super::inputs::query_typed_resolved(
            || super::inputs::Key::World(Query::IndustrialOrganisationFlags(reference)),
            || self.organisation_details(reference).map(organisation_flags),
        )
    }

    pub(crate) fn organisation_tasks(
        &self,
        reference: u64,
    ) -> Result<Cow<'a, [replay_data::rows::IndustrialOrganisationTask]>, Failure> {
        super::inputs::query_typed_resolved(
            || super::inputs::Key::World(Query::IndustrialOrganisationTasks(reference)),
            || self.organisation_details(reference).map(organisation_tasks),
        )
    }

    fn organisation_details(
        &self,
        reference: u64,
    ) -> Result<Cow<'a, replay_data::rows::IndustrialOrganisationDetailsRow>, Failure> {
        let details = table::<tables::IndustrialOrganisationDetails>(
            self.frame,
            tables::IndustrialOrganisationDetails::ID.full_mask(),
        )?;
        keyed::<tables::IndustrialOrganisationDetails>(&details, u128::from(reference))?
            .ok_or_else(|| -> Failure {
                super::refusal::missing_reference(tables::IndustrialOrganisationDetails::ID, None)
            })
    }

    pub fn slot(&self, raw: u32) -> Option<u32> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::Slot(raw)),
            || self.slot_unrecorded(raw),
        )
    }

    fn slot_unrecorded(&self, raw: u32) -> Option<u32> {
        let slot = self.linked(raw);
        if slot.is_none() && raw != 0 {
            let failure = self
                .table_gap("CountryLinks")
                .unwrap_or_else(|| self.unlinked_unrecorded("a country reference"));
            failure.resolution_failed();
        }
        slot
    }

    pub fn tag(&self, slot: u32) -> Option<&'a [u8]> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::Tag(slot)),
            || self.tag_unrecorded(slot),
        )
    }

    fn tag_unrecorded(&self, slot: u32) -> Option<&'a [u8]> {
        super::refusal::reference(self.country(slot).map(|(_, tag)| tag), slot != 0, || {
            self.table_gap("Countries").unwrap_or_else(|| {
                super::refusal::missing_reference(tables::Countries::ID, Some(Store::Country(slot)))
            })
        })
    }

    pub fn state_row(&self, state: u32) -> Option<(u32, u32)> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::StateRow(state)),
            || self.state_row_unrecorded(state),
        )
    }

    fn state_row_unrecorded(&self, state: u32) -> Option<(u32, u32)> {
        {
            self.state_map()
                .get(&state)
                .map(|row| (row.owner, row.controller))
        }
    }

    pub fn province_row(&self, province: u32) -> Option<(u32, u32)> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::ProvinceRow(province)),
            || self.province_row_unrecorded(province),
        )
    }

    fn province_row_unrecorded(&self, province: u32) -> Option<(u32, u32)> {
        {
            self.province_map()
                .get(&province)
                .map(|row| (row.owner, row.controller))
        }
    }

    pub fn state_controller(&self, state: u32) -> Option<u32> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::StateController(state)),
            || self.state_controller_unrecorded(state),
        )
    }

    fn state_controller_unrecorded(&self, state: u32) -> Option<u32> {
        self.slot(self.state_map().get(&state)?.controller)
    }

    pub fn province_controller(&self, province: u32) -> Option<u32> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::ProvinceController(province)),
            || self.province_controller_unrecorded(province),
        )
    }

    fn province_controller_unrecorded(&self, province: u32) -> Option<u32> {
        self.slot(self.province_map().get(&province)?.controller)
    }

    pub fn token(&self, token: u32) -> Result<Option<&'a [u8]>, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::Token(token)),
            || self.token_unrecorded(token),
        )
    }

    fn token_unrecorded(&self, token: u32) -> Result<Option<&'a [u8]>, Failure> {
        let literals = get(&self.literals, || {
            Ok(literals::Lookup::new(lexical(self.frame)?))
        })?;
        Ok(super::refusal::reference(
            literals.get(token)?,
            token != 0,
            || super::refusal::missing_reference(tables::LexicalTokens::ID, None),
        ))
    }

    pub fn token_of(&self, name: &[u8]) -> Result<Option<u32>, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::TokenOf(name.to_vec())),
            || self.token_of_unrecorded(name),
        )
    }

    fn token_of_unrecorded(&self, name: &[u8]) -> Result<Option<u32>, Failure> {
        match self.names()? {
            Names::Own(folds) => match folds.get(name) {
                None => Ok(None),
                Some(Ok(token)) => Ok(Some(token)),
                Some(Err(reason)) => {
                    reason.reached();
                    Err(reason.clone())
                }
            },
        }
    }

    pub(super) fn names(&self) -> Result<&Names, Failure> {
        super::inputs::table(self.frame, replay_data::rows::TableId::LexicalTokens);
        get(&self.names, || {
            let view = self.frame.identity(replay_data::rows::TableId::LexicalTokens);
            let cached = FOLDS.with_borrow(|cache| {
                cache
                    .iter()
                    .find(|(source, _)| view.as_ref().is_some_and(|view| source.same(view)))
                    .map(|(_, folds)| Arc::clone(folds))
            });
            if let Some(folds) = cached {
                return Ok(Names::Own(folds));
            }
            let rows = lexical(self.frame)?;
            let out = Arc::new(Folds::read(
                rows.len(),
                rows.keys()
                    .zip(rows.texts(|row| &row.literal, LITERAL_COLUMN))
                    .map(|(key, literal)| {
                        Ok((key as u32, literal.map_err(walked::<tables::LexicalTokens>)?))
                    }),
            )?);
            if let Some(view) = view {
                FOLDS.with_borrow_mut(|cache| {
                    cache.insert(0, (view, Arc::clone(&out)));
                    cache.truncate(2);
                });
            }
            Ok(Names::Own(out))
        })
    }

    pub(super) fn unlinked(&self, table: &str, raw: u32) -> Failure {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::Unlinked(table.to_owned(), raw)),
            || self.unlinked_unrecorded(table),
        )
    }

    fn unlinked_unrecorded(&self, table: &str) -> Failure {
        Failure::input(
            replay_data::rows::TableId::CountryLinks,
            "canonical_slot",
            None,
            format!("{table} names a country that no recorded country link holds"),
        )
    }

    pub fn faction(&self, slot: u32) -> Result<Option<u64>, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::Faction(slot)),
            || self.faction_unrecorded(slot),
        )
    }

    fn faction_unrecorded(&self, slot: u32) -> Result<Option<u64>, Failure> {
        const RESERVED: u64 = mask::<replay_data::rows::ReservedCountryNamingRow>(&["faction"]);
        const MEMBER: u64 = mask::<replay_data::rows::FactionMemberRow>(&["faction"]);
        if slot == 0 {
            let Some(reserved) =
                only::<tables::ReservedCountryNaming>(table::<tables::ReservedCountryNaming>(self.frame, RESERVED)?)?
            else {
                return Err("reserved country naming does not hold one reserved country".into());
            };
            return reserved
                .faction
                .require("faction of reserved country naming")
                .map_err(|why| {
                    Failure::input(
                        replay_data::rows::TableId::ReservedCountryNaming,
                        "faction",
                        None,
                        why,
                    )
                })?
                .ok_or_else(|| {
                    Failure::input(
                        replay_data::rows::TableId::ReservedCountryNaming,
                        "faction",
                        None,
                        "faction of reserved country naming capture unavailable",
                    )
                })
                .map(|faction| (faction != 0).then_some(faction));
        }
        let (factions, unresolved) = get(&self.factions, || {
            let mut out = BTreeMap::new();
            let mut unresolved = None;
            for row in each::<tables::FactionMembers>(table::<tables::FactionMembers>(self.frame, MEMBER)?) {
                let row = row?;
                if row.country == 0 {
                    continue;
                }
                match self.slot(row.country) {
                    Some(slot) => {
                        out.insert(slot, row.faction);
                    }
                    None => unresolved = Some(self.unlinked("faction members", row.country)),
                }
            }
            Ok((out, unresolved))
        })?;
        linked(factions.get(&slot).copied(), unresolved)
    }

    pub fn same_faction(&self, a: u32, b: u32) -> Result<bool, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::SameFaction(a, b)),
            || self.same_faction_unrecorded(a, b),
        )
    }

    fn same_faction_unrecorded(&self, a: u32, b: u32) -> Result<bool, Failure> {
        Ok(matches!((self.faction(a)?, self.faction(b)?), (Some(x), Some(y)) if x == y))
    }

    pub fn at_war(&self, a: u32, b: u32) -> Result<bool, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::AtWar(a, b)),
            || self.at_war_unrecorded(a, b),
        )
    }

    fn at_war_unrecorded(&self, a: u32, b: u32) -> Result<bool, Failure> {
        let (wars, unresolved) = get(&self.wars, || {
            let mut out = BTreeSet::new();
            let mut unresolved = None;
            for row in each::<tables::WarRelations>(table::<tables::WarRelations>(self.frame, SPAN)?) {
                let row = row?;
                if !ongoing(row.start_hour, row.end_hour, self.hour) {
                    continue;
                }
                match (self.slot(row.first_country), self.slot(row.second_country)) {
                    (Some(x), Some(y)) => {
                        out.insert((x.min(y), x.max(y)));
                    }
                    (None, _) => {
                        unresolved = Some(self.unlinked("war relations", row.first_country))
                    }
                    (_, None) => {
                        unresolved = Some(self.unlinked("war relations", row.second_country));
                    }
                }
            }
            Ok((out, unresolved))
        })?;
        Ok(linked(
            wars.contains(&(a.min(b), a.max(b))).then_some(()),
            unresolved,
        )?
        .is_some())
    }

    pub fn overlord(&self, subject: u32) -> Result<Option<u32>, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::Overlord(subject)),
            || self.overlord_unrecorded(subject),
        )
    }

    fn overlord_unrecorded(&self, subject: u32) -> Result<Option<u32>, Failure> {
        const SUBJECT: u64 =
            mask::<replay_data::rows::SubjectRow>(&["start_hour", "end_hour"]);
        let (overlords, unresolved) = get(&self.overlords, || {
            let mut out = BTreeMap::new();
            let mut unresolved = None;
            for row in each::<tables::Subjects>(table::<tables::Subjects>(self.frame, SUBJECT)?) {
                let row = row?;
                if !ongoing(row.start_hour, row.end_hour, self.hour) {
                    continue;
                }
                match (self.slot(row.subject), self.slot(row.overlord)) {
                    (Some(subject), Some(overlord)) => {
                        out.insert(subject, overlord);
                    }
                    (None, _) => unresolved = Some(self.unlinked("subjects", row.subject)),
                    (_, None) => unresolved = Some(self.unlinked("subjects", row.overlord)),
                }
            }
            Ok((out, unresolved))
        })?;
        linked(overlords.get(&subject).copied(), unresolved)
    }

    pub fn original_reference(&self, slot: u32) -> Result<i32, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::OriginalReference(slot)),
            || self.original_reference_unrecorded(slot),
        )
    }

    fn original_reference_unrecorded(&self, slot: u32) -> Result<i32, Failure> {
        const ORIGINAL: u64 = mask::<CountryAppearanceRow>(&["original_tag_reference"]);
        if slot == 0 {
            return self.recorded().original_tags().original_reference();
        }
        let country = required_row::<tables::Countries>(
            self.frame,
            COUNTRY,
            Some(Store::Country(slot)),
            |row| row.canonical_slot == slot,
            format!("country slot {slot} has no Countries row"),
        )?;
        let row = required_row::<tables::CountryAppearance>(
            self.frame,
            ORIGINAL,
            Some(Store::Country(slot)),
            |row| row.country == country.id,
            format!("country slot {slot} has no country appearance row"),
        )?;
        let raw = row
            .original_tag_reference
            .require("original tag reference of country appearance")
            .map_err(|why| {
                Failure::input(
                    replay_data::rows::TableId::CountryAppearance,
                    "original_tag_reference",
                    Some(Store::Country(slot)),
                    why,
                )
            })?
            .ok_or_else(|| {
                Failure::input(
                    replay_data::rows::TableId::CountryAppearance,
                    "original_tag_reference",
                    Some(Store::Country(slot)),
                    "original tag reference of country appearance capture refused",
                )
            })?;
        Ok(if raw > 0 { raw } else { country.id as i32 })
    }

    pub(crate) fn same_tag(&self, a: i32, b: i32) -> Result<bool, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::SameTag(a, b)),
            || self.same_tag_unrecorded(a, b),
        )
    }

    fn same_tag_unrecorded(&self, a: i32, b: i32) -> Result<bool, Failure> {
        if a == b {
            return Ok(true);
        }
        if a == 0 || b == 0 {
            return Ok(false);
        }
        Ok(self.recorded().raw_slot(a as u32)? == self.recorded().raw_slot(b as u32)?)
    }

    pub(super) fn has_focus(&self, slot: u32, name: &[u8]) -> Result<bool, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::HasFocus(slot, name.to_vec())),
            || self.has_focus_unrecorded(slot, name),
        )
    }

    fn has_focus_unrecorded(&self, slot: u32, name: &[u8]) -> Result<bool, Failure> {
        let rows = table::<tables::CompletedFocuses>(self.frame, 0)?;
        let Some(countries) = self.country_rows(&self.focus_rows, &rows, 32)? else {
            return self.has_focus_indexed(slot, name);
        };
        let mut found = false;
        for &raw in self.slot_links(slot) {
            for token in rows.within(countries.of(raw)).texts(|row| &row.token, FOCUS_TOKEN) {
                if token.map_err(walked::<tables::CompletedFocuses>)? == name {
                    found = true;
                    break;
                }
            }
            if found {
                break;
            }
        }
        let unresolved = countries
            .unlinked
            .map(|(_, last)| self.unlinked("completed focuses", last));
        Ok(linked(found.then_some(()), &unresolved)?.is_some())
    }

    fn has_focus_indexed(&self, slot: u32, name: &[u8]) -> Result<bool, Failure> {
        let (focuses, unresolved) = get(&self.focuses, || {
            let mut out = BTreeSet::new();
            let mut unresolved = None;
            let rows = table::<tables::CompletedFocuses>(self.frame, 0)?;
            for row in rows.with_text(|row| &row.token, FOCUS_TOKEN) {
                let (row, token) = row.map_err(walked::<tables::CompletedFocuses>)?;
                match self.slot(row.country) {
                    Some(slot) => {
                        out.insert((slot, token));
                    }
                    None => unresolved = Some(self.unlinked("completed focuses", row.country)),
                }
            }
            Ok((out, unresolved))
        })?;
        Ok(linked(focuses.contains(&(slot, name)).then_some(()), unresolved)?.is_some())
    }

    pub(super) fn has_tech(&self, slot: u32, name: &[u8]) -> Result<bool, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::HasTech(slot, name.to_vec())),
            || self.has_tech_unrecorded(slot, name),
        )
    }

    fn has_tech_unrecorded(&self, slot: u32, name: &[u8]) -> Result<bool, Failure> {
        const ID: usize = column::<replay_data::rows::TechnologyDefinitionRow>("id");
        const CLASSIFIER: u64 =
            mask::<replay_data::rows::ResearchTechnologyRow>(&["classifier"]);
        let techs = get(&self.techs, || {
            let definitions = table::<tables::TechnologyDefinitions>(self.frame, 0)?;
            let mut names: BTreeMap<i32, Vec<u8>> = BTreeMap::new();
            let mut first = BTreeMap::new();
            for row in definitions.with_text(|row| &row.id, ID) {
                let (row, id) = row.map_err(walked::<tables::TechnologyDefinitions>)?;
                let name = content_reader_parser::parser::c_string(id).to_ascii_lowercase();
                first.entry(name.clone()).or_insert(row.database_index);
                names.insert(row.database_index, name);
            }
            let mut techs = Techs::default();
            for row in each::<tables::ResearchTechnologies>(table::<tables::ResearchTechnologies>(self.frame, CLASSIFIER)?) {
                let row = row?;
                if row.classifier != engine_research::ctechnology::CLASSIFIER_COMPLETED {
                    continue;
                }
                match (
                    self.slot(row.country),
                    super::refusal::reference(
                        names.get(&row.database_index),
                        row.database_index >= 0,
                        || {
                            super::refusal::missing_reference(
                                tables::TechnologyDefinitions::ID,
                                None,
                            )
                        },
                    ),
                ) {
                    (Some(slot), Some(name)) => {
                        if first.get(name) == Some(&row.database_index) {
                            techs.researched.insert((slot, name.clone()));
                        }
                    }
                    (Some(slot), None) => {
                        techs.unnamed.insert(slot);
                    }
                    (None, _) => techs.unresolved = true,
                }
            }
            Ok(techs)
        })?;
        if techs.researched.contains(&(
            slot,
            content_reader_parser::parser::c_string(name).to_ascii_lowercase(),
        )) {
            return Ok(true);
        }
        if techs.unresolved || techs.unnamed.contains(&slot) {
            return Err(format!(
                "has_tech {}: a completed technology has no recorded definition or country",
                String::from_utf8_lossy(name)
            )
            .into());
        }
        Ok(false)
    }

    pub(super) fn completed_project(&self, slot: u32, name: &[u8]) -> Result<bool, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::CompletedProject(slot, name.to_vec())),
            || self.completed_project_unrecorded(slot, name),
        )
    }

    fn completed_project_unrecorded(&self, slot: u32, name: &[u8]) -> Result<bool, Failure> {
        const COMPLETED: u64 =
            mask::<replay_data::rows::ProjectRow>(&["completed", "definition_id"]);
        let token = self.token_of(name)?;
        if let Some(accepted) = self.accepted_projects() {
            let Some(token) = token else {
                return Err(format!("project {} has no recorded spelling at this hour, so whether it was completed is not recorded", String::from_utf8_lossy(name)).into());
            };
            if accepted.rows.values().any(|row| row.completed != 0 && row.definition_id == token && self.slot(row.owner) == Some(slot)) {
                return Ok(true);
            }
            if accepted.slots.contains(&slot) {
                return Ok(false);
            }
            return Err(format!("whether country slot {slot} completed this project is not recorded: not every owner of that country has a Projects read supplying this hour").into());
        }
        let Some(token) = token else {
            return Ok(false);
        };
        let (projects, unresolved) = get(&self.projects, || {
            let mut out = BTreeSet::new();
            let mut unresolved = None;
            for row in each::<tables::Projects>(table::<tables::Projects>(self.frame, COMPLETED)?) {
                let row = row?;
                if row.completed == 0 {
                    continue;
                }
                match self.slot(row.owner) {
                    Some(slot) => {
                        out.insert((slot, row.definition_id));
                    }
                    None => unresolved = Some(self.unlinked("projects", row.owner)),
                }
            }
            Ok((out, unresolved))
        })?;
        Ok(linked(projects.contains(&(slot, token)).then_some(()), unresolved)?.is_some())
    }

    fn slot_links(&self, slot: u32) -> &[u32] {
        self.slot_links
            .get_or_init(|| {
                let mut out: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
                for row in self.link_rows.iter() {
                    match row {
                        Ok(row) => out.entry(row.canonical_slot).or_default().push(row.id),
                        Err(why) => walked::<tables::CountryLinks>(why).reached(),
                    }
                }
                out
            })
            .get(&slot)
            .map_or(&[], Vec::as_slice)
    }

    fn country_rows<'s, R: RowCodec>(
        &'s self,
        cell: &'s Index<Option<CountryRows>>,
        rows: &Rowset<'_, R>,
        shift: u32,
    ) -> Result<Option<&'s CountryRows>, Failure> {
        let found = get(cell, || {
            CountryRows::new(rows.keys().map(|key| Ok((key >> shift) as u32)), |raw| {
                self.linked(raw).is_some()
            })
        })?
        .as_ref();
        if let Some((_, raw)) = found.and_then(CountryRows::first_unlinked) {
            self.slot(raw);
        }
        Ok(found)
    }

    fn political_parties(&self) -> Result<&PoliticalParties<'a>, Failure> {
        get(&self.political_parties, || {
            let mut out = PoliticalParties {
                ruling: BTreeMap::new(),
                unresolved: None,
                ids: BTreeMap::new(),
                tokens: BTreeMap::new(),
                members: BTreeSet::new(),
            };
            let rows = table::<tables::PoliticalParties>(self.frame, RULING)?;
            for row in rows.with_text(|row| &row.token, PARTY_TOKEN) {
                let (row, token) = row.map_err(walked::<tables::PoliticalParties>)?;
                if let Some(previous) = out.ids.insert(row.id, token)
                    && !previous.eq_ignore_ascii_case(token)
                {
                    return Err(format!(
                        "political parties ideology ID {} has conflicting tokens",
                        row.id
                    )
                    .into());
                }
                if let Some(previous) = out.tokens.insert(token.to_ascii_lowercase(), row.id)
                    && previous != row.id
                {
                    return Err(format!(
                        "political parties ideology {} has conflicting native IDs",
                        String::from_utf8_lossy(token)
                    )
                    .into());
                }
                match self.slot(row.country) {
                    Some(slot) => {
                        out.members.insert((slot, row.id));
                        if row.is_ruling != 0 {
                            out.ruling.insert(slot, row.id);
                        }
                    }
                    None => out.unresolved = Some(self.unlinked("political parties", row.country)),
                }
            }
            Ok(out)
        })
    }

    pub(super) fn government(&self, slot: u32) -> Result<Option<u32>, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::Government(slot)),
            || self.government_unrecorded(slot),
        )
    }

    fn government_unrecorded(&self, slot: u32) -> Result<Option<u32>, Failure> {
        let parties = self.political_parties()?;
        linked(parties.ruling.get(&slot).copied(), &parties.unresolved)
    }

    pub fn named_ideology(&self, token: &[u8]) -> Result<(u32, &'a [u8]), Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::NamedIdeology(token.to_vec())),
            || self.named_ideology_unrecorded(token),
        )
    }

    fn named_ideology_unrecorded(&self, token: &[u8]) -> Result<(u32, &'a [u8]), Failure> {
        let parties = self.political_parties()?;
        let id = parties
            .tokens
            .get(&token.to_ascii_lowercase())
            .copied()
            .ok_or_else(|| -> Failure {
                format!(
                    "ideology {} has no recorded native ID/token",
                    String::from_utf8_lossy(token)
                )
                .into()
            })?;
        Ok((id, parties.ids[&id]))
    }

    pub(super) fn ideology_token(&self, id: u32) -> Result<Option<&'a [u8]>, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::IdeologyToken(id)),
            || self.ideology_token_unrecorded(id),
        )
    }

    fn ideology_token_unrecorded(&self, id: u32) -> Result<Option<&'a [u8]>, Failure> {
        Ok(super::refusal::reference(
            self.political_parties()?.ids.get(&id).copied(),
            id != 0,
            || super::refusal::missing_reference(tables::PoliticalParties::ID, None),
        ))
    }

    pub(super) fn ruling_party(&self, slot: u32) -> Result<Option<u32>, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::RulingParty(slot)),
            || self.ruling_party_unrecorded(slot),
        )
    }

    fn raw_ruling_party(&self, slot: u32) -> Result<Option<Option<u32>>, Failure> {
        const RULING_PARTY: u64 = mask::<replay_data::rows::PoliticsRow>(&["ruling_party"]);
        let (ruling, unresolved) = get(&self.politics_ruling, || {
            let mut out = BTreeMap::new();
            let mut unresolved = None;
            for row in each::<tables::Politics>(table::<tables::Politics>(self.frame, RULING_PARTY)?) {
                let row = row?;
                match self.slot(row.country) {
                    Some(slot) => {
                        out.insert(slot, row.ruling_party);
                    }
                    None => unresolved = Some(self.unlinked("politics", row.country)),
                }
            }
            Ok((out, unresolved))
        })?;
        linked(ruling.get(&slot).copied(), unresolved)
    }

    fn ruling_party_unrecorded(&self, slot: u32) -> Result<Option<u32>, Failure> {
        self.raw_ruling_party(slot)
            .and_then(|ruling| {
                ruling.ok_or_else(|| {
                    Failure::input(
                        tables::Politics::ID,
                        "ruling_party",
                        Some(Store::Country(slot)),
                        format!("Politics lacks country slot {slot}"),
                    )
                })
            })
            .map_err(|why| why.context(|why| format!("ruling party of country slot {slot}: {why}")))
    }

    pub(super) fn scoped_ruling_ideology(&self, slot: u32) -> Result<(u32, &'a [u8]), Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::ScopedRulingIdeology(slot)),
            || self.scoped_ruling_ideology_unrecorded(slot),
        )
    }

    fn scoped_ruling_ideology_unrecorded(&self, slot: u32) -> Result<(u32, &'a [u8]), Failure> {
        let id = self
            .raw_ruling_party(slot)?
            .flatten()
            .ok_or("Politics lacks ruling ideology")?;
        let parties = self.political_parties()?;
        if !parties.members.contains(&(slot, id)) {
            return Err("political parties lacks ruling ideology".into());
        }
        Ok((id, parties.ids[&id]))
    }

    pub(super) fn has_elections(&self, slot: u32) -> Result<bool, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::HasElections(slot)),
            || self.has_elections_unrecorded(slot),
        )
    }

    fn has_elections_unrecorded(&self, slot: u32) -> Result<bool, Failure> {
        const ELECTIONS: u64 = mask::<replay_data::rows::PoliticsRow>(&["has_elections"]);
        let (elections, unresolved) = get(&self.politics_elections, || {
            let mut out = BTreeMap::new();
            let mut unresolved = None;
            for row in each::<tables::Politics>(table::<tables::Politics>(self.frame, ELECTIONS)?) {
                let row = row?;
                match self.slot(row.country) {
                    Some(slot) => {
                        out.insert(slot, row.has_elections);
                    }
                    None => unresolved = Some(self.unlinked("politics", row.country)),
                }
            }
            Ok((out, unresolved))
        })?;
        linked(elections.get(&slot).copied(), unresolved)?
            .ok_or_else(|| -> Failure { format!("Politics lacks country slot {slot}").into() })?
            .require("has elections of politics")
            .map_err(|why| {
                Failure::input(
                    replay_data::rows::TableId::Politics,
                    "has_elections",
                    Some(Store::Country(slot)),
                    why,
                )
            })
            .copied()
    }

    pub(super) fn flag(
        &self,
        kind: u8,
        id: u32,
        name: &[u8],
    ) -> Result<Option<(i16, i32)>, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::Flag(kind, id, name.to_vec())),
            || self.flag_unrecorded(kind, id, name),
        )
    }

    fn flag_unrecorded(
        &self,
        kind: u8,
        id: u32,
        name: &[u8],
    ) -> Result<Option<(i16, i32)>, Failure> {
        const VALUE: u64 = mask::<replay_data::rows::ScriptFlagRow>(&["value", "set_hour"]);
        let Some(token) = self.token_of(name)? else {
            return Ok(None);
        };
        let flags = table::<tables::ScriptFlags>(self.frame, VALUE)?;
        let key = (u128::from(kind) << 64) | (u128::from(id) << 32) | u128::from(token);
        Ok(keyed::<tables::ScriptFlags>(&flags, key)?.map(|row| (row.value, row.set_hour)))
    }

    pub(super) fn has_country_modifier(
        &self,
        slot: u32,
        name: &[u8],
        target: Option<Scope>,
    ) -> Result<bool, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::HasCountryModifier(slot, name.to_vec(), target)),
            || self.has_country_modifier_unrecorded(slot, name, target),
        )
    }

    fn has_country_modifier_unrecorded(
        &self,
        slot: u32,
        name: &[u8],
        target: Option<Scope>,
    ) -> Result<bool, Failure> {
        const TARGET: u64 =
            mask::<replay_data::rows::DynamicModifierRow>(&["scope_country", "scope_state"]);
        const TOKEN: usize = column::<replay_data::rows::DynamicModifierRow>("token");
        let (modifiers, unlinked) = get(&self.country_modifiers, || {
            let mut modifiers: Modifiers<'a> = BTreeMap::new();
            let mut unlinked = BTreeMap::new();
            let rows = table::<tables::DynamicModifiers>(self.frame, TARGET)?;
            for row in rows.with_text(|row| &row.token, TOKEN) {
                let (row, token) = row.map_err(walked::<tables::DynamicModifiers>)?;
                match self.slot(row.country) {
                    Some(slot) => {
                        modifiers
                            .entry((slot, token))
                            .or_default()
                            .push((row.scope_country, row.scope_state));
                    }
                    None => {
                        unlinked.insert(token, row.country);
                    }
                }
            }
            Ok((modifiers, unlinked))
        })?;
        let entries = modifiers.get(&(slot, name)).map_or(&[][..], Vec::as_slice);
        let found = self.modifier_matches(entries, target, "dynamic modifiers");
        if let Some(raw) = unlinked.get(name) {
            if target.is_some() && found == Ok(true) {
                return found;
            }
            return Err(self.unlinked("dynamic modifiers", *raw));
        }
        found
    }

    pub(super) fn has_state_modifier(
        &self,
        state: u32,
        name: &[u8],
        target: Option<Scope>,
    ) -> Result<bool, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::HasStateModifier(state, name.to_vec(), target)),
            || self.has_state_modifier_unrecorded(state, name, target),
        )
    }

    fn has_state_modifier_unrecorded(
        &self,
        state: u32,
        name: &[u8],
        target: Option<Scope>,
    ) -> Result<bool, Failure> {
        const TARGET: u64 =
            mask::<replay_data::rows::StateDynamicModifierRow>(&["scope_country", "scope_state"]);
        const TOKEN: usize = column::<replay_data::rows::StateDynamicModifierRow>("token");
        let modifiers = get(&self.state_modifiers, || {
            let mut modifiers: Modifiers<'a> = BTreeMap::new();
            let rows = table::<tables::StateDynamicModifiers>(self.frame, TARGET)?;
            for row in rows.with_text(|row| &row.token, TOKEN) {
                let (row, token) = row.map_err(walked::<tables::StateDynamicModifiers>)?;
                modifiers
                    .entry((row.state, token))
                    .or_default()
                    .push((row.scope_country, row.scope_state));
            }
            Ok(modifiers)
        })?;
        self.modifier_matches(
            modifiers.get(&(state, name)).map_or(&[][..], Vec::as_slice),
            target,
            "state dynamic modifiers",
        )
    }

    fn modifier_matches(
        &self,
        entries: &[(u32, i32)],
        target: Option<Scope>,
        table: &str,
    ) -> Result<bool, Failure> {
        let country = match target {
            Some(Scope::Country(slot) | Scope::CountryLeader { country: slot, .. }) => {
                let raw = self.raw_country(slot).ok_or_else(|| -> Failure {
                    format!("has_dynamic_modifier target country slot {slot} has no Countries row")
                        .into()
                })?;
                ((raw as i32) > 0).then_some((raw, slot))
            }
            _ => None,
        };
        let state = match target {
            Some(Scope::State(state)) if (state as i32) > 0 => Some(state as i32),
            _ => None,
        };
        let mut unresolved = None;
        if let Some((raw, slot)) = country {
            for &(country, _) in entries {
                if country == raw {
                    return Ok(true);
                }
                if country != 0 {
                    match self.slot(country) {
                        Some(other) if other == slot => return Ok(true),
                        Some(_) => {}
                        None => unresolved = Some(self.unlinked(table, country)),
                    }
                }
            }
        } else if let Some(state) = state {
            return Ok(entries.iter().any(|&(_, stored)| stored == state));
        } else {
            return Ok(entries
                .first()
                .is_some_and(|&(country, state)| (country as i32) <= 0 && state == 0));
        }
        match unresolved {
            Some(why) => Err(why),
            None => Ok(false),
        }
    }

    fn resource_amounts(&self) -> Result<&StateResources<'a>, Failure> {
        self.resource_amounts.as_ref().map_err(|why| {
            Failure::from(*why)
                .context(|why| format!("the state resource amounts are unavailable: {why}"))
        })
    }

    pub fn state_resource(&self, state: u32, resource: u32) -> Result<i64, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::StateResource(state, resource)),
            || self.state_resource_unrecorded(state, resource),
        )
    }

    fn state_resource_unrecorded(&self, state: u32, resource: u32) -> Result<i64, Failure> {
        self.state_resource_amount(state, resource, self.state_controller(state))
    }

    pub(super) fn state_resource_amount(
        &self,
        state: u32,
        resource: u32,
        controller: Option<u32>,
    ) -> Result<i64, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::StateResourceAmount(state, resource, controller)),
            || self.state_resource_amount_unrecorded(state, resource, controller),
        )
    }

    fn state_resource_amount_unrecorded(
        &self,
        state: u32,
        resource: u32,
        controller: Option<u32>,
    ) -> Result<i64, Failure> {
        self.resource_amounts()?
            .amount(state, resource, controller, self)
    }

    pub fn building_level(
        &self,
        province: bool,
        location: u32,
        building_type: u32,
    ) -> Result<Option<i16>, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::BuildingLevel(province, location, building_type)),
            || self.building_level_unrecorded(province, location, building_type),
        )
    }

    fn building_level_unrecorded(
        &self,
        province: bool,
        location: u32,
        building_type: u32,
    ) -> Result<Option<i16>, Failure> {
        const LEVEL: u64 = mask::<replay_data::rows::BuildingRow>(&["level"]);
        let key = (u128::from(location) << 32) | u128::from(building_type);
        Ok(if province {
            let rows = table::<tables::ProvinceBuildings>(self.frame, LEVEL)?;
            keyed::<tables::ProvinceBuildings>(&rows, key)?.map(|row| row.level)
        } else {
            let rows = table::<tables::StateBuildings>(self.frame, LEVEL)?;
            keyed::<tables::StateBuildings>(&rows, key)?.map(|row| row.level)
        })
    }

    pub(super) fn state_members(&self) -> Result<&BTreeMap<u32, Vec<u32>>, Failure> {
        super::inputs::query_typed_resolved(
            || super::inputs::Key::World(Query::StateMembers),
            || self.state_members_unrecorded(),
        )
    }

    fn state_members_unrecorded(&self) -> Result<&BTreeMap<u32, Vec<u32>>, Failure> {
        get(&self.state_members, || {
            let mut out: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
            for (&id, row) in self.province_map() {
                out.entry(row.state).or_default().push(id);
            }
            Ok(out)
        })
    }

    pub fn state_order(&self, state: u32) -> Result<&'a [u32], Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::StateOrder(state)),
            || self.state_order_unrecorded(state),
        )
    }

    fn state_order_unrecorded(&self, state: u32) -> Result<&'a [u32], Failure> {
        self.state_content.map_err(Failure::from)?;
        let content = self
            .state_provinces
            .get(&state)
            .ok_or_else(|| -> Failure { format!("state {state} is not in the content").into() })?;
        let recorded = self
            .state_members()?
            .get(&state)
            .map_or(&[][..], Vec::as_slice);
        if recorded.len() != content.len()
            || content.iter().any(|id| recorded.binary_search(id).is_err())
        {
            return Err(format!(
                "state {state}'s provinces differ from the content, so their native order is unknown"
            ).into());
        }
        Ok(content)
    }

    pub fn exists(&self, slot: u32) -> Result<bool, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::Exists(slot)),
            || self.exists_unrecorded(slot),
        )
    }

    fn exists_unrecorded(&self, slot: u32) -> Result<bool, Failure> {
        self.owned_state_count(slot).map(|count| count > 0)
    }

    pub(crate) fn owned_state_count(&self, slot: u32) -> Result<i32, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::OwnedStateCount(slot)),
            || self.owned_state_count_unrecorded(slot),
        )
    }

    fn owned_state_count_unrecorded(&self, slot: u32) -> Result<i32, Failure> {
        if slot == 0 {
            return self.recorded().owned_states(0);
        }
        Ok(self.country_counter_inputs(slot)?.owned_state_count)
    }

    pub fn country_counter_inputs(
        &self,
        slot: u32,
    ) -> Result<Cow<'a, CountryCounterInputRow>, Failure> {
        super::inputs::query_typed_resolved(
            || super::inputs::Key::World(Query::CountryCounterInputs(slot)),
            || self.country_counter_inputs_unrecorded(slot),
        )
    }

    fn country_counter_inputs_unrecorded(
        &self,
        slot: u32,
    ) -> Result<Cow<'a, CountryCounterInputRow>, Failure> {
        let rows = table::<tables::CountryCounterInputs>(
            self.frame,
            tables::CountryCounterInputs::ID.full_mask(),
        )?;
        let raw = self.raw_country(slot).ok_or_else(|| -> Failure {
            format!("country slot {slot} has no Countries row").into()
        })?;
        keyed::<tables::CountryCounterInputs>(&rows, u128::from(raw))?.ok_or_else(|| {
            Failure::input(
                replay_data::rows::TableId::CountryCounterInputs,
                "",
                Some(Store::Country(slot)),
                format!("country {raw} has no country counter inputs row"),
            )
        })
    }

    pub(super) fn ordered_country_states(
        &self,
        slot: u32,
        controlled: bool,
    ) -> Result<Cow<'a, [u32]>, Failure> {
        let inputs = self.country_counter_inputs(slot)?;
        let field = if controlled {
            "controlled_states"
        } else {
            "owned_states"
        };
        states_of(&inputs, controlled)
            .require(&replay_data::rows::TableId::CountryCounterInputs.field_words(field))
            .map_err(|why| {
                Failure::input(
                    replay_data::rows::TableId::CountryCounterInputs,
                    field,
                    Some(Store::Country(slot)),
                    why,
                )
            })?
            .as_deref()
            .ok_or_else(|| {
                Failure::input(
                    replay_data::rows::TableId::CountryCounterInputs,
                    field,
                    Some(Store::Country(slot)),
                    format!("{} capture unavailable", replay_data::rows::TableId::CountryCounterInputs.field_words(field)),
                )
            })?;
        Ok(part(inputs, |inputs| state_list(inputs, controlled)))
    }

    pub(super) fn country_state_roster(
        &self,
        slot: u32,
        controlled: bool,
    ) -> Result<Option<Cow<'a, [u32]>>, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::CountryStateRoster(slot, controlled)),
            || self.country_state_roster_unrecorded(slot, controlled),
        )
    }

    fn country_state_roster_unrecorded(
        &self,
        slot: u32,
        controlled: bool,
    ) -> Result<Option<Cow<'a, [u32]>>, Failure> {
        let inputs = self.country_counter_inputs(slot)?;
        let name = if controlled {
            "controlled states of country counter inputs"
        } else {
            "owned states of country counter inputs"
        };
        match states_of(&inputs, controlled) {
            Recorded::NotRecorded => Ok(None),
            Recorded::Value(Some(_)) => {
                Ok(Some(part(inputs, |inputs| state_list(inputs, controlled))))
            }
            Recorded::Value(None) => Err(Failure::input(
                replay_data::rows::TableId::CountryCounterInputs,
                if controlled {
                    "controlled_states"
                } else {
                    "owned_states"
                },
                Some(Store::Country(slot)),
                format!("{name} capture unavailable"),
            )),
        }
    }

    pub(super) fn invasion_orders(
        &self,
        slot: u32,
    ) -> Result<PartialCollection<'_, Invasion<'a>>, Failure> {
        super::inputs::query_typed_resolved(
            || super::inputs::Key::World(Query::InvasionOrders(slot)),
            || self.invasion_orders_unrecorded(slot),
        )
    }

    fn invasion_orders_unrecorded(
        &self,
        slot: u32,
    ) -> Result<PartialCollection<'_, Invasion<'a>>, Failure> {
        const COUNT: u64 =
            mask::<replay_data::rows::ReservedCountryNamingRow>(&["land_theatre_count"]);
        const THEATRE: u64 = mask::<replay_data::rows::LandTheatreRow>(&["owner", "ordinal"]);
        const GROUP: u64 = mask::<replay_data::rows::OrdersGroupRow>(&[
            "is_army_group",
            "theatre",
            "theatre_ordinal",
        ]);
        const ROOTS: usize = column::<replay_data::rows::OrdersGroupPlanRow>("primary_orders");
        const ORDER: u64 = mask::<replay_data::rows::ArmyOrderRow>(&["order_type", "active"]);
        const PATH: usize = column::<replay_data::rows::ArmyOrderRow>("path");
        const SUCCESSORS: u64 = mask::<replay_data::rows::ArmyOrderPlanRow>(&["successors"]);
        if slot == 0 {
            let Some(reserved) =
                only::<tables::ReservedCountryNaming>(table::<tables::ReservedCountryNaming>(self.frame, COUNT)?)?
            else {
                return Err("reserved country naming does not hold one reserved country".into());
            };
            let count = reserved
                .land_theatre_count
                .require("land theatre count of reserved country naming")
                .map_err(|why| {
                    Failure::input(
                        replay_data::rows::TableId::ReservedCountryNaming,
                        "land_theatre_count",
                        None,
                        why,
                    )
                })?
                .ok_or_else(|| {
                    Failure::input(
                        replay_data::rows::TableId::ReservedCountryNaming,
                        "land_theatre_count",
                        None,
                        "land theatre count of reserved country naming capture unavailable",
                    )
                })?;
            if count == 0 {
                if let Ok(rows) = self.frame.complete::<tables::LandTheatres>(THEATRE) {
                    for row in each::<tables::LandTheatres>(rows) {
                        if row?.owner == 0 {
                            return Err(
                                "reserved-country land theatres contradict their certificate".into()
                            );
                        }
                    }
                }
                return Ok(PartialCollection {
                    known: &[],
                    unresolved: None,
                });
            }
            let mut ids = BTreeSet::new();
            let mut ordinals = BTreeSet::new();
            for row in each::<tables::LandTheatres>(table::<tables::LandTheatres>(self.frame, THEATRE)?) {
                let row = row?;
                if row.owner != 0 {
                    continue;
                }
                if row.ordinal >= count || !ids.insert(row.id) || !ordinals.insert(row.ordinal) {
                    return Err("reserved-country land theatres contradict their certificate".into());
                }
            }
            if ids.len() != count as usize {
                return Err(
                    "reserved-country land theatres do not match their certified count".into(),
                );
            }
        }
        let (countries, unresolved) = get(&self.invasions, || {
            let mut unresolved = None;
            let mut theatres = BTreeMap::new();
            for row in each::<tables::LandTheatres>(table::<tables::LandTheatres>(self.frame, THEATRE)?) {
                let row = row?;
                if row.owner == 0 {
                    theatres.insert(row.id, 0);
                    continue;
                }
                match self.slot(row.owner) {
                    Some(owner) => {
                        theatres.insert(row.id, owner);
                    }
                    None => unresolved = Some(self.unlinked("land theatres", row.owner)),
                }
            }
            let mut out: Invasions<'a> = BTreeMap::new();
            for group in each::<tables::OrdersGroups>(table::<tables::OrdersGroups>(self.frame, GROUP)?) {
                let group = group?;
                if group.is_army_group != 0 || group.theatre_ordinal.is_none() {
                    continue;
                }
                let Some(&owner) = theatres.get(&group.theatre) else {
                    continue;
                };
                out.entry(owner).or_default().groups.push(group.handle);
            }
            Ok((out, unresolved))
        })?;
        let orders = if let Some(country) = countries.get(&slot) {
            get(&country.orders, || {
                let mut collected = Vec::new();
                for &handle in &country.groups {
                    let roots = table::<tables::OrdersGroupPlans>(self.frame, 0)?;
                    let first = roots
                        .u32s(u128::from(handle), |row| &row.primary_orders, ROOTS)
                        .map_err(walked::<tables::OrdersGroupPlans>)?
                        .ok_or_else(|| -> Failure {
                            format!("orders group {handle:#x} has no orders group plans row").into()
                        })?;
                    let mut pending: Vec<u32> = first.iter().rev().copied().collect();
                    let mut seen = BTreeSet::new();
                    while let Some(instance) = pending.pop() {
                        if !seen.insert(instance) {
                            continue;
                        }
                        let instances = table::<tables::ArmyOrders>(self.frame, ORDER)?;
                        let key = (handle, instance);
                        let at = (u128::from(handle) << 32) | u128::from(instance);
                        let order = keyed::<tables::ArmyOrders>(&instances, at)?.ok_or_else(
                            || -> Failure {
                                format!(
                                    "order {instance} of group {handle:#x} has no army orders row"
                                )
                                .into()
                            },
                        )?;
                        if order.order_type == 3 {
                            collected.push(Invasion {
                                key,
                                active: order.active,
                                path: instances
                                    .u32s(at, |row| &row.path, PATH)
                                    .map_err(walked::<tables::ArmyOrders>)?
                                    .ok_or_else(|| -> Failure {
                                        format!("order {instance} of group {handle:#x} has no path")
                                            .into()
                                    })?,
                            });
                        }
                        let plans = table::<tables::ArmyOrderPlans>(self.frame, SUCCESSORS)?;
                        let plan = keyed::<tables::ArmyOrderPlans>(&plans, at)?.ok_or_else(
                            || -> Failure {
                                format!(
                                    "order {instance} of group {handle:#x} has no army order plans row"
                                )
                                .into()
                            },
                        )?;
                        pending.extend(plan.successors.iter().rev());
                    }
                }
                Ok(collected)
            })?
            .as_slice()
        } else {
            &[]
        };
        Ok(PartialCollection {
            known: orders,
            unresolved: if slot == 0 { None } else { unresolved.as_ref() },
        })
    }

    #[cfg(test)]
    fn complete_invasion_orders(&self, slot: u32) -> Result<&[Invasion<'a>], Failure> {
        let orders = self.invasion_orders(slot)?;
        if let Some(reason) = orders.unresolved {
            reason.reached();
            return Err(reason.clone());
        }
        Ok(orders.known)
    }

    pub(super) fn invasion_preparation(&self, key: (u64, u32)) -> Result<i64, Failure> {
        const PREPARATION: u64 = mask::<InvasionPreparationRow>(&[
            "progress",
            "country_term",
            "relation_term",
            "relation_bonus",
            "leader_term",
            "leader_applies",
            "days_term",
        ]);
        super::inputs::table(self.frame, replay_data::rows::TableId::InvasionPreparations);
        let rows = table::<tables::InvasionPreparations>(self.frame, PREPARATION)?;
        let row = keyed::<tables::InvasionPreparations>(
            &rows,
            (u128::from(key.0) << 32) | u128::from(key.1),
        )?
        .ok_or_else(|| -> Failure {
            format!(
                "invasion order {} of group {:#x} has no invasion preparations row",
                key.1, key.0
            )
            .into()
        })?;
        let days = self.invasion_prepare_days.map_err(Failure::from)?;
        let scale = i64::from(required_invasion_hours(&row, days)).wrapping_mul(ONE);
        let whole = i64::from((row.progress / ONE) as i32);
        Ok(if scale == 0 {
            0
        } else if whole.wrapping_mul(ONE) < -0x7fff_ffff_ffff_fffe {
            0xffff_ffff
        } else {
            whole.wrapping_mul(10_000_000_000) / scale
        })
    }

    pub(super) fn is_ally_with(&self, slot: u32, other: Option<u32>) -> Result<bool, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::IsAllyWith(slot, other)),
            || self.is_ally_with_unrecorded(slot, other),
        )
    }

    fn is_ally_with_unrecorded(&self, slot: u32, other: Option<u32>) -> Result<bool, Failure> {
        let Some(other) = other else {
            return Ok(false);
        };
        if !self.exists(slot)? || !self.exists(other)? {
            return Ok(false);
        }
        Ok(slot == other
            || (!self.at_war(slot, other)?
                && (self.same_faction(slot, other)?
                    || self.overlord(slot)? == Some(other)
                    || self.overlord(other)? == Some(slot))))
    }

    pub(super) fn variable(&self, store: Store, key: &[u8]) -> Result<Option<i64>, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::Variable(store, key.to_vec())),
            || self.variable_unrecorded(store, key),
        )
    }

    fn variable_unrecorded(&self, store: Store, key: &[u8]) -> Result<Option<i64>, Failure> {
        if let Store::IndustrialOrganisation(reference) = store {
            let details = self.organisation_details(reference)?;
            let mut found = details
                .variables
                .entries
                .iter()
                .filter(|entry| entry.name == key);
            let value = found.next().map(|entry| entry.value_raw);
            if found.next().is_some() {
                return Err(super::refusal::refused(format!(
                    "industrial organisation {reference:#x} records variable {} more than once, and the game's store holds each key once",
                    String::from_utf8_lossy(key)
                )));
            }
            return Ok(value);
        }
        if let Store::Project { owner, id } = store {
            let project = self.project(owner, id)?;
            let runtime = project.runtime.require("runtime of projects")?;
            let variables = runtime.variables.as_ref().ok_or_else(|| {
                format!("project {owner}:{id} variable store capture unavailable")
            })?;
            return Ok(variables
                .entries
                .iter()
                .rev()
                .find(|entry| entry.name == key)
                .map(|entry| entry.value_raw));
        }
        let variables = self.variable_store(store)?;
        if !variables.entries.is_empty() {
            return Ok(variables
                .entries
                .iter()
                .rev()
                .find(|entry| entry.name == key)
                .map(|entry| entry.value_raw));
        }
        if matches!(
            self.frame
                .status(replay_data::rows::TableId::ScriptVariableValues),
            OperandStatus::NotRecorded(_)
        ) {
            return Ok(None);
        }
        const VALUE: u64 = mask::<replay_data::rows::ScriptVariableValueRow>(&["value_raw"]);
        let values = table::<tables::ScriptVariableValues>(self.frame, VALUE)?;
        let names = self.variable_names()?;
        let Some(&var_id) = names.get(key) else {
            return Ok(None);
        };
        let row_key = (u128::from(variables.scope_kind) << 64)
            | (u128::from(variables.scope_id) << 32)
            | u128::from(var_id);
        Ok(keyed::<tables::ScriptVariableValues>(&values, row_key)?.map(|row| row.value_raw))
    }

    fn variable_names(&self) -> Result<&std::collections::HashMap<Vec<u8>, u32>, Failure> {
        const NAME: u64 = mask::<replay_data::rows::ScriptVariableNameRow>(&["name"]);
        super::inputs::table(self.frame, replay_data::rows::TableId::ScriptVariableNames);
        get(&self.variable_names, || {
            let view = self
                .frame
                .identity(replay_data::rows::TableId::ScriptVariableNames);
            let cached = VAR_NAMES.with_borrow(|cache| {
                cache
                    .iter()
                    .find(|(source, _)| view.as_ref().is_some_and(|view| source.same(view)))
                    .map(|(_, names)| Arc::clone(names))
            });
            if let Some(names) = cached {
                return Ok(names);
            }
            let rows = table::<tables::ScriptVariableNames>(self.frame, NAME)?;
            let mut out = std::collections::HashMap::with_capacity(rows.len());
            for row in each::<tables::ScriptVariableNames>(rows) {
                let row = row?;
                out.insert(row.name.clone(), row.id);
            }
            let out = Arc::new(out);
            if let Some(view) = view {
                VAR_NAMES.with_borrow_mut(|cache| {
                    cache.insert(0, (view, Arc::clone(&out)));
                    cache.truncate(2);
                });
            }
            Ok(out)
        })
        .map(Arc::as_ref)
    }

    pub(super) fn variable_seed(&self, store: Store) -> Result<u32, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::VariableSeed(store)),
            || self.variable_seed_unrecorded(store),
        )
    }

    fn variable_seed_unrecorded(&self, store: Store) -> Result<u32, Failure> {
        if let Store::Project { owner, id } = store {
            let project = self.project(owner, id)?;
            let runtime = project.runtime.require("runtime of projects").map_err(|why| {
                Failure::input(
                    replay_data::rows::TableId::Projects,
                    "runtime",
                    Some(store),
                    why,
                )
            })?;
            return runtime
                .variables
                .as_ref()
                .map(|variables| variables.seed)
                .ok_or_else(|| {
                    Failure::input(
                        replay_data::rows::TableId::Projects,
                        "runtime.variables",
                        Some(store),
                        format!("project {owner}:{id} variable store capture unavailable"),
                    )
                });
        }
        let variables = self.variable_store(store)?;
        variables
            .seed
            .require("seed of script variables")
            .map_err(|why| {
                Failure::input(
                    replay_data::rows::TableId::ScriptVariables,
                    "seed",
                    Some(store),
                    why,
                )
            })?
            .ok_or_else(|| -> Failure {
                Failure::input(
                    replay_data::rows::TableId::ScriptVariables,
                    "seed",
                    Some(store),
                    "seed of script variables capture unavailable",
                )
            })
    }

    fn variable_store(&self, store: Store) -> Result<Cow<'a, ScriptVariablesRow>, Failure> {
        const STORE: u64 = mask::<ScriptVariablesRow>(&["entries", "seed"]);
        let rows = table::<tables::ScriptVariables>(self.frame, STORE)?;
        let (kind, id) = match store {
            Store::Global => (0, 0),
            Store::Country(slot) => (
                1,
                self.raw_country(slot).ok_or_else(|| -> Failure {
                    format!("country slot {slot} has no Countries row").into()
                })?,
            ),
            Store::State(id) => (2, id),
            Store::Character(id) => (3, id),
            Store::Project { .. } => unreachable!("project stores are carried by Projects.runtime"),
            Store::IndustrialOrganisation(_) => {
                unreachable!("organisation stores are carried by IndustrialOrganisationDetails")
            }
        };
        keyed::<tables::ScriptVariables>(&rows, (u128::from(kind as u8) << 32) | u128::from(id))?
            .ok_or_else(|| {
                Failure::input(
                    replay_data::rows::TableId::ScriptVariables,
                    "",
                    Some(store),
                    format!(
                        "the script variable store of scope {kind}:{id} has no script variables row"
                    ),
                )
            })
    }

    pub fn script_constants(&self) -> Result<&'a Constants, &'a str> {
        *self.script_constants
    }

    pub(super) fn character_exists(&self, id: u32) -> Result<bool, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::CharacterExists(id)),
            || self.character_exists_unrecorded(id),
        )
    }

    fn character_exists_unrecorded(&self, id: u32) -> Result<bool, Failure> {
        Ok(self.character(id)?.is_some())
    }

    pub(super) fn read_role_traits(&self) {
        let (table, names) = character_scope::FIELDS[3];
        super::inputs::fields(self.frame, table, names);
    }

    pub(super) fn canonical(&self, raw: u32) -> Result<u32, Failure> {
        self.slot(raw).ok_or_else(|| -> Failure {
            format!("country link {raw} has no country links row").into()
        })
    }

    pub(super) fn trait_names(&self) -> Result<character_scope::TraitNames<'_>, Failure> {
        let (units, country_leaders) = (*self.character_traits).map_err(|why| {
            Failure::from(why).context(|why| format!("the trait databases are unread: {why}"))
        })?;
        Ok(character_scope::TraitNames {
            units,
            country_leaders: Ok(country_leaders),
            lexer: self,
        })
    }

    pub(super) fn character_tag(&self, character: u32, tagged: u32) -> Result<bool, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::CharacterTag(character, tagged)),
            || self.character_tag_unrecorded(character, tagged),
        )
    }

    fn character_tag_unrecorded(&self, character: u32, tagged: u32) -> Result<bool, Failure> {
        self.character_tables()
            .tag(character, tagged, |raw| self.canonical(raw))
    }

    pub(super) fn character_skill(&self, character: u32) -> Result<i32, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::CharacterSkill(character)),
            || self.character_skill_unrecorded(character),
        )
    }

    fn character_skill_unrecorded(&self, character: u32) -> Result<i32, Failure> {
        self.character_tables().skill(character)
    }

    pub(super) fn character_retired(&self, character: u32) -> Result<bool, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::CharacterRetired(character)),
            || self.character_retired_unrecorded(character),
        )
    }

    fn character_retired_unrecorded(&self, character: u32) -> Result<bool, Failure> {
        self.character_tables()
            .retired(character, |raw| self.canonical(raw))
    }

    pub(super) fn character_has_trait(&self, character: u32, name: &[u8]) -> Result<bool, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::CharacterHasTrait(character, name.to_vec())),
            || self.character_has_trait_unrecorded(character, name),
        )
    }

    fn character_has_trait_unrecorded(&self, character: u32, name: &[u8]) -> Result<bool, Failure> {
        let names = self.trait_names()?;
        self.character_tables()
            .has_trait(character, name, &names, |raw| self.canonical(raw))
    }

    pub(super) fn state_of_value(&self, raw: i64) -> Result<u32, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::StateOfValue(raw)),
            || self.state_of_value_unrecorded(raw),
        )
    }

    fn state_of_value_unrecorded(&self, raw: i64) -> Result<u32, Failure> {
        let bits = raw as u64;
        let id = if bits & 0xffff_ffff_f000_0000 == 0xffff_ffff_c000_0000 {
            (bits & 0x0fff_ffff) as i64
        } else {
            fixed_round(raw)
        };
        match u32::try_from(id) {
            Ok(id) if id > 0 && self.state_map().rows.contains_key(&id) => Ok(id),
            Ok(id) if id > 0 && self.state_provinces.contains_key(&id) => {
                Err(format!("state {id} is in the content but not recorded").into())
            }
            Ok(id) if id > 0 => match self.table_gap("States") {
                Some(why) => Err(why.context(|why| format!("state {id} is not recorded: {why}"))),
                None => Ok(NULL_STATE),
            },
            _ => Ok(NULL_STATE),
        }
    }

    pub fn unit_leader(
        &self,
        reference: u64,
    ) -> Result<Option<Cow<'a, replay_data::rows::UnitLeaderRow>>, Failure> {
        super::inputs::query_typed_resolved(
            || super::inputs::Key::World(Query::UnitLeader(reference)),
            || self.unit_leader_unrecorded(reference),
        )
    }

    fn unit_leader_unrecorded(
        &self,
        reference: u64,
    ) -> Result<Option<Cow<'a, replay_data::rows::UnitLeaderRow>>, Failure> {
        let rows = leaders(self.frame)?;
        Ok(super::refusal::reference(
            keyed::<tables::UnitLeaders>(&rows, u128::from(reference))?,
            reference != 0,
            || super::refusal::missing_reference(tables::UnitLeaders::ID, None),
        ))
    }

    pub(super) fn leader_visibility<'r>(&self, row: impl Into<Leader<'r>>) -> LeaderVisibility {
        let row = row.into();
        self.leader_visibility_from(row, |id| self.character(id), |token| self.token(token))
    }

    pub fn decoded_leader_visibility<'r>(&self, row: impl Into<Leader<'r>>) -> LeaderVisibility {
        let row = row.into();
        self.leader_visibility_from(
            row,
            |id| {
                let rows = characters(self.frame, crate::identity::CHARACTER_IDENTITY)?;
                Ok(super::refusal::reference(
                    keyed::<tables::Characters>(&rows, u128::from(id))?,
                    id != 0,
                    || {
                        super::refusal::missing_reference(
                            tables::Characters::ID,
                            Some(Store::Character(id)),
                        )
                    },
                ))
            },
            |token| {
                let rows = lexical(self.frame)?;
                Ok(super::refusal::reference(
                    literal(&rows, token)?,
                    token != 0,
                    || super::refusal::missing_reference(tables::LexicalTokens::ID, None),
                ))
            },
        )
    }

    fn leader_visibility_from(
        &self,
        row: Leader<'_>,
        character: impl FnOnce(u32) -> Result<Option<Cow<'a, CharacterRow>>, Failure>,
        literal: impl FnOnce(u32) -> Result<Option<&'a [u8]>, Failure>,
    ) -> LeaderVisibility {
        if row.visibility_choice == 0 && row.template_visibility == 0 {
            return LeaderVisibility::Decided(Eval::True);
        }
        let template = character(row.character_id).and_then(|character| match character {
            Some(character) => Ok(recorded_template(&character)),
            None => Err(format!(
                "unit leader {}'s character {} is not recorded",
                row.id, row.character_id
            )
            .into()),
        });
        self.template_visibility(row, template, literal)
    }

    pub(super) fn template_visibility(
        &self,
        row: Leader<'_>,
        template: Result<Option<RecordedTemplate>, Failure>,
        literal: impl FnOnce(u32) -> Result<Option<&'a [u8]>, Failure>,
    ) -> LeaderVisibility {
        if let Ok(Some(template)) = &template
            && template.generated
        {
            return self.recorded_role_visibility(row);
        }
        match template.and_then(|template| self.template_role(row, template, literal)) {
            Ok(Some((_, visible))) if visible.is_empty() => LeaderVisibility::Unconditioned,
            Ok(Some((index, _))) => LeaderVisibility::Conditioned(index),
            Ok(None) if row.visibility_choice == 0 => LeaderVisibility::Decided(Eval::True),
            Ok(None) => LeaderVisibility::Decided(Eval::unknown(format!(
                "unit leader {} has its own role choice, but its character's template has no unit leader role",
                row.id
            ))),
            Err(reason) => LeaderVisibility::Decided(Eval::unknown(reason)),
        }
    }

    fn recorded_role_visibility(&self, row: Leader<'_>) -> LeaderVisibility {
        use replay_data::rows::UnitLeaderRow;
        let refused = |why: String| {
            LeaderVisibility::Decided(Eval::unknown(format!(
                "unit leader {}'s character template was made by generate_character, and {why}",
                row.id
            )))
        };
        match (row.visible_state, row.visible_origin) {
            (Recorded::Value(UnitLeaderRow::NO_ROLE), _) => LeaderVisibility::Decided(Eval::True),
            (Recorded::Value(UnitLeaderRow::EMPTY_VISIBLE), _) => LeaderVisibility::Unconditioned,
            (Recorded::Value(UnitLeaderRow::VISIBLE), Recorded::Value(Some(origin))) => {
                match self.leader_roles.map_err(str::to_owned).and_then(|roles| roles.placed(&origin.file, origin.line)) {
                    Ok(index) => LeaderVisibility::Conditioned(index),
                    Err(why) => refused(why),
                }
            }
            (Recorded::Value(UnitLeaderRow::VISIBLE), _) => refused("its role's visible has no recorded source".into()),
            (Recorded::Value(state), _) => refused(format!("its role's visibility state {state} is not known")),
            _ => refused("this recording predates the record of its role's visibility".into()),
        }
    }

    fn template_role(
        &self,
        row: Leader<'_>,
        template: Option<RecordedTemplate>,
        literal: impl FnOnce(u32) -> Result<Option<&'a [u8]>, Failure>,
    ) -> Result<Option<(usize, &'a content_reader_script::triggers::Triggers)>, Failure> {
        let template = template.ok_or_else(|| -> Failure {
            format!(
                "unit leader {} has a visibility input, but its character has no template",
                row.id
            )
            .into()
        })?;
        let roles = self.leader_roles.map_err(Failure::from)?;
        let tag = literal(template.tag)?
            .and_then(|literal| std::str::from_utf8(literal).ok())
            .ok_or_else(|| -> Failure {
                format!(
                    "character template tag token {} has no recorded text",
                    template.tag
                )
                .into()
            })?;
        let mut candidates = u32::try_from(template.line)
            .ok()
            .and_then(|line| roles.by_place.get(&(template.file.clone(), line)))
            .into_iter()
            .flatten()
            .copied()
            .filter(|&index| roles.templates[index].tag.eq_ignore_ascii_case(tag));
        let where_ = || {
            format!(
                "{tag} at {}:{}",
                String::from_utf8_lossy(&template.file),
                template.line
            )
        };
        match (candidates.next(), candidates.next()) {
            (Some(index), None) => roles.templates[index]
                .role
                .as_ref()
                .map(|role| role.as_ref().map(|visible| (index, visible)))
                .map_err(|reason| {
                    Failure::from(reason.clone()).context(|reason| {
                        format!("character template {} is refused: {reason}", where_())
                    })
                }),
            (None, _) => Err(format!("no content character template is {}", where_()).into()),
            (Some(_), Some(_)) => {
                Err(format!("several content character templates are {}", where_()).into())
            }
        }
    }

    pub(super) fn role_visible(&self, index: usize) -> RoleVisible<'a> {
        let Ok(roles) = *self.leader_roles else {
            unreachable!("leader role {index} has no content roles");
        };
        if let Some(template) = roles.templates.get(index) {
            let Ok(Some(visible)) = &template.role else {
                unreachable!("leader role {index} is no content role");
            };
            return RoleVisible(RoleBody::Content(visible));
        }
        let placed = roles.placed.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let (file, at) = placed.bodies[index - roles.templates.len()].clone();
        RoleVisible(RoleBody::Placed(file, at))
    }

    pub fn party_leaders(&self, slot: u32) -> Result<Vec<PartyLeaders>, Failure> {
        super::inputs::query_typed_resolved(
            || super::inputs::Key::World(Query::PartyLeaders(slot)),
            || self.party_leaders_unrecorded(slot),
        )
    }

    fn party_leaders_unrecorded(&self, slot: u32) -> Result<Vec<PartyLeaders>, Failure> {
        let party_rows = table::<tables::PoliticalParties>(self.frame, RULING)?;
        let Some(by_party) = self.country_rows(&self.party_rows, &party_rows, 32)? else {
            return self.party_leaders_scanned(slot);
        };
        if let Some((_, raw)) = by_party.first_unlinked() {
            return Err(self.unlinked("political parties", raw));
        }
        let raws = self.slot_links(slot);
        let mut parties = BTreeMap::new();
        for &raw in raws {
            for row in each::<tables::PoliticalParties>(party_rows.within(by_party.of(raw))) {
                let row = row?;
                parties.insert(
                    row.id,
                    PartyLeaders {
                        ruling: row.is_ruling != 0,
                        leaders: Vec::new(),
                    },
                );
            }
        }
        let leader_rows = table::<tables::PartyLeaders>(self.frame, PARTY_LEADER)?;
        let Some(by_leader) = self.country_rows(&self.party_leader_rows, &leader_rows, 64)? else {
            return self.party_leaders_scanned(slot);
        };
        let unlinked = by_leader.first_unlinked();
        for &raw in raws {
            let range = by_leader.of(raw);
            let rows = each::<tables::PartyLeaders>(leader_rows.within(range.clone()));
            for (at, row) in range.zip(rows) {
                if let Some((first, raw)) = unlinked
                    && first < at
                {
                    return Err(self.unlinked("party leaders", raw));
                }
                let row = row?;
                parties
                    .get_mut(&row.party)
                    .ok_or_else(|| -> Failure {
                        format!(
                            "party leaders party {} has no political parties row",
                            row.party
                        )
                        .into()
                    })?
                    .leaders
                    .push(PartyLeader {
                        character: row.character,
                        ordinal: row.ordinal,
                        role: row.role,
                        id: row.leader_id,
                    });
            }
        }
        if let Some((_, raw)) = unlinked {
            return Err(self.unlinked("party leaders", raw));
        }
        Ok(parties.into_values().collect())
    }

    fn party_leaders_scanned(&self, slot: u32) -> Result<Vec<PartyLeaders>, Failure> {
        let mut parties = BTreeMap::new();
        for row in each::<tables::PoliticalParties>(table::<tables::PoliticalParties>(self.frame, RULING)?) {
            let row = row?;
            match self.slot(row.country) {
                Some(country) if country == slot => {
                    parties.insert(
                        row.id,
                        PartyLeaders {
                            ruling: row.is_ruling != 0,
                            leaders: Vec::new(),
                        },
                    );
                }
                Some(_) => {}
                None => return Err(self.unlinked("political parties", row.country)),
            }
        }
        for row in each::<tables::PartyLeaders>(table::<tables::PartyLeaders>(self.frame, PARTY_LEADER)?) {
            let row = row?;
            match self.slot(row.country) {
                Some(country) if country == slot => parties
                    .get_mut(&row.party)
                    .ok_or_else(|| -> Failure {
                        format!(
                            "party leaders party {} has no political parties row",
                            row.party
                        )
                        .into()
                    })?
                    .leaders
                    .push(PartyLeader {
                        character: row.character,
                        ordinal: row.ordinal,
                        role: row.role,
                        id: row.leader_id,
                    }),
                Some(_) => {}
                None => return Err(self.unlinked("party leaders", row.country)),
            }
        }
        Ok(parties.into_values().collect())
    }

    pub fn country_characters(&self, slot: u32) -> Result<Vec<Option<u32>>, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::CountryCharacters(slot)),
            || self.country_characters_unrecorded(slot),
        )
    }

    fn country_characters_unrecorded(&self, slot: u32) -> Result<Vec<Option<u32>>, Failure> {
        let rows = table::<tables::CountryCharacters>(self.frame, COUNTRY_CHARACTER)?;
        let Some(countries) = self.country_rows(&self.character_rows, &rows, 32)? else {
            return self.country_characters_scanned(slot);
        };
        if let Some((_, raw)) = countries.first_unlinked() {
            return Err(self.unlinked("country characters", raw));
        }
        let mut out = Vec::new();
        for &raw in self.slot_links(slot) {
            for row in each::<tables::CountryCharacters>(rows.within(countries.of(raw))) {
                out.push(row?.character);
            }
        }
        Ok(out)
    }

    fn country_characters_scanned(&self, slot: u32) -> Result<Vec<Option<u32>>, Failure> {
        let mut out = Vec::new();
        for row in each::<tables::CountryCharacters>(table::<tables::CountryCharacters>(self.frame, COUNTRY_CHARACTER)?) {
            let row = row?;
            match self.slot(row.country) {
                Some(country) if country == slot => out.push(row.character),
                Some(_) => {}
                None => return Err(self.unlinked("country characters", row.country)),
            }
        }
        Ok(out)
    }

    pub(super) fn leader_has_trait(&self, slot: u32, name: &str) -> Result<bool, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::LeaderHasTrait(slot, name.to_owned())),
            || self.leader_has_trait_unrecorded(slot, name),
        )
    }

    fn leader_has_trait_unrecorded(&self, slot: u32, name: &str) -> Result<bool, Failure> {
        let (_, traits) = self.character_traits.map_err(|why| {
            Failure::from(why)
                .context(|why| format!("the country-leader traits are unavailable: {why}"))
        })?;
        let Some(ordinal) = traits.first_ordinal(name).map_err(|gap| {
            format!(
                "which country-leader trait definition the lookup finds for {name} is unknown: {gap}"
            )
        })?
        else {
            return Ok(false);
        };
        let parties = self.party_leaders(slot)?;
        let mut ruling = parties.iter().filter(|party| party.ruling);
        let Some(party) = ruling.next() else {
            return Ok(false);
        };
        if ruling.next().is_some() {
            return Err(format!("country {slot} has multiple ruling parties").into());
        }
        let Some(leader) = party.leaders.first() else {
            return Ok(false);
        };
        if leader.ordinal != 0 {
            return Err(format!("country {slot}'s party leaders do not start at zero").into());
        }
        let (Some(character), Some(role)) = (leader.character, leader.role) else {
            return Err(
                format!("country {slot}'s first leader has no character role identity").into(),
            );
        };
        if !table::<tables::CountryLeaderRoles>(self.frame, 0)?
            .contains((u128::from(character) << 32) | u128::from(role as u32))
        {
            return Err(
                format!("party leader {character}/{role} has no country leader roles row").into(),
            );
        }
        const HELD: u64 =
            mask::<replay_data::rows::CharacterRoleTraitRow>(&["tokens", "definitions"]);
        let held = table::<tables::CharacterRoleTraits>(self.frame, HELD)?;
        let held = keyed::<tables::CharacterRoleTraits>(
            &held,
            (u128::from(character) << 40)
                | (u128::from(replay_data::rows::CharacterRoleTraitRow::COUNTRY_LEADER) << 32)
                | u128::from(role as u32),
        )?;
        character_scope::holds_country_leader_trait(
            held.as_deref().into_iter(),
            traits,
            name,
            ordinal,
            self.token_of(name.as_bytes())?,
        )
    }

    pub(super) fn civil_war_countries(&self, raw: u32) -> Result<Vec<u32>, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::CivilWarCountries(raw)),
            || self.civil_war_countries_unrecorded(raw),
        )
    }

    fn civil_war_countries_unrecorded(&self, raw: u32) -> Result<Vec<u32>, Failure> {
        const TARGET: u64 = mask::<replay_data::rows::CountryCivilWarRow>(&["target"]);
        let low = u128::from(raw) << 32;
        table::<tables::CountryCivilWars>(self.frame, TARGET)?
            .range(low..=low | 0xffff_ffff)
            .map(|row| row.map(|row| row.target).map_err(walked::<tables::CountryCivilWars>))
            .collect()
    }

    pub(super) fn advisor_token(&self, name: &[u8]) -> Result<bool, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::AdvisorToken(name.to_vec())),
            || self.advisor_token_unrecorded(name),
        )
    }

    fn advisor_token_unrecorded(&self, name: &[u8]) -> Result<bool, Failure> {
        const TOKEN: usize = column::<replay_data::rows::AdvisorTokenRow>("token");
        for token in table::<tables::AdvisorTokens>(self.frame, 0)?.texts(|row| &row.token, TOKEN) {
            if token.map_err(walked::<tables::AdvisorTokens>)? == name {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(super) fn faction_advisors(&self, faction: u64) -> Result<Vec<Vec<u8>>, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::FactionAdvisors(faction)),
            || self.faction_advisors_unrecorded(faction),
        )
    }

    fn faction_advisors_unrecorded(&self, faction: u64) -> Result<Vec<Vec<u8>>, Failure> {
        const TOKEN: usize = column::<replay_data::rows::FactionAdvisorRow>("token");
        let low = u128::from(faction) << 32;
        table::<tables::FactionAdvisors>(self.frame, 0)?
            .span(low, low | 0xffff_ffff)
            .texts(|row| &row.token, TOKEN)
            .map(|token| {
                token
                    .map(<[u8]>::to_vec)
                    .map_err(walked::<tables::FactionAdvisors>)
            })
            .collect()
    }

    pub(super) fn has_civil_war(&self, slot: u32) -> Result<bool, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::HasCivilWar(slot)),
            || self.has_civil_war_unrecorded(slot),
        )
    }

    fn has_civil_war_unrecorded(&self, slot: u32) -> Result<bool, Failure> {
        let raw = self.raw_country(slot).ok_or_else(|| -> Failure {
            format!("country slot {slot} has no Countries row").into()
        })?;
        for other in self.civil_war_countries(raw)? {
            let other = self
                .slot(other)
                .ok_or_else(|| -> Failure { self.unlinked("civil war", other) })?;
            if self.exists(other)? {
                return Ok(true);
            }
        }
        for enemy in self.enemies(slot)? {
            if !self.exists(enemy)? {
                continue;
            }
            let enemy_raw = self.raw_country(enemy).ok_or_else(|| -> Failure {
                format!("country slot {enemy} has no Countries row").into()
            })?;
            for listed in self.civil_war_countries(enemy_raw)? {
                if self.slot(listed) == Some(slot) {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    pub(super) fn enemies(&self, slot: u32) -> Result<Vec<u32>, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::Enemies(slot)),
            || self.enemies_unrecorded(slot),
        )
    }

    fn enemies_unrecorded(&self, slot: u32) -> Result<Vec<u32>, Failure> {
        const ENEMIES: u64 =
            mask::<replay_data::rows::CountryWarRow>(&["enemies_read", "enemies"]);
        let wars = table::<tables::CountryWars>(self.frame, ENEMIES)?;
        for key in wars.keys() {
            if self.slot(key as u32) != Some(slot) {
                continue;
            }
            let row = keyed::<tables::CountryWars>(&wars, key)?
                .ok_or_else(|| walked::<tables::CountryWars>(std::io::Error::other("a listed key holds no row")))?;
            if row.enemies_read != 0 {
                return row
                    .enemies
                    .iter()
                    .map(|&raw| {
                        self.slot(raw)
                            .ok_or_else(|| -> Failure { self.unlinked("country wars", raw) })
                    })
                    .collect();
            }
        }
        let mut out = Vec::new();
        for row in each::<tables::WarRelations>(table::<tables::WarRelations>(self.frame, SPAN)?) {
            let row = row?;
            if !ongoing(row.start_hour, row.end_hour, self.hour) {
                continue;
            }
            let (first, second) = (self.slot(row.first_country), self.slot(row.second_country));
            match (first, second) {
                (Some(a), Some(b)) if a == slot => out.push(b),
                (Some(a), Some(b)) if b == slot => out.push(a),
                (Some(_), Some(_)) => {}
                _ => return Err(self.unlinked("war relations", row.first_country)),
            }
        }
        Ok(out)
    }

    pub(super) fn has_idea(&self, slot: u32, name: &str) -> Result<bool, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::HasIdea(slot, name.to_owned())),
            || self.has_idea_unrecorded(slot, name),
        )
    }

    fn has_idea_unrecorded(&self, slot: u32, name: &str) -> Result<bool, Failure> {
        const IDEA: usize = column::<replay_data::rows::HiredAdvisorRow>("idea");
        let name = name.as_bytes();
        if self.advisor_token(name)? {
            let hired = table::<tables::HiredAdvisors>(self.frame, 0)?;
            for (key, idea) in hired.keys().zip(hired.texts(|row| &row.idea, IDEA)) {
                if self.slot((key >> 32) as u32) == Some(slot)
                    && idea.map_err(walked::<tables::HiredAdvisors>)? == name
                {
                    return Ok(true);
                }
            }
            return match self.faction(slot)? {
                None => Ok(false),
                Some(faction) => Ok(self
                    .faction_advisors(faction)?
                    .iter()
                    .any(|idea| idea == name)),
            };
        }
        self.active_idea(slot, name)
    }

    fn active_idea(&self, slot: u32, name: &[u8]) -> Result<bool, Failure> {
        let rows = table::<tables::ActiveIdeas>(self.frame, 0)?;
        let Some(countries) = self.country_rows(&self.idea_rows, &rows, 32)? else {
            return self.active_idea_scanned(slot, name);
        };
        let mut found = None;
        for &raw in self.slot_links(slot) {
            let range = countries.of(raw);
            let start = range.start;
            for (at, token) in rows.within(range).texts(|row| &row.token, IDEA_TOKEN).enumerate() {
                if token.map_err(walked::<tables::ActiveIdeas>)? == name {
                    found = Some(found.map_or(start + at, |found: usize| found.min(start + at)));
                    break;
                }
            }
        }
        match (found, countries.first_unlinked()) {
            (Some(at), Some((first, raw))) if first < at => Err(self.unlinked("active ideas", raw)),
            (Some(_), _) => Ok(true),
            (None, Some((_, raw))) => Err(self.unlinked("active ideas", raw)),
            (None, None) => Ok(false),
        }
    }

    fn active_idea_scanned(&self, slot: u32, name: &[u8]) -> Result<bool, Failure> {
        let rows = table::<tables::ActiveIdeas>(self.frame, 0)?;
        for row in rows.with_text(|row| &row.token, IDEA_TOKEN) {
            let (row, token) = row.map_err(walked::<tables::ActiveIdeas>)?;
            match self.slot(row.country) {
                Some(country) if country == slot && token == name => return Ok(true),
                Some(_) => {}
                None => return Err(self.unlinked("active ideas", row.country)),
            }
        }
        Ok(false)
    }

    pub(super) fn controlled_state_count(&self, slot: u32) -> Result<i64, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::ControlledStateCount(slot)),
            || self.controlled_state_count_unrecorded(slot),
        )
    }

    fn controlled_state_count_unrecorded(&self, slot: u32) -> Result<i64, Failure> {
        const CONTROLLED: u64 = mask::<CountryCounterInputRow>(&["controlled_state_count"]);
        let raw = self.raw_country(slot).ok_or_else(|| -> Failure {
            format!("country slot {slot} has no Countries row").into()
        })?;
        let rows = table::<tables::CountryCounterInputs>(self.frame, CONTROLLED)?;
        keyed::<tables::CountryCounterInputs>(&rows, u128::from(raw))?
            .map(|row| i64::from(row.controlled_state_count))
            .ok_or_else(|| -> Failure {
                format!("country {raw} has no country counter inputs row").into()
            })
    }

    pub(super) fn opinion(&self, owner: u32, target: Option<u32>) -> Result<i64, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::Opinion(owner, target)),
            || self.opinion_unrecorded(owner, target),
        )
    }

    fn opinion_unrecorded(&self, owner: u32, target: Option<u32>) -> Result<i64, Failure> {
        let Some(target) = target.filter(|&slot| slot != 0) else {
            return Err(refused(
                "opinion: country zero reads reserved relation slot zero, which relation opinions does not record",
            ));
        };
        for slot in [owner, target] {
            self.raw_country(slot).ok_or_else(|| -> Failure {
                format!("country slot {slot} has no Countries row").into()
            })?;
        }
        const OPINION: u64 = mask::<replay_data::rows::RelationOpinionRow>(&["opinion"]);
        let (opinions, unresolved) = get(&self.opinions, || {
            table::<tables::CountryLinks>(self.frame, 0)?;
            let mut out = BTreeMap::new();
            let mut unresolved = None;
            for row in each::<tables::RelationOpinions>(table::<tables::RelationOpinions>(self.frame, OPINION)?) {
                let row = row?;
                match (self.slot(row.owner_country), self.slot(row.target_country)) {
                    (Some(owner), Some(target)) => {
                        out.insert((owner, target), row.opinion);
                    }
                    (None, _) => {
                        unresolved = Some(self.unlinked("relation opinions", row.owner_country))
                    }
                    (_, None) => {
                        unresolved = Some(self.unlinked("relation opinions", row.target_country))
                    }
                }
            }
            Ok((out, unresolved))
        })?;
        Ok(
            i64::from(linked(opinions.get(&(owner, target)).copied(), unresolved)?.unwrap_or(0))
                * ONE,
        )
    }

    fn owned_controlled_state_count(&self, slot: u32) -> Result<i64, Failure> {
        self.raw_country(slot).ok_or_else(|| -> Failure {
            format!("country slot {slot} has no Countries row").into()
        })?;
        let counts = get(&self.owned_controlled_states, || {
            table::<tables::CountryLinks>(self.frame, 0)?;
            provinces(self.frame)?;
            let members = self.state_members()?;
            let mut out: BTreeMap<u32, i32> = BTreeMap::new();
            for row in each::<tables::States>(table::<tables::States>(self.frame, STATE)?) {
                let row = row?;
                if row.owner == 0 || !members.contains_key(&row.id) {
                    continue;
                }
                let owner = self
                    .slot(row.owner)
                    .ok_or_else(|| -> Failure { self.unlinked("states", row.owner) })?;
                if row.controller == 0 {
                    continue;
                }
                let controller = self
                    .slot(row.controller)
                    .ok_or_else(|| -> Failure { self.unlinked("states", row.controller) })?;
                if owner == controller {
                    let count = out.entry(owner).or_default();
                    *count = count.wrapping_add(1);
                }
            }
            Ok(out)
        })?;
        Ok(i64::from(*counts.get(&slot).unwrap_or(&0)))
    }

    pub(super) fn builtin_variable(
        &self,
        store: Store,
        name: &[u8],
    ) -> Result<Option<i64>, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::BuiltinVariable(store, name.to_vec())),
            || self.builtin_variable_unrecorded(store, name),
        )
    }

    fn builtin_variable_unrecorded(
        &self,
        store: Store,
        name: &[u8],
    ) -> Result<Option<i64>, Failure> {
        match (store, name) {
            (Store::Country(slot), b"num_owned_states") => Ok(Some(
                i64::from(self.owned_state_count(slot)?).wrapping_mul(ONE),
            )),
            (Store::Country(slot), b"num_owned_controlled_states") => {
                Ok(Some(self.owned_controlled_state_count(slot)? * ONE))
            }
            (Store::Country(slot), b"num_of_controlled_factories") => {
                let raw = self.raw_country(slot).ok_or_else(|| -> Failure {
                    format!("country slot {slot} has no Countries row").into()
                })?;
                const LEVELS: u64 = mask::<replay_data::rows::CountryFactoriesRow>(&[
                    "civilian_levels",
                    "naval_levels",
                    "military_levels",
                ]);
                let rows = table::<tables::CountryFactories>(self.frame, LEVELS)?;
                let row = keyed::<tables::CountryFactories>(&rows, u128::from(raw))?
                    .ok_or_else(|| -> Failure {
                        format!(
                            "num_of_controlled_factories: country {raw} has no country factories row"
                        )
                        .into()
                    })?;
                let count = row
                    .civilian_levels
                    .wrapping_add(row.naval_levels)
                    .wrapping_add(row.military_levels);
                Ok(Some(i64::from(count) * ONE))
            }
            _ => Ok(None),
        }
    }

    pub(crate) fn with_template_names(
        mut self,
        names: &'a content_reader_layers::content::diagnostics::Once<
            Result<content_reader_localisation::localisation::Localisation, String>,
        >,
        layers: &'a content_reader_layers::ResolvedLayers,
        language: &'a str,
    ) -> Self {
        self.template_names = Some((names, layers, language));
        self
    }

    pub fn has_template(&self, slot: u32, wanted: &[u8]) -> Result<bool, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::HasTemplate(slot, wanted.to_vec())),
            || self.has_template_unrecorded(slot, wanted),
        )
    }

    fn has_template_unrecorded(&self, slot: u32, wanted: &[u8]) -> Result<bool, Failure> {
        const TEMPLATE: u64 =
            mask::<replay_data::rows::DivisionTemplateRow>(&["owner", "name", "name_key"]);
        const ICON: u64 = mask::<replay_data::rows::DivisionTemplateIconRow>(&["country"]);
        let rows = table::<tables::DivisionTemplates>(self.frame, TEMPLATE)?;
        let mut unknown = None;
        for row in each::<tables::DivisionTemplates>(rows) {
            let row = row?;
            let Some(owner) = self.slot(row.owner) else {
                unknown = Some(self.unlinked("division templates", row.owner));
                continue;
            };
            if owner != slot {
                continue;
            }
            if !row.name.is_empty() {
                if row.name == wanted {
                    return Ok(true);
                }
                continue;
            }
            let name = (|| -> Result<String, Failure> {
                let name_key =
                    row.name_key
                        .require("name key of division templates")
                        .map_err(|why| {
                            Failure::input(
                                replay_data::rows::TableId::DivisionTemplates,
                                "name_key",
                                Some(Store::Country(slot)),
                                why,
                            )
                        })?;
                if name_key.is_empty() {
                    return Ok(String::new());
                }
                let (names, layers, language) = self
                    .template_names
                    .ok_or("template display-name localisation is unavailable")?;
                let names = names
                    .get_or_init(|| {
                        content_reader_localisation::localisation::read(layers, language)
                            .map_err(|e| e.to_string())
                    })
                    .as_ref()
                    .map_err(Clone::clone)?;
                let icons = table::<tables::DivisionTemplateIcons>(self.frame, ICON)?;
                let icon = keyed::<tables::DivisionTemplateIcons>(&icons, u128::from(row.template))?
                    .ok_or_else(|| -> Failure {
                        format!(
                            "template {} has no division template icons country",
                            row.template
                        )
                        .into()
                    })?;
                let key =
                    std::str::from_utf8(name_key).map_err(|_| "template name_key is not UTF-8")?;
                let country = match u32::try_from(icon.country).ok().filter(|raw| *raw > 0) {
                    Some(raw) => Some(self.slot(raw).ok_or_else(|| -> Failure {
                        self.unlinked("division template icons", raw)
                    })?),
                    None => None,
                };
                let tag = country
                    .and_then(|country| self.tag(country))
                    .and_then(|tag| std::str::from_utf8(tag).ok());
                let prefixed = tag.map(|tag| format!("{tag}_{key}"));
                let key = prefixed
                    .as_deref()
                    .filter(|key| names.get(key).is_some())
                    .unwrap_or(key);
                names
                    .localise(key.as_bytes())
                    .map_err(|e| Failure::from(e.to_string()))
            })();
            match name {
                Ok(name) if name.as_bytes() == wanted => return Ok(true),
                Ok(_) => {}
                Err(why) => unknown = Some(why),
            }
        }
        unknown.map_or(Ok(false), Err)
    }

    pub fn character(&self, id: u32) -> Result<Option<Cow<'a, CharacterRow>>, Failure> {
        super::inputs::query_typed_resolved(
            || super::inputs::Key::World(Query::Character(id)),
            || self.character_unrecorded(id),
        )
    }

    fn character_unrecorded(&self, id: u32) -> Result<Option<Cow<'a, CharacterRow>>, Failure> {
        let rows = characters(self.frame, tables::Characters::ID.full_mask())?;
        Ok(super::refusal::reference(
            keyed::<tables::Characters>(&rows, u128::from(id))?,
            id != 0,
            || {
                super::refusal::missing_reference(
                    tables::Characters::ID,
                    Some(Store::Character(id)),
                )
            },
        ))
    }

    pub fn appearance(&self, slot: u32) -> Result<Cow<'a, CountryAppearanceRow>, Failure> {
        super::inputs::query_typed_resolved(
            || super::inputs::Key::World(Query::Appearance(slot)),
            || self.appearance_unrecorded(slot),
        )
    }

    fn appearance_unrecorded(&self, slot: u32) -> Result<Cow<'a, CountryAppearanceRow>, Failure> {
        let table = table::<tables::CountryAppearance>(
            self.frame,
            tables::CountryAppearance::ID.full_mask(),
        )?;
        let (rows, unresolved) = get(&self.appearance, || {
            let mut out = BTreeMap::new();
            let mut unresolved = None;
            for key in table.keys() {
                let country = key as u32;
                match self.slot(country) {
                    Some(slot) => {
                        out.insert(slot, key);
                    }
                    None => unresolved = Some(self.unlinked("country appearance", country)),
                }
            }
            Ok((out, unresolved))
        })?;
        let key = linked(rows.get(&slot).copied(), unresolved)?.ok_or_else(|| -> Failure {
            format!("country slot {slot} has no country appearance row").into()
        })?;
        keyed::<tables::CountryAppearance>(&table, key)?.ok_or_else(|| -> Failure {
            format!("country slot {slot} has no country appearance row").into()
        })
    }

    pub(super) fn is_core_of(&self, state: u32, country: Option<u32>) -> Result<Eval, Failure> {
        super::inputs::query_typed_resolved(
            || super::inputs::Key::World(Query::IsCoreOf(state, country)),
            || self.is_core_of_unrecorded(state, country),
        )
    }

    fn is_core_of_unrecorded(&self, state: u32, country: Option<u32>) -> Result<Eval, Failure> {
        const CORES: usize = column::<replay_data::rows::StateCoreRow>("cores");
        let cores = table::<tables::StateCores>(self.frame, 0)?
            .u32s(u128::from(state), |row| &row.cores, CORES)
            .map_err(walked::<tables::StateCores>)?
            .ok_or_else(|| -> Failure { format!("state {state} has no state cores row").into() })?;
        Ok(self.lists(cores, country, "state cores"))
    }

    pub(super) fn is_claimed_by(&self, state: u32, country: Option<u32>) -> Result<Eval, Failure> {
        super::inputs::query_typed_resolved(
            || super::inputs::Key::World(Query::IsClaimedBy(state, country)),
            || self.is_claimed_by_unrecorded(state, country),
        )
    }

    fn is_claimed_by_unrecorded(&self, state: u32, country: Option<u32>) -> Result<Eval, Failure> {
        const CLAIMS: u64 = mask::<replay_data::rows::StateCoreRow>(&["claims"]);
        let rows = table::<tables::StateCores>(self.frame, CLAIMS)?;
        let row = keyed::<tables::StateCores>(&rows, u128::from(state))?
            .ok_or_else(|| -> Failure { format!("state {state} has no state cores row").into() })?;
        let Recorded::Value(claims) = &row.claims else {
            return Ok(Eval::unknown(
                "claims of state cores is not recorded".to_owned(),
            ));
        };
        Ok(self.lists(claims, country, "claims of state cores"))
    }

    fn lists(&self, links: &[u32], country: Option<u32>, table: &str) -> Eval {
        let Some(country) = country else {
            return Eval::False;
        };
        let mut result = Eval::False;
        for &link in links {
            result = result.or(|| match self.slot(link) {
                Some(slot) => Eval::from_bool(slot == country),
                None => Eval::unknown(self.unlinked(table, link)),
            });
        }
        result
    }

    pub fn raw_country(&self, slot: u32) -> Option<u32> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::RawCountry(slot)),
            || self.raw_country_unrecorded(slot),
        )
    }

    fn raw_country_unrecorded(&self, slot: u32) -> Option<u32> {
        super::refusal::reference(self.country(slot).map(|(raw, _)| raw), slot != 0, || {
            self.table_gap("Countries").unwrap_or_else(|| {
                super::refusal::missing_reference(tables::Countries::ID, Some(Store::Country(slot)))
            })
        })
    }

    pub(super) fn nukes(&self, slot: u32) -> Result<i64, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::Nukes(slot)),
            || self.nukes_unrecorded(slot),
        )
    }

    fn nukes_unrecorded(&self, slot: u32) -> Result<i64, Failure> {
        const AMOUNT: u64 = mask::<replay_data::rows::NukeRow>(&["amount_raw"]);
        let nukes = table::<tables::Nukes>(self.frame, AMOUNT)?;
        let key = nukes
            .keys()
            .find(|&key| key as u32 == 0 && self.slot((key >> 32) as u32) == Some(slot))
            .ok_or_else(|| -> Failure { format!("country slot {slot} has no Nukes row").into() })?;
        let row = keyed::<tables::Nukes>(&nukes, key)?
            .ok_or_else(|| -> Failure { format!("country slot {slot} has no Nukes row").into() })?;
        let raw = row.amount_raw;
        Ok(
            if (raw as u64).wrapping_add(0x7fff_ffff_ffff_fffe) < 0xffff_ffff_ffff_fffd {
                raw.wrapping_mul(100_000) / 10_000_000_000_000
            } else {
                0xa7c5
            },
        )
    }

    pub(super) fn country_scientists(&self, slot: u32) -> Result<Vec<u32>, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::CountryScientists(slot)),
            || self.country_scientists_unrecorded(slot),
        )
    }

    fn country_scientists_unrecorded(&self, slot: u32) -> Result<Vec<u32>, Failure> {
        let mut rostered = Vec::new();
        for row in each::<tables::Scientists>(table::<tables::Scientists>(self.frame, SCIENTIST)?) {
            let row = row?;
            match self.slot(row.country) {
                Some(country) if country == slot => rostered.push((row.ordinal, row.character)),
                Some(_) => {}
                None => return Err(self.unlinked("scientists", row.country)),
            }
        }
        rostered.sort_unstable();
        Ok(rostered.into_iter().map(|(_, character)| character).collect())
    }

    fn scientist(&self, character: u32) -> Result<Option<Cow<'a, replay_data::rows::ScientistRow>>, Failure> {
        let scientists = table::<tables::Scientists>(self.frame, SCIENTIST)?;
        if let Some(row) = keyed::<tables::Scientists>(&scientists, u128::from(character))? {
            return Ok(Some(row));
        }
        for row in each::<tables::CountryCharacters>(table::<tables::CountryCharacters>(
            self.frame,
            COUNTRY_CHARACTER,
        )?) {
            if row?.character == Some(character) {
                return Ok(None);
            }
        }
        Err(format!(
            "character {character} is in no recorded roster, so whether it holds a scientist role is not recorded"
        )
        .into())
    }

    pub(super) fn scientist_active(&self, character: u32) -> Result<Option<u8>, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::ScientistActive(character)),
            || self.scientist_active_unrecorded(character),
        )
    }

    fn scientist_active_unrecorded(&self, character: u32) -> Result<Option<u8>, Failure> {
        let Some(row) = self.scientist(character)? else {
            return Ok(None);
        };
        super::refusal::optional(
            &row.active,
            tables::Scientists::ID,
            "active",
            Some(Store::Character(character)),
        )
        .map(|active| Some(*active))
    }

    pub(super) fn scientist_level(
        &self,
        character: u32,
        specialization: u32,
    ) -> Result<Option<i32>, Failure> {
        super::inputs::query_resolved(
            || super::inputs::Key::World(Query::ScientistLevel(character, specialization)),
            || self.scientist_level_unrecorded(character, specialization),
        )
    }

    fn scientist_level_unrecorded(
        &self,
        character: u32,
        specialization: u32,
    ) -> Result<Option<i32>, Failure> {
        let Some(row) = self.scientist(character)? else {
            return Ok(None);
        };
        let skills = table::<tables::ScientistSkills>(self.frame, SCIENTIST_SKILL)?;
        let first = u128::from(character) << 32;
        let mut held = 0_i64;
        let mut level = None;
        for skill in skills.range(first..=first | u128::from(u32::MAX)) {
            let skill = skill.map_err(walked::<tables::ScientistSkills>)?;
            held += 1;
            if level.is_none() && skill.token == specialization {
                level = Some(skill.level);
            }
        }
        if held != i64::from(row.skill_count) {
            return Err(format!(
                "scientist {character} has {held} recorded specialization levels of its {}",
                row.skill_count
            )
            .into());
        }
        Ok(Some(level.unwrap_or(0)))
    }
}

#[derive(Default)]
struct Techs {
    researched: BTreeSet<(u32, Vec<u8>)>,
    unnamed: BTreeSet<u32>,
    unresolved: bool,
}

pub fn required_invasion_hours(row: &InvasionPreparationRow, prepare_days: i32) -> i32 {
    let leader = if row.leader_applies != 0 {
        row.leader_term.unwrap_or(0)
    } else {
        0
    };
    let factor = ONE
        .wrapping_add(row.country_term)
        .wrapping_add(row.relation_term.unwrap_or(0))
        .wrapping_add(leader)
        .wrapping_add(row.relation_bonus.unwrap_or(0));
    let days = i64::from(prepare_days)
        .wrapping_mul(ONE)
        .wrapping_add(row.days_term);
    ((factor.wrapping_mul(days) / 10_000_000_000) as i32).wrapping_mul(24)
}

pub fn fixed_round(value: i64) -> i64 {
    if value < 0 {
        value.wrapping_sub(50_000) / ONE
    } else {
        value.wrapping_add(50_000) / ONE
    }
}

impl Links for World<'_> {
    fn slot(&self, raw: u32) -> Option<u32> {
        World::slot(self, raw)
    }

    fn token_of(&self, name: &[u8]) -> Result<Option<u32>, String> {
        World::token_of(self, name).map_err(|why| why.to_string())
    }

    fn token_input(&self, name: &[u8]) -> Result<Option<u32>, Failure> {
        World::token_of(self, name)
    }
}

fn get<T>(cell: &Index<T>, build: impl FnOnce() -> Result<T, Failure>) -> Result<&T, Failure> {
    cell.get_or_init(|| {
        let _cpu = replay_frame::cpu::scope_phase("triggers:index");
        super::refusal::Resolved::new(|| super::refusal::deferred(build))
    })
        .value()
        .as_ref()
        .map_err(|failure| {
            failure.reached();
            failure.clone()
        })
}

#[path = "world/dependence.rs"]
mod dependence;

#[cfg(test)]
#[path = "world/lookup_tests.rs"]
mod lookup_tests;

#[cfg(test)]
#[path = "world/zero_countries_tests.rs"]
mod zero_countries_tests;

#[cfg(test)]
#[path = "world/literal_tests.rs"]
mod literal_tests;
