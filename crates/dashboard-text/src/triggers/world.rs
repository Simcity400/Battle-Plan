//! The recorded facts triggers read over one hour, each index built on
//! first use from the table the game itself reads, joined through
//! `CountryLinks`.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex, OnceLock},
};

use installed_content::{
    characters::LeaderTemplate, leader_traits::LeaderTraits, script_constants::Constants,
    tag_aliases::TagAliases, trigger_keys::TriggerNames, unit_leader::Traits,
};
use replay::{
    Hour,
    frame::{DecodedFrame, TableView, ViewIdentity},
    rows::{
        ArmyOrderPlanRow, ArmyOrderRow, CharacterRow, CountryAppearanceRow, CountryCounterInputRow,
        CountryLinkRow, CountryRow, InvasionPreparationRow, ProvinceRow, Recorded,
        ScriptVariablesRow, StateRow, TableSchema, UnitLeaderRow, tables,
    },
};

use super::{Eval, Scope, ongoing, refused, variables::Store};
use crate::{
    character_scope,
    scope::RecordedScopes,
    state_resources::{Links, ONE, StateResources},
};

type Index<T> = OnceLock<Result<T, String>>;

type OperandLoader<'a, T> = (
    &'a dyn PreparedContent,
    &'a DecodedFrame,
    fn(&'a dyn PreparedContent, &'a DecodedFrame) -> T,
);

/// An operand retains its authored result and runs its reader only on first use.
pub struct PreparedOperand<'a, T> {
    value: OnceLock<T>,
    load: Option<OperandLoader<'a, T>>,
}
impl<'a, T> PreparedOperand<'a, T> {
    fn lazy(
        content: &'a dyn PreparedContent,
        frame: &'a DecodedFrame,
        load: fn(&'a dyn PreparedContent, &'a DecodedFrame) -> T,
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
    fn authored_states(&self) -> AuthoredStates<'_>;
    fn continents(&self) -> Result<&[String], &str>;
    fn ideologies(&self) -> Result<&installed_content::ideologies::Ideologies, &str>;
    fn triggers(&self) -> Result<&TriggerNames, &str>;
    fn character_traits(&self) -> Result<(&Traits, &LeaderTraits), &str>;
    fn resources(&self) -> Result<(&installed_content::buildings::Buildings, [i64; 2]), &str>;
    fn leader_roles(&self) -> Result<&LeaderRoles, &str>;
    fn constants(&self) -> Result<&Constants, &str>;
    fn invasion_days(&self) -> Result<i32, &str>;
    fn geography(&self) -> GeographyInputs<'_>;
}
/// Lexer tokens by their literal under the lexer's fold; `Err` for a fold two
/// recorded literals share.
type Folds = BTreeMap<Vec<u8>, Result<u32, String>>;
/// The latest folds by the LexicalTokens allocation they were read from. The
/// table is static, so every hour's World shares one until it is read again;
/// an identity matches no other allocation.
static FOLDS: Mutex<Vec<(ViewIdentity, Arc<Folds>)>> = Mutex::new(Vec::new());
/// An index of rows joined through `CountryLinks`, and why some row's link
/// did not resolve: an answer the index does not hold is then unknown, since
/// the unresolved row may be the one asked about.
type Linked<T> = (T, Option<String>);
/// Each canonical country's regular roster groups and lazily reached orders.
type Invasions<'a> = BTreeMap<u32, CountryInvasions<'a>>;

#[derive(Default)]
struct CountryInvasions<'a> {
    groups: Vec<u64>,
    orders: Index<Vec<Invasion<'a>>>,
}

/// One naval invasion order: its `(owner group, instance)` key,
/// active byte and path.
#[derive(Clone, Copy, Debug)]
pub(super) struct Invasion<'a> {
    pub key: (u64, u32),
    pub active: u8,
    pub path: &'a [u32],
}
/// (canonical slot, literal) pairs.
type CountryNames<'a> = BTreeSet<(u32, &'a [u8])>;

/// `found`, else `Err` when an unresolved row may hold the answer.
fn linked<T>(found: Option<T>, unresolved: &Option<String>) -> Result<Option<T>, String> {
    match (found, unresolved) {
        (Some(value), _) => Ok(Some(value)),
        (None, Some(reason)) => Err(reason.clone()),
        (None, None) => Ok(None),
    }
}
/// A table's rows keyed by raw country first, by country: each country's
/// rows are one range when the rows ascend by country, as a decoded table's
/// keys do. A query of one country then reads that country's rows, and
/// whether any row's country does not resolve, instead of every row.
struct CountryRows {
    /// Each country's first row and the row past its last, by country.
    runs: Vec<(u32, usize, usize)>,
    /// The least and the greatest country with no `CountryLinks` row: the
    /// first and the last row in table order whose country does not resolve.
    unlinked: Option<(u32, u32)>,
}

impl CountryRows {
    /// `rows` by `country`; None when the countries do not ascend.
    fn new<R>(
        rows: &[R],
        country: impl Fn(&R) -> u32,
        linked: impl Fn(u32) -> bool,
    ) -> Option<Self> {
        let mut runs: Vec<(u32, usize, usize)> = Vec::new();
        for (at, row) in rows.iter().enumerate() {
            let country = country(row);
            match runs.last_mut() {
                Some(run) if run.0 == country => run.2 = at + 1,
                Some(run) if run.0 > country => return None,
                _ => runs.push((country, at, at + 1)),
            }
        }
        let mut unlinked = runs
            .iter()
            .map(|run| run.0)
            .filter(|&country| !linked(country));
        let first = unlinked.next();
        let unlinked = first.map(|first| (first, unlinked.last().unwrap_or(first)));
        Some(Self { runs, unlinked })
    }

    /// The rows of `country`.
    fn of(&self, country: u32) -> std::ops::Range<usize> {
        self.runs
            .binary_search_by_key(&country, |run| run.0)
            .map_or(0..0, |at| self.runs[at].1..self.runs[at].2)
    }

    /// The first row whose country does not resolve, and its country.
    fn first_unlinked(&self) -> Option<(usize, u32)> {
        let (country, _) = self.unlinked?;
        Some((self.of(country).start, country))
    }
}

/// Each (owner, native definition name)'s entries in container order:
/// their raw scope-country references and state IDs.
type Modifiers<'a> = BTreeMap<(u32, &'a [u8]), Vec<(u32, i32)>>;

/// Native ideology IDs/tokens and each country's party membership.
struct PoliticalParties<'a> {
    ruling: BTreeMap<u32, u32>,
    unresolved: Option<String>,
    ids: BTreeMap<u32, &'a [u8]>,
    tokens: BTreeMap<Vec<u8>, u32>,
    members: BTreeSet<(u32, u32)>,
}

/// The core tables' rows: CountryLinks, Countries, Provinces and States.
type CoreRows<'f> = (
    &'f [CountryLinkRow],
    &'f [CountryRow],
    &'f [ProvinceRow],
    &'f [StateRow],
);

/// The core tables' rows and why each, in their order, is not complete this
/// hour: a truncated table's rows are exact; a row it lacks is unknown.
fn core_tables(frame: &DecodedFrame) -> (CoreRows<'_>, Vec<(&'static str, String)>) {
    fn rows<'f, T: TableSchema>(
        frame: &'f DecodedFrame,
        name: &'static str,
        gaps: &mut Vec<(&'static str, String)>,
    ) -> &'f [T::Row] {
        match &*frame.tables[T::ID.index()] {
            TableView::Complete(rows) => rows.rows::<T>().unwrap_or_default(),
            TableView::Truncated { rows, missing } => {
                gaps.push((
                    name,
                    format!("{name} is truncated ({missing} rows missing)"),
                ));
                rows.rows::<T>().unwrap_or_default()
            }
            TableView::Unavailable { reason } => {
                gaps.push((name, format!("{name} is unavailable: {reason}")));
                &[]
            }
        }
    }
    let mut gaps = Vec::new();
    let core = (
        rows::<tables::CountryLinks>(frame, "CountryLinks", &mut gaps),
        rows::<tables::Countries>(frame, "Countries", &mut gaps),
        rows::<tables::Provinces>(frame, "Provinces", &mut gaps),
        rows::<tables::States>(frame, "States", &mut gaps),
    );
    (core, gaps)
}

/// [`World::core_gap`] of the hour, without building its indexes.
pub fn core_gap(frame: &DecodedFrame) -> Option<String> {
    core_tables(frame).1.into_iter().next().map(|(_, why)| why)
}

/// The localisation, content stack and language division template names are read with.
type TemplateNames<'a> = (
    &'a OnceLock<Result<installed_content::localisation::Localisation, String>>,
    &'a installed_content::ResolvedLayers,
    &'a str,
);

/// Borrowed preparation inputs keep geography lazy without owning scope lifetimes.
#[derive(Clone, Copy)]
pub struct GeographyInputs<'a> {
    pub neighbors: &'a OnceLock<Result<BTreeMap<u32, Vec<u32>>, String>>,
    pub terrain: &'a OnceLock<Option<installed_content::movement::MovementMap>>,
    pub layers: &'a installed_content::ResolvedLayers,
    pub state_provinces: &'a BTreeMap<u32, Vec<u32>>,
    pub state_error: Option<&'a str>,
    pub defines: &'a installed_content::defines::Defines,
    pub terrain_definitions: Option<&'a installed_content::terrain::TerrainDefinitions>,
}

impl<'a> GeographyInputs<'a> {
    pub(crate) fn neighbors(self) -> Result<&'a BTreeMap<u32, Vec<u32>>, &'a str> {
        self.neighbors
            .get_or_init(|| match self.state_error {
                Some(why) => Err(why.to_owned()),
                None => installed_content::province_graph::ProvinceGraph::read(self.layers)
                    .and_then(|(graph, _)| graph.neighbor_states(self.state_provinces))
                    .map_err(|error| error.to_string()),
            })
            .as_ref()
            .map_err(String::as_str)
    }

    pub(crate) fn terrain(self) -> Option<&'a installed_content::movement::MovementMap> {
        self.terrain
            .get_or_init(|| {
                self.terrain_definitions.and_then(|terrain| {
                    installed_content::movement::MovementMap::read_with_terrain(
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

/// One hour's trigger inputs, each built on first use.
pub struct World<'a> {
    frame: &'a DecodedFrame,
    prepared_content: Option<&'a dyn PreparedContent>,
    hour: Hour,
    /// Which of the tables the indexes below are built from is not complete
    /// this hour, and why ([`World::read`]); the triggers that read them are
    /// then unknown.
    core_gaps: Vec<(&'static str, String)>,
    /// Raw country reference to canonical slot.
    links: BTreeMap<u32, u32>,
    tags: BTreeMap<u32, &'a [u8]>,
    /// Canonical slot to the country's own raw id, as script flags key it.
    raw_ids: BTreeMap<u32, u32>,
    /// The hour's Provinces and States rows, and each by id, built when
    /// first read.
    province_rows: &'a [ProvinceRow],
    state_rows: &'a [StateRow],
    provinces: OnceLock<BTreeMap<u32, &'a ProvinceRow>>,
    states: OnceLock<BTreeMap<u32, &'a StateRow>>,
    /// Content: every state's provinces in its own order, and impassability.
    state_provinces: PreparedOperand<'a, &'a BTreeMap<u32, Vec<u32>>>,
    /// Why authored state templates are unread, if they are.
    pub(super) state_content: PreparedOperand<'a, Result<(), &'a str>>,
    /// Independently read authored continent names.
    pub(super) continents: PreparedOperand<'a, Result<&'a [String], &'a str>>,
    /// The independently read subtype declarations, or why they are unread.
    pub(super) ideologies:
        PreparedOperand<'a, Result<&'a installed_content::ideologies::Ideologies, &'a str>>,
    pub(super) impassable: PreparedOperand<'a, &'a BTreeSet<u32>>,
    /// Content: each state's neighbour states, or why they are unknown.
    neighbor_states: Result<&'a BTreeMap<u32, Vec<u32>>, &'a str>,
    /// Content: each province's static terrain flags; `None` when the
    /// movement map could not be read.
    terrain: Option<&'a installed_content::movement::MovementMap>,
    lazy_geography: Option<GeographyInputs<'a>>,
    pub(super) dlcs: &'a BTreeSet<String>,
    /// Content: the scripted triggers and the count trigger names.
    pub(super) triggers: PreparedOperand<'a, Result<&'a TriggerNames, &'a str>>,
    /// Content: the unit-leader and country-leader trait databases a
    /// character's `has_trait` reads, or why they are unread.
    pub(super) character_traits:
        PreparedOperand<'a, Result<(&'a Traits, &'a LeaderTraits), &'a str>>,
    /// A character scope's recorded operands (`crate::character_scope`).
    character_tables: OnceLock<character_scope::Tables<'a>>,
    /// Content: the tag alias database, which the hour's scopes consult.
    aliases: &'a Arc<TagAliases>,
    /// The hour's scope facts ([`RecordedScopes`]).
    recorded: OnceLock<RecordedScopes>,
    /// The lexer folds ([`FOLDS`]).
    names: Index<Arc<Folds>>,
    literals: Index<BTreeMap<u32, &'a [u8]>>,
    factions: Index<Linked<BTreeMap<u32, u64>>>,
    wars: Index<Linked<BTreeSet<(u32, u32)>>>,
    overlords: Index<Linked<BTreeMap<u32, u32>>>,
    focuses: Index<Linked<CountryNames<'a>>>,
    /// Each canonical slot's raw country references, ascending.
    slot_links: OnceLock<BTreeMap<u32, Vec<u32>>>,
    /// The country-keyed tables' rows by country, where they ascend.
    focus_rows: OnceLock<Option<CountryRows>>,
    party_rows: OnceLock<Option<CountryRows>>,
    party_leader_rows: OnceLock<Option<CountryRows>>,
    character_rows: OnceLock<Option<CountryRows>>,
    idea_rows: OnceLock<Option<CountryRows>>,
    techs: Index<Techs>,
    projects: Index<Linked<BTreeSet<(u32, u32)>>>,
    political_parties: Index<PoliticalParties<'a>>,
    politics_ruling: Index<Linked<BTreeMap<u32, Option<u32>>>>,
    politics_elections: Index<Linked<BTreeMap<u32, Recorded<bool>>>>,
    /// With each modifier name a row of an unlinked country holds, and
    /// that country.
    country_modifiers: Index<(Modifiers<'a>, BTreeMap<&'a [u8], u32>)>,
    state_modifiers: Index<Modifiers<'a>>,
    /// Each state's recorded provinces, ascending.
    state_members: Index<BTreeMap<u32, Vec<u32>>>,
    /// Each raw country's stored counts and native ordered rosters.
    counter_inputs: Index<BTreeMap<u32, &'a CountryCounterInputRow>>,
    /// Nonempty owned states whose controller is their owner, by canonical slot.
    owned_controlled_states: Index<BTreeMap<u32, i32>>,
    /// Nonzero cached opinions by directed canonical country pair.
    opinions: Index<Linked<BTreeMap<(u32, u32), i32>>>,
    /// Province and state building levels by (location, type).
    building_levels: [Index<BTreeMap<(u32, u32), i16>>; 2],
    /// The state resource operands (`crate::state_resources`), shared with
    /// the tactic triggers.
    resource_amounts: PreparedOperand<'a, Result<StateResources<'a>, &'a str>>,
    /// Each canonical country's naval invasion orders (type 3) reachable from
    /// its theatres' regular groups: the order's active byte
    /// and path.
    invasions: Index<Linked<Invasions<'a>>>,
    /// Order payloads and primary roots, indexed only when a country's
    /// native traversal reaches them. Their refusals stay with that country.
    army_orders: Index<BTreeMap<(u64, u32), &'a ArmyOrderRow>>,
    army_order_plans: Index<BTreeMap<(u64, u32), &'a ArmyOrderPlanRow>>,
    group_order_roots: Index<BTreeMap<u64, &'a [u32]>>,
    /// Content: the script constants a `constant:` operand reads, or why
    /// they are unread.
    script_constants: PreparedOperand<'a, Result<&'a Constants, &'a str>>,
    /// Naval invasion orders' preparation operands by `(owner group, instance)`.
    invasion_preparations: Index<BTreeMap<(u64, u32), &'a InvasionPreparationRow>>,
    /// Content: `NNavy.NAVAL_INVASION_PREPARE_DAYS` as the game reads it.
    invasion_prepare_days: PreparedOperand<'a, Result<i32, &'a str>>,
    /// Unit leaders by their reference (`id << 32 | reference type`, the
    /// raid target's handle).
    leaders: Index<BTreeMap<u64, &'a UnitLeaderRow>>,
    /// Characters by their id.
    characters: Index<BTreeMap<u32, &'a CharacterRow>>,
    /// Content: the character templates, or why unread.
    leader_roles: PreparedOperand<'a, Result<&'a LeaderRoles, &'a str>>,
    /// Each canonical country's appearance row (`CountryAppearance`).
    appearance: Index<Linked<BTreeMap<u32, &'a CountryAppearanceRow>>>,
    /// Each state's cores, raw country references.
    cores: Index<BTreeMap<u32, &'a [u32]>>,
    /// Each state's claims, raw country references; `None` when the
    /// recording holds no claims.
    claims: Index<BTreeMap<u32, Option<&'a [u32]>>>,
    template_names: Option<TemplateNames<'a>>,
}

/// The content facts triggers read.
pub struct ContentFacts<'a> {
    pub state_provinces: &'a BTreeMap<u32, Vec<u32>>,
    /// Why authored state templates are unread, if they are.
    pub state_content: Result<(), &'a str>,
    /// Independently read authored continent names.
    pub continents: Result<&'a [String], &'a str>,
    /// The independently read subtype declarations, or why they are unread.
    pub ideologies: Result<&'a installed_content::ideologies::Ideologies, &'a str>,
    pub impassable: &'a BTreeSet<u32>,
    /// Each state's authored neighbour states, or
    /// why they cannot be derived.
    pub neighbor_states: Result<&'a BTreeMap<u32, Vec<u32>>, &'a str>,
    /// The content movement map's static province terrain, whose
    /// flags hold the coastal bit.
    pub terrain: Option<&'a installed_content::movement::MovementMap>,
    pub dlcs: &'a BTreeSet<String>,
    /// The scripted triggers and the building, ideology and resource names
    /// trigger loading finds after the registry.
    pub triggers: Result<&'a TriggerNames, &'a str>,
    /// The unit-leader and country-leader trait databases a character's
    /// `has_trait` looks its operand up in, or why they are unread.
    pub character_traits: Result<(&'a Traits, &'a LeaderTraits), &'a str>,
    /// The tag alias database, which a tag consults after the
    /// country tags.
    pub tag_aliases: &'a Arc<TagAliases>,
    /// Building types, for their `local_resources_<resource>` grants.
    pub buildings: Result<&'a installed_content::buildings::Buildings, &'a str>,
    /// `NBuildings` `INFRASTRUCTURE_RESOURCE_BONUS` and
    /// `SUPPLY_ROUTE_RESOURCE_BONUS`, fixed5, or why they are unread.
    pub resource_bonuses: Result<[i64; 2], &'a str>,
    /// The character templates, or why they are unread.
    pub leader_roles: Result<&'a LeaderRoles, &'a str>,
    /// `NNavy.NAVAL_INVASION_PREPARE_DAYS` (an `i32` define), or why it is
    /// unread.
    pub invasion_prepare_days: Result<i32, &'a str>,
    /// The script constants database (`common/script_constants`), or why it
    /// is unread.
    pub script_constants: Result<&'a Constants, &'a str>,
}

impl<'a> ContentFacts<'a> {
    /// DLCs and trigger names, with other content inputs unavailable.
    pub fn content_only(
        dlcs: &'a BTreeSet<String>,
        triggers: &'a TriggerNames,
        aliases: &'a Arc<TagAliases>,
    ) -> Self {
        static STATES: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
        static IMPASSABLE: BTreeSet<u32> = BTreeSet::new();
        static BUILDINGS: OnceLock<installed_content::buildings::Buildings> = OnceLock::new();
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

/// The character templates the game builds
/// ([`installed_content::characters::leader_templates`]), by the file and line each
/// stores.
#[derive(Debug)]
pub struct LeaderRoles {
    pub(super) templates: Vec<LeaderTemplate>,
    pub(super) by_place: BTreeMap<(Vec<u8>, u32), Vec<usize>>,
    /// Each template's tag folded as the lexer folds a token
    /// as the database indexes it by token.
    names: BTreeSet<Vec<u8>>,
}

impl LeaderRoles {
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
        }
    }
}

/// The template a recorded character was built from, as
/// the stored values the recorder reads from it (the Characters template
/// columns).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordedTemplate {
    /// The tag's lexer token.
    pub tag: u32,
    /// The file its reading began in.
    pub file: Vec<u8>,
    /// The reader's line as its reading began.
    pub line: i32,
    /// Set for `generate_character` templates.
    pub generated: bool,
}

/// The recorded template of a character, `None` without one.
fn recorded_template(character: &CharacterRow) -> Option<RecordedTemplate> {
    (character.template_present != 0).then(|| RecordedTemplate {
        tag: character.template_tag,
        file: character.template_file.clone(),
        line: character.template_line,
        generated: character.template_generated != 0,
    })
}

/// What decides a unit leader's visibility before its owner's scope is
/// entered ([`super::Evaluator::leader_visible`]), read from its visibility
/// inputs, its character's recorded template and the content alone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LeaderVisibility {
    /// Decided without the owner's scope.
    Decided(Eval),
    /// A role without `visible`: its empty conjunction holds in the owner's scope.
    Unconditioned,
    /// The role of the content template at this index of [`LeaderRoles`],
    /// whose `visible` conjunction decides in the owner's scope.
    Conditioned(usize),
}

/// The null state scope: a child event scope whose
/// state is 0 and every other object empty. A trigger
/// accepting a nonzero scope mask is incompatible with it
/// so trigger checking
/// answers false without evaluating it; a mask-0 trigger (the logic
/// triggers, `always`, `has_dlc`, `has_global_flag` and the collection
/// triggers) evaluates as anywhere. A scripted trigger (mask 0) evaluates,
/// but its template's own check (mask 0x3FFE) fails. Real state ids start
/// at 1. A character target the scope country lacks leaves the same empty
/// scope by setting character with clearing.
pub const NULL_STATE: u32 = 0;

/// ScriptFlags' character scope, keyed by the
/// character id.
pub(super) const CHARACTER_FLAGS: u8 = 3;

/// One political party's country leaders,
/// in stored order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PartyLeaders {
    /// The ruling party (`PoliticalParties.is_ruling`).
    pub ruling: bool,
    pub leaders: Vec<PartyLeader>,
}

/// One country leader of a party.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PartyLeader {
    /// Its character's id.
    pub character: Option<u32>,
    /// Native vector position and signed character role-map key (PartyLeaders).
    pub ordinal: u32,
    pub role: Option<i32>,
    /// The role's `id`.
    pub id: i32,
}

pub(super) fn table<T: TableSchema>(frame: &DecodedFrame) -> Result<&[T::Row], String> {
    frame.complete::<T>().map_err(|error| error.to_string())
}

/// A recorded query of the hour's world ([`super::inputs`]): its method and arguments.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Query {
    TableStatus(replay::rows::TableId),
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
}

impl World<'_> {
    /// The digest `query` reads in this hour's world.
    pub fn reread(&self, query: &Query) -> u64 {
        match query {
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
                super::inputs::digest_debug(&self.unit_status_unrecorded(*reference))
            }
            Query::RaidWingName(reference) => {
                super::inputs::digest(&self.raid_wing_name_unrecorded(*reference))
            }
            Query::IndustrialOrganisation(reference) => {
                super::inputs::digest_debug(&self.industrial_organisation_unrecorded(*reference))
            }
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
            Query::Unlinked(table, raw) => {
                super::inputs::digest(&self.unlinked_unrecorded(table, *raw))
            }
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
            Query::StateMembers => super::inputs::digest_debug(&self.state_members_unrecorded()),
            Query::StateOrder(state) => super::inputs::digest(&self.state_order_unrecorded(*state)),
            Query::Exists(slot) => super::inputs::digest(&self.exists_unrecorded(*slot)),
            Query::OwnedStateCount(slot) => {
                super::inputs::digest(&self.owned_state_count_unrecorded(*slot))
            }
            Query::CountryCounterInputs(slot) => {
                super::inputs::digest_debug(&self.country_counter_inputs_unrecorded(*slot))
            }
            Query::CountryStateRoster(slot, controlled) => {
                super::inputs::digest(&self.country_state_roster_unrecorded(*slot, *controlled))
            }
            Query::InvasionOrders(slot) => {
                super::inputs::digest_debug(&self.invasion_orders_unrecorded(*slot))
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
            Query::StateOfValue(raw) => {
                super::inputs::digest(&self.state_of_value_unrecorded(*raw))
            }
            Query::UnitLeader(reference) => {
                super::inputs::digest_debug(&self.unit_leader_unrecorded(*reference))
            }
            Query::PartyLeaders(slot) => {
                super::inputs::digest_debug(&self.party_leaders_unrecorded(*slot))
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
            Query::Character(id) => super::inputs::digest_debug(&self.character_unrecorded(*id)),
            Query::Appearance(slot) => {
                super::inputs::digest_debug(&self.appearance_unrecorded(*slot))
            }
            Query::IsCoreOf(state, country) => {
                super::inputs::digest_debug(&self.is_core_of_unrecorded(*state, *country))
            }
            Query::IsClaimedBy(state, country) => {
                super::inputs::digest_debug(&self.is_claimed_by_unrecorded(*state, *country))
            }
            Query::RawCountry(slot) => super::inputs::digest(&self.raw_country_unrecorded(*slot)),
            Query::Nukes(slot) => super::inputs::digest(&self.nukes_unrecorded(*slot)),
        }
    }
}

impl<'a> World<'a> {
    /// Whether `table` is complete this hour, as reading it would report.
    pub(crate) fn table_status(&self, table: replay::rows::TableId) -> Result<(), String> {
        super::inputs::query(
            || super::inputs::Key::World(Query::TableStatus(table)),
            || self.table_status_unrecorded(table),
        )
    }

    fn table_status_unrecorded(&self, id: replay::rows::TableId) -> Result<(), String> {
        use replay::rows::TableId;
        match id {
            TableId::Countries => table::<tables::Countries>(self.frame).map(drop),
            TableId::CountryLinks => table::<tables::CountryLinks>(self.frame).map(drop),
            TableId::States => table::<tables::States>(self.frame).map(drop),
            _ => unreachable!("only the core tables' completeness is queried"),
        }
    }

    /// Every recorded country's canonical slot, ascending: the physical
    /// country-array order.
    pub(crate) fn country_slots(&self) -> Result<Vec<u32>, String> {
        super::inputs::query(
            || super::inputs::Key::World(Query::CountrySlots),
            || self.country_slots_unrecorded(),
        )
    }

    fn country_slots_unrecorded(&self) -> Result<Vec<u32>, String> {
        let mut slots: Vec<u32> = table::<tables::Countries>(self.frame)?
            .iter()
            .map(|row| row.canonical_slot)
            .collect();
        slots.sort_unstable();
        Ok(slots)
    }

    /// Every recorded state's id, ascending: the state-array order.
    pub(crate) fn state_ids(&self) -> Result<Vec<u32>, String> {
        super::inputs::query(
            || super::inputs::Key::World(Query::StateIds),
            || self.state_ids_unrecorded(),
        )
    }

    fn state_ids_unrecorded(&self) -> Result<Vec<u32>, String> {
        let mut ids: Vec<u32> = table::<tables::States>(self.frame)?
            .iter()
            .map(|row| row.id)
            .collect();
        ids.sort_unstable();
        Ok(ids)
    }

    /// A recorded state's native continent token.
    pub(crate) fn state_continent_token(&self, state: u32) -> Result<u32, String> {
        super::inputs::query(
            || super::inputs::Key::World(Query::StateContinentToken(state)),
            || self.state_continent_token_unrecorded(state),
        )
    }

    fn state_continent_token_unrecorded(&self, state: u32) -> Result<u32, String> {
        table::<tables::States>(self.frame)?;
        let row = self
            .state_map()
            .get(&state)
            .ok_or_else(|| format!("state {state} is not recorded"))?;
        Ok(row
            .continent
            .require("States.continent")?
            .ok_or("the native state-template continent capture failed")?)
    }

    /// Whether States is complete and records a state `id`.
    pub(crate) fn state_exists(&self, id: u32) -> Result<bool, String> {
        super::inputs::query(
            || super::inputs::Key::World(Query::StateExists(id)),
            || self.state_exists_unrecorded(id),
        )
    }

    fn state_exists_unrecorded(&self, id: u32) -> Result<bool, String> {
        table::<tables::States>(self.frame)?;
        Ok(self.state_map().contains_key(&id))
    }

    /// A raw country's recorded focus tree id.
    pub(crate) fn focus_tree(&self, raw: u32) -> Result<Option<Vec<u8>>, String> {
        super::inputs::query(
            || super::inputs::Key::World(Query::FocusTree(raw)),
            || self.focus_tree_unrecorded(raw),
        )
    }

    fn focus_tree_unrecorded(&self, raw: u32) -> Result<Option<Vec<u8>>, String> {
        let row = table::<tables::Focus>(self.frame)?
            .iter()
            .find(|row| row.country == raw)
            .ok_or_else(|| format!("country {raw} has no Focus row"))?;
        Ok(row.tree.clone())
    }

    /// Every recorded state by id, read whole: an evaluation depends on the
    /// whole States table ([`super::inputs::table`]).
    pub fn states(&self) -> &BTreeMap<u32, &'a StateRow> {
        super::inputs::table(self.frame, replay::rows::TableId::States);
        self.state_map()
    }

    fn state_map(&self) -> &BTreeMap<u32, &'a StateRow> {
        self.states
            .get_or_init(|| self.state_rows.iter().map(|row| (row.id, row)).collect())
    }

    fn province_map(&self) -> &BTreeMap<u32, &'a ProvinceRow> {
        self.provinces
            .get_or_init(|| self.province_rows.iter().map(|row| (row.id, row)).collect())
    }

    /// The hour this world was read at.
    pub fn hour(&self) -> Hour {
        self.hour
    }

    /// Every raw country reference's canonical slot, read whole.
    pub fn links(&self) -> &BTreeMap<u32, u32> {
        super::inputs::table(self.frame, replay::rows::TableId::CountryLinks);
        &self.links
    }

    /// Every recorded province by id, read whole ([`Self::states`]).
    pub fn provinces(&self) -> &BTreeMap<u32, &'a ProvinceRow> {
        super::inputs::table(self.frame, replay::rows::TableId::Provinces);
        self.province_map()
    }

    /// Content: every state's provinces in its own order.
    pub fn state_provinces(&self) -> &BTreeMap<u32, Vec<u32>> {
        &self.state_provinces
    }

    /// The hour's character tables, read whole: an evaluation depends on
    /// the fields of all six that they read ([`character_scope::FIELDS`]).
    pub(super) fn character_tables(&self) -> &character_scope::Tables<'a> {
        for (table, fields) in character_scope::FIELDS {
            super::inputs::fields(self.frame, table, fields);
        }
        self.character_tables
            .get_or_init(|| super::inputs::unrecorded(|| character_scope::Tables::read(self.frame)))
    }
}

impl<'a> World<'a> {
    /// The recorded content stack's DLC admission, shared by load-time
    /// predicates and native feature selectors derived from that roster.
    pub fn has_dlc(&self, name: &str) -> bool {
        self.dlcs.contains(name)
    }

    /// Content-only checks (ambient admission), with runtime inputs unavailable.
    /// Mask-0 content predicates can still decide in the default event scope.
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
        Self::read(frame, ContentFacts::content_only(dlcs, triggers, aliases))
    }

    /// Whether `province`'s definition has the coastal bit (8),
    /// which the content map decides: a land
    /// province next to sea or an authored-coastal lake, an authored-coastal
    /// lake. A province the map does not know is not coastal.
    pub fn coastal(&self, province: u32) -> Result<bool, String> {
        let terrain = self.terrain().ok_or(
            "the content movement map is unavailable, so a province's coastal flag is unknown",
        )?;
        Ok(terrain
            .terrain(province)
            .is_some_and(|terrain| terrain.flags & 8 != 0))
    }

    /// Shared content readers run only when a geographic operand is reached.
    /// Worlds constructed from static content facts retain those supplied facts.
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn with_geography(mut self, geography: GeographyInputs<'a>) -> Self {
        self.lazy_geography = Some(geography);
        self
    }

    pub(crate) fn neighbor_states(&self) -> Result<&'a BTreeMap<u32, Vec<u32>>, &'a str> {
        if let Some(content) = self.prepared_content {
            return content.geography().neighbors();
        }
        self.lazy_geography
            .map_or_else(|| self.neighbor_states, GeographyInputs::neighbors)
    }

    fn terrain(&self) -> Option<&'a installed_content::movement::MovementMap> {
        if let Some(content) = self.prepared_content {
            return content.geography().terrain();
        }
        self.lazy_geography
            .map_or_else(|| self.terrain, GeographyInputs::terrain)
    }

    /// The hour's inputs, strictly: a raid or scripted text needs the
    /// country links, countries, provinces and states complete.
    pub fn new(frame: &'a DecodedFrame, content: ContentFacts<'a>) -> Result<Self, String> {
        let world = Self::read(frame, content);
        match world.core_gap() {
            Some(gap) => Err(gap),
            None => Ok(world),
        }
    }

    /// The hour's inputs, a table that is not complete noted: the triggers
    /// that read the country links, countries, provinces or states are then
    /// unknown, the others still decided.
    pub fn read(frame: &'a DecodedFrame, content: ContentFacts<'a>) -> Self {
        let ((links, countries, provinces, states), core_gaps) = core_tables(frame);
        Self {
            frame,
            prepared_content: None,
            hour: frame.hour,
            core_gaps,
            links: links
                .iter()
                .map(|row| (row.id, row.canonical_slot))
                .collect(),
            tags: countries
                .iter()
                .map(|row| (row.canonical_slot, row.tag.as_slice()))
                .collect(),
            raw_ids: countries
                .iter()
                .map(|row| (row.canonical_slot, row.id))
                .collect(),
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
            aliases: content.tag_aliases,
            recorded: OnceLock::new(),
            names: OnceLock::new(),
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
            counter_inputs: OnceLock::new(),
            owned_controlled_states: OnceLock::new(),
            opinions: OnceLock::new(),
            building_levels: [OnceLock::new(), OnceLock::new()],
            resource_amounts: content
                .buildings
                .and_then(|buildings| {
                    content
                        .resource_bonuses
                        .map(|bonuses| StateResources::new(frame, buildings, bonuses))
                })
                .into(),
            invasions: OnceLock::new(),
            army_orders: OnceLock::new(),
            army_order_plans: OnceLock::new(),
            group_order_roots: OnceLock::new(),
            script_constants: content.script_constants.into(),
            invasion_preparations: OnceLock::new(),
            invasion_prepare_days: content.invasion_prepare_days.into(),
            leaders: OnceLock::new(),
            characters: OnceLock::new(),
            leader_roles: content.leader_roles.into(),
            appearance: OnceLock::new(),
            cores: OnceLock::new(),
            claims: OnceLock::new(),
            template_names: None,
        }
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

    /// The content's character template tags, folded, or why unread.
    pub(super) fn character_templates(&self) -> Result<&'a BTreeSet<Vec<u8>>, &'a str> {
        self.leader_roles.map(|roles| &roles.names)
    }

    /// Why the core tables are not complete this hour, if they are not.
    pub fn core_gap(&self) -> Option<String> {
        super::inputs::query(
            || super::inputs::Key::World(Query::CoreGap),
            || self.core_gap_unrecorded(),
        )
    }

    fn core_gap_unrecorded(&self) -> Option<String> {
        self.core_gaps.first().map(|(_, why)| why.clone())
    }

    /// Why one core table (`CountryLinks`, `Countries`, `Provinces` or
    /// `States`) is not complete this hour, if it is not.
    pub(super) fn table_gap(&self, table: &str) -> Option<String> {
        super::inputs::query(
            || super::inputs::Key::World(Query::TableGap(table.to_owned())),
            || self.table_gap_unrecorded(table),
        )
    }

    fn table_gap_unrecorded(&self, table: &str) -> Option<String> {
        self.core_gaps
            .iter()
            .find(|(name, _)| *name == table)
            .map(|(_, why)| why.clone())
    }

    /// The hour's scope facts ([`RecordedScopes`]), read on first use.
    pub fn recorded(&self) -> &RecordedScopes {
        self.recorded.get_or_init(|| {
            super::inputs::unrecorded(|| RecordedScopes::read(self.frame, Arc::clone(self.aliases)))
        })
    }

    pub fn frame(&self) -> &'a DecodedFrame {
        self.frame
    }

    /// Retained army name and historical generator operands, captured
    /// at DivisionStatus freshness.
    pub(crate) fn unit_status(
        &self,
        reference: u64,
    ) -> Result<&'a replay::rows::DivisionStatusRow, String> {
        super::inputs::query_debug(
            || super::inputs::Key::World(Query::UnitStatus(reference)),
            || self.unit_status_unrecorded(reference),
        )
    }

    fn unit_status_unrecorded(
        &self,
        reference: u64,
    ) -> Result<&'a replay::rows::DivisionStatusRow, String> {
        self.unit(reference)?;
        table::<tables::DivisionStatus>(self.frame)?
            .iter()
            .find(|row| row.division == reference)
            // Retain the existing carrier diagnostic when no status row was captured.
            .ok_or_else(|| format!("UNIT {reference:#x} name: cached army name is not recorded"))
    }

    /// Selected active-raid wing bytes, at RaidTooltipOperands freshness.
    /// Raid naming takes a resolving army before the wing; the wing
    /// retains no naming provenance, so the caller must use stored_text.
    pub(crate) fn raid_wing_name(&self, reference: u64) -> Result<Vec<u8>, String> {
        super::inputs::query(
            || super::inputs::Key::World(Query::RaidWingName(reference)),
            || self.raid_wing_name_unrecorded(reference),
        )
    }

    fn raid_wing_name_unrecorded(&self, reference: u64) -> Result<Vec<u8>, String> {
        crate::air_wing_names::raid_name(self.frame, reference)
    }

    pub(crate) fn industrial_organisation(
        &self,
        reference: u64,
    ) -> Result<&'a replay::rows::IndustrialOrganisationRow, String> {
        super::inputs::query_debug(
            || super::inputs::Key::World(Query::IndustrialOrganisation(reference)),
            || self.industrial_organisation_unrecorded(reference),
        )
    }

    fn industrial_organisation_unrecorded(
        &self,
        reference: u64,
    ) -> Result<&'a replay::rows::IndustrialOrganisationRow, String> {
        table::<tables::IndustrialOrganisations>(self.frame)?
            .iter()
            .find(|row| row.organisation == reference)
            .ok_or_else(|| format!("IndustrialOrganisations lacks organisation {reference:#x}"))
    }

    pub fn slot(&self, raw: u32) -> Option<u32> {
        super::inputs::query(
            || super::inputs::Key::World(Query::Slot(raw)),
            || self.slot_unrecorded(raw),
        )
    }

    fn slot_unrecorded(&self, raw: u32) -> Option<u32> {
        self.links.get(&raw).copied()
    }

    pub fn tag(&self, slot: u32) -> Option<&'a [u8]> {
        super::inputs::query(
            || super::inputs::Key::World(Query::Tag(slot)),
            || self.tag_unrecorded(slot),
        )
    }

    fn tag_unrecorded(&self, slot: u32) -> Option<&'a [u8]> {
        self.tags.get(&slot).copied()
    }

    /// A recorded state's raw owner and controller.
    pub fn state_row(&self, state: u32) -> Option<(u32, u32)> {
        super::inputs::query(
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

    /// A recorded province's raw owner and controller.
    pub fn province_row(&self, province: u32) -> Option<(u32, u32)> {
        super::inputs::query(
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
        super::inputs::query(
            || super::inputs::Key::World(Query::StateController(state)),
            || self.state_controller_unrecorded(state),
        )
    }

    fn state_controller_unrecorded(&self, state: u32) -> Option<u32> {
        self.slot(self.state_map().get(&state)?.controller)
    }

    pub fn province_controller(&self, province: u32) -> Option<u32> {
        super::inputs::query(
            || super::inputs::Key::World(Query::ProvinceController(province)),
            || self.province_controller_unrecorded(province),
        )
    }

    fn province_controller_unrecorded(&self, province: u32) -> Option<u32> {
        self.slot(self.province_map().get(&province)?.controller)
    }

    /// A lexer token's literal.
    pub fn token(&self, token: u32) -> Result<Option<&'a [u8]>, String> {
        super::inputs::query(
            || super::inputs::Key::World(Query::Token(token)),
            || self.token_unrecorded(token),
        )
    }

    fn token_unrecorded(&self, token: u32) -> Result<Option<&'a [u8]>, String> {
        {
            let literals = get(&self.literals, || {
                Ok(table::<tables::LexicalTokens>(self.frame)?
                    .iter()
                    .map(|row| (row.token, row.literal.as_slice()))
                    .collect())
            })?;
            Ok(literals.get(&token).copied())
        }
    }

    /// The lexer token of a name: the one whose literal equals it under the
    /// lexer's fold (ASCII A-Z only), as the
    /// game finds a token for a key.
    pub fn token_of(&self, name: &[u8]) -> Result<Option<u32>, String> {
        super::inputs::query(
            || super::inputs::Key::World(Query::TokenOf(name.to_vec())),
            || self.token_of_unrecorded(name),
        )
    }

    fn token_of_unrecorded(&self, name: &[u8]) -> Result<Option<u32>, String> {
        match self.names()?.get(&name.to_ascii_lowercase()) {
            None => Ok(None),
            Some(Ok(token)) => Ok(Some(*token)),
            Some(Err(reason)) => Err(reason.clone()),
        }
    }

    pub(super) fn names(&self) -> Result<&Folds, String> {
        super::inputs::table(self.frame, replay::rows::TableId::LexicalTokens);
        get(&self.names, || {
            let view = &self.frame.tables[replay::rows::TableId::LexicalTokens.index()];
            let cached = |cache: &[(ViewIdentity, Arc<Folds>)]| {
                cache
                    .iter()
                    .find(|(identity, _)| identity.is(view))
                    .map(|(_, folds)| Arc::clone(folds))
            };
            if let Some(folds) = cached(&FOLDS.lock().unwrap_or_else(|e| e.into_inner())) {
                return Ok(folds);
            }
            let mut out = Folds::new();
            for row in table::<tables::LexicalTokens>(self.frame)? {
                let folded = row.literal.to_ascii_lowercase();
                match out.get_mut(&folded) {
                    None => {
                        out.insert(folded, Ok(row.token));
                    }
                    Some(held) => {
                        // One token per fold natively: two are refused.
                        *held = Err(refused(format!(
                            "{} names several lexer tokens",
                            String::from_utf8_lossy(&row.literal)
                        )));
                    }
                }
            }
            let out = Arc::new(out);
            let mut cache = FOLDS.lock().unwrap_or_else(|e| e.into_inner());
            cache.retain(|(identity, _)| !identity.is(view));
            cache.insert(0, (view.identity(), Arc::clone(&out)));
            // One per table allocation a concurrent scan can be reading.
            cache.truncate(8);
            Ok(out)
        })
        .map(|folds| &**folds)
    }

    /// Why a raw country reference of `table` does not resolve.
    pub(super) fn unlinked(&self, table: &str, raw: u32) -> String {
        super::inputs::query(
            || super::inputs::Key::World(Query::Unlinked(table.to_owned(), raw)),
            || self.unlinked_unrecorded(table, raw),
        )
    }

    fn unlinked_unrecorded(&self, table: &str, raw: u32) -> String {
        format!("a {table} row's country {raw} has no CountryLinks row")
    }

    /// The country's faction, if any.
    pub fn faction(&self, slot: u32) -> Result<Option<u64>, String> {
        super::inputs::query(
            || super::inputs::Key::World(Query::Faction(slot)),
            || self.faction_unrecorded(slot),
        )
    }

    fn faction_unrecorded(&self, slot: u32) -> Result<Option<u64>, String> {
        if slot == 0 {
            let [reserved] = table::<tables::ReservedCountryNaming>(self.frame)? else {
                return Err("ReservedCountryNaming does not hold one reserved country".into());
            };
            // The positive membership census does not reverse-certify Country0
            // absence. Its authenticated faction getter does, including null.
            return reserved
                .faction
                .require("ReservedCountryNaming.faction")?
                .ok_or_else(|| "ReservedCountryNaming.faction capture unavailable".into())
                .map(|faction| (faction != 0).then_some(faction));
        }
        let (factions, unresolved) = get(&self.factions, || {
            let mut out = BTreeMap::new();
            let mut unresolved = None;
            for row in table::<tables::FactionMembers>(self.frame)? {
                // A faction's member roster can contain genuine Country0. That
                // literal member cannot be an unresolved positive country.
                if row.country == 0 {
                    continue;
                }
                match self.slot(row.country) {
                    Some(slot) => {
                        out.insert(slot, row.faction);
                    }
                    None => unresolved = Some(self.unlinked("FactionMembers", row.country)),
                }
            }
            Ok((out, unresolved))
        })?;
        linked(factions.get(&slot).copied(), unresolved)
    }

    pub fn same_faction(&self, a: u32, b: u32) -> Result<bool, String> {
        super::inputs::query(
            || super::inputs::Key::World(Query::SameFaction(a, b)),
            || self.same_faction_unrecorded(a, b),
        )
    }

    fn same_faction_unrecorded(&self, a: u32, b: u32) -> Result<bool, String> {
        Ok(matches!((self.faction(a)?, self.faction(b)?), (Some(x), Some(y)) if x == y))
    }

    pub fn at_war(&self, a: u32, b: u32) -> Result<bool, String> {
        super::inputs::query(
            || super::inputs::Key::World(Query::AtWar(a, b)),
            || self.at_war_unrecorded(a, b),
        )
    }

    fn at_war_unrecorded(&self, a: u32, b: u32) -> Result<bool, String> {
        let (wars, unresolved) = get(&self.wars, || {
            let mut out = BTreeSet::new();
            let mut unresolved = None;
            for row in table::<tables::WarRelations>(self.frame)?
                .iter()
                .filter(|row| ongoing(row.start_hour, row.end_hour, self.hour))
            {
                match (self.slot(row.first_country), self.slot(row.second_country)) {
                    (Some(x), Some(y)) => {
                        out.insert((x.min(y), x.max(y)));
                    }
                    (None, _) => {
                        unresolved = Some(self.unlinked("WarRelations", row.first_country))
                    }
                    (_, None) => {
                        unresolved = Some(self.unlinked("WarRelations", row.second_country));
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

    pub fn overlord(&self, subject: u32) -> Result<Option<u32>, String> {
        super::inputs::query(
            || super::inputs::Key::World(Query::Overlord(subject)),
            || self.overlord_unrecorded(subject),
        )
    }

    fn overlord_unrecorded(&self, subject: u32) -> Result<Option<u32>, String> {
        let (overlords, unresolved) = get(&self.overlords, || {
            let mut out = BTreeMap::new();
            let mut unresolved = None;
            for row in table::<tables::Subjects>(self.frame)?
                .iter()
                .filter(|row| ongoing(row.start_hour, row.end_hour, self.hour))
            {
                match (self.slot(row.subject), self.slot(row.overlord)) {
                    (Some(subject), Some(overlord)) => {
                        out.insert(subject, overlord);
                    }
                    (None, _) => unresolved = Some(self.unlinked("Subjects", row.subject)),
                    (_, None) => unresolved = Some(self.unlinked("Subjects", row.overlord)),
                }
            }
            Ok((out, unresolved))
        })?;
        linked(overlords.get(&subject).copied(), unresolved)
    }

    /// Numeric original-tag reference, independent of tag text.
    pub(crate) fn original_reference(&self, slot: u32) -> Result<i32, String> {
        super::inputs::query(
            || super::inputs::Key::World(Query::OriginalReference(slot)),
            || self.original_reference_unrecorded(slot),
        )
    }

    fn original_reference_unrecorded(&self, slot: u32) -> Result<i32, String> {
        installed_content::country_names::original_or_own_reference(self.frame, slot)
    }

    /// Country-tag equality compares raw words first, then their canonical
    /// indices when both differ and neither is zero. Missing mappings refuse.
    pub(crate) fn same_tag(&self, a: i32, b: i32) -> Result<bool, String> {
        super::inputs::query(
            || super::inputs::Key::World(Query::SameTag(a, b)),
            || self.same_tag_unrecorded(a, b),
        )
    }

    fn same_tag_unrecorded(&self, a: i32, b: i32) -> Result<bool, String> {
        if a == b {
            return Ok(true);
        }
        if a == 0 || b == 0 {
            return Ok(false);
        }
        Ok(self.recorded().raw_slot(a as u32)? == self.recorded().raw_slot(b as u32)?)
    }

    pub(super) fn has_focus(&self, slot: u32, name: &[u8]) -> Result<bool, String> {
        super::inputs::query(
            || super::inputs::Key::World(Query::HasFocus(slot, name.to_vec())),
            || self.has_focus_unrecorded(slot, name),
        )
    }

    fn has_focus_unrecorded(&self, slot: u32, name: &[u8]) -> Result<bool, String> {
        let rows = table::<tables::CompletedFocuses>(self.frame)?;
        let Some(countries) = self.country_rows(&self.focus_rows, rows, |row| row.country) else {
            return self.has_focus_indexed(slot, name);
        };
        let found = self
            .slot_links(slot)
            .iter()
            .any(|&raw| rows[countries.of(raw)].iter().any(|row| row.token == name));
        let unresolved = countries
            .unlinked
            .map(|(_, last)| self.unlinked("CompletedFocuses", last));
        Ok(linked(found.then_some(()), &unresolved)?.is_some())
    }

    /// [`Self::has_focus_unrecorded`] over an index of every row, for rows
    /// that do not ascend by country.
    fn has_focus_indexed(&self, slot: u32, name: &[u8]) -> Result<bool, String> {
        let (focuses, unresolved) = get(&self.focuses, || {
            let mut out = BTreeSet::new();
            let mut unresolved = None;
            for row in table::<tables::CompletedFocuses>(self.frame)? {
                match self.slot(row.country) {
                    Some(slot) => {
                        out.insert((slot, row.token.as_slice()));
                    }
                    None => unresolved = Some(self.unlinked("CompletedFocuses", row.country)),
                }
            }
            Ok((out, unresolved))
        })?;
        Ok(linked(focuses.contains(&(slot, name)).then_some(()), unresolved)?.is_some())
    }

    pub(super) fn has_tech(&self, slot: u32, name: &[u8]) -> Result<bool, String> {
        super::inputs::query(
            || super::inputs::Key::World(Query::HasTech(slot, name.to_vec())),
            || self.has_tech_unrecorded(slot, name),
        )
    }

    fn has_tech_unrecorded(&self, slot: u32, name: &[u8]) -> Result<bool, String> {
        let techs = get(&self.techs, || {
            let definitions = table::<tables::TechnologyDefinitions>(self.frame)?;
            let names: BTreeMap<i32, Vec<u8>> = definitions
                .iter()
                .map(|row| {
                    (
                        row.database_index,
                        installed_content::parser::c_string(&row.id).to_ascii_lowercase(),
                    )
                })
                .collect();
            let mut first = BTreeMap::new();
            // Pick the first name; completion tests only
            // that indexed instance's completed classifier, not researched membership.
            for row in definitions {
                first
                    .entry(installed_content::parser::c_string(&row.id).to_ascii_lowercase())
                    .or_insert(row.database_index);
            }
            let mut techs = Techs::default();
            for row in table::<tables::ResearchTechnologies>(self.frame)? {
                if row.classifier != engine::ctechnology::CLASSIFIER_COMPLETED {
                    continue;
                }
                match (self.slot(row.country), names.get(&row.database_index)) {
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
        if techs
            .researched
            .contains(&(slot, installed_content::parser::c_string(name).to_ascii_lowercase()))
        {
            return Ok(true);
        }
        // A completed technology this country holds (or a country that does
        // not resolve) with no recorded definition name could be this one.
        if techs.unresolved || techs.unnamed.contains(&slot) {
            return Err(format!(
                "has_tech {}: a completed technology has no recorded definition or country",
                String::from_utf8_lossy(name)
            ));
        }
        Ok(false)
    }

    pub(super) fn completed_project(&self, slot: u32, name: &[u8]) -> Result<bool, String> {
        super::inputs::query(
            || super::inputs::Key::World(Query::CompletedProject(slot, name.to_vec())),
            || self.completed_project_unrecorded(slot, name),
        )
    }

    fn completed_project_unrecorded(&self, slot: u32, name: &[u8]) -> Result<bool, String> {
        let Some(token) = self.token_of(name)? else {
            return Ok(false);
        };
        let (projects, unresolved) = get(&self.projects, || {
            let mut out = BTreeSet::new();
            let mut unresolved = None;
            for row in table::<tables::Projects>(self.frame)?
                .iter()
                .filter(|row| row.completed != 0)
            {
                match self.slot(row.owner) {
                    Some(slot) => {
                        out.insert((slot, row.definition_id));
                    }
                    None => unresolved = Some(self.unlinked("Projects", row.owner)),
                }
            }
            Ok((out, unresolved))
        })?;
        Ok(linked(projects.contains(&(slot, token)).then_some(()), unresolved)?.is_some())
    }

    /// `slot`'s raw country references, ascending: the rows of a
    /// country-keyed table whose country resolves to it.
    fn slot_links(&self, slot: u32) -> &[u32] {
        self.slot_links
            .get_or_init(|| {
                let mut out: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
                for (&raw, &slot) in &self.links {
                    out.entry(slot).or_default().push(raw);
                }
                out
            })
            .get(&slot)
            .map_or(&[], Vec::as_slice)
    }

    /// `rows` by their raw `country`, built on first use; None when they
    /// do not ascend by country.
    fn country_rows<'s, R>(
        &'s self,
        cell: &'s OnceLock<Option<CountryRows>>,
        rows: &[R],
        country: impl Fn(&R) -> u32,
    ) -> Option<&'s CountryRows> {
        cell.get_or_init(|| CountryRows::new(rows, country, |raw| self.links.contains_key(&raw)))
            .as_ref()
    }

    fn political_parties(&self) -> Result<&PoliticalParties<'a>, String> {
        get(&self.political_parties, || {
            let mut out = PoliticalParties {
                ruling: BTreeMap::new(),
                unresolved: None,
                ids: BTreeMap::new(),
                tokens: BTreeMap::new(),
                members: BTreeSet::new(),
            };
            for row in table::<tables::PoliticalParties>(self.frame)? {
                if let Some(previous) = out.ids.insert(row.id, &row.token)
                    && !previous.eq_ignore_ascii_case(&row.token)
                {
                    return Err(format!(
                        "PoliticalParties ideology ID {} has conflicting tokens",
                        row.id
                    ));
                }
                if let Some(previous) = out.tokens.insert(row.token.to_ascii_lowercase(), row.id)
                    && previous != row.id
                {
                    return Err(format!(
                        "PoliticalParties ideology {} has conflicting native IDs",
                        String::from_utf8_lossy(&row.token)
                    ));
                }
                match self.slot(row.country) {
                    Some(slot) => {
                        out.members.insert((slot, row.id));
                        if row.is_ruling != 0 {
                            out.ruling.insert(slot, row.id);
                        }
                    }
                    None => out.unresolved = Some(self.unlinked("PoliticalParties", row.country)),
                }
            }
            Ok(out)
        })
    }

    pub(super) fn government(&self, slot: u32) -> Result<Option<u32>, String> {
        super::inputs::query(
            || super::inputs::Key::World(Query::Government(slot)),
            || self.government_unrecorded(slot),
        )
    }

    fn government_unrecorded(&self, slot: u32) -> Result<Option<u32>, String> {
        let parties = self.political_parties()?;
        linked(parties.ruling.get(&slot).copied(), &parties.unresolved)
    }

    pub(crate) fn named_ideology(&self, token: &[u8]) -> Result<(u32, &'a [u8]), String> {
        super::inputs::query(
            || super::inputs::Key::World(Query::NamedIdeology(token.to_vec())),
            || self.named_ideology_unrecorded(token),
        )
    }

    fn named_ideology_unrecorded(&self, token: &[u8]) -> Result<(u32, &'a [u8]), String> {
        let parties = self.political_parties()?;
        let id = parties
            .tokens
            .get(&token.to_ascii_lowercase())
            .copied()
            .ok_or_else(|| {
                format!(
                    "ideology {} has no recorded native ID/token",
                    String::from_utf8_lossy(token)
                )
            })?;
        Ok((id, parties.ids[&id]))
    }

    pub(super) fn ideology_token(&self, id: u32) -> Result<Option<&'a [u8]>, String> {
        super::inputs::query(
            || super::inputs::Key::World(Query::IdeologyToken(id)),
            || self.ideology_token_unrecorded(id),
        )
    }

    fn ideology_token_unrecorded(&self, id: u32) -> Result<Option<&'a [u8]>, String> {
        Ok(self.political_parties()?.ids.get(&id).copied())
    }

    pub(super) fn scoped_ruling_ideology(&self, slot: u32) -> Result<(u32, &'a [u8]), String> {
        super::inputs::query(
            || super::inputs::Key::World(Query::ScopedRulingIdeology(slot)),
            || self.scoped_ruling_ideology_unrecorded(slot),
        )
    }

    fn scoped_ruling_ideology_unrecorded(&self, slot: u32) -> Result<(u32, &'a [u8]), String> {
        let (ruling, unresolved) = get(&self.politics_ruling, || {
            let mut out = BTreeMap::new();
            let mut unresolved = None;
            for row in table::<tables::Politics>(self.frame)? {
                match self.slot(row.country) {
                    Some(slot) => {
                        out.insert(slot, row.ruling_party);
                    }
                    None => unresolved = Some(self.unlinked("Politics", row.country)),
                }
            }
            Ok((out, unresolved))
        })?;
        let id = linked(ruling.get(&slot).copied(), unresolved)?
            .flatten()
            .ok_or("Politics lacks ruling ideology")?;
        let parties = self.political_parties()?;
        if !parties.members.contains(&(slot, id)) {
            return Err("PoliticalParties lacks ruling ideology".into());
        }
        Ok((id, parties.ids[&id]))
    }

    /// The elections trigger tests the
    /// country's political status (`Politics.has_elections`).
    pub(super) fn has_elections(&self, slot: u32) -> Result<bool, String> {
        super::inputs::query(
            || super::inputs::Key::World(Query::HasElections(slot)),
            || self.has_elections_unrecorded(slot),
        )
    }

    fn has_elections_unrecorded(&self, slot: u32) -> Result<bool, String> {
        let (elections, unresolved) = get(&self.politics_elections, || {
            let mut out = BTreeMap::new();
            let mut unresolved = None;
            for row in table::<tables::Politics>(self.frame)? {
                match self.slot(row.country) {
                    Some(slot) => {
                        out.insert(slot, row.has_elections);
                    }
                    None => unresolved = Some(self.unlinked("Politics", row.country)),
                }
            }
            Ok((out, unresolved))
        })?;
        linked(elections.get(&slot).copied(), unresolved)?
            .ok_or_else(|| format!("Politics lacks country slot {slot}"))?
            .require("Politics.has_elections")
            .copied()
    }

    /// A script flag's value, or `None` when it is not set.
    pub(super) fn flag(&self, kind: u8, id: u32, name: &[u8]) -> Result<Option<i16>, String> {
        super::inputs::query(
            || super::inputs::Key::World(Query::Flag(kind, id, name.to_vec())),
            || self.flag_unrecorded(kind, id, name),
        )
    }

    fn flag_unrecorded(&self, kind: u8, id: u32, name: &[u8]) -> Result<Option<i16>, String> {
        let Some(token) = self.token_of(name)? else {
            return Ok(None);
        };
        // Rows hold unique keys ascending by (scope kind, scope id, token).
        let flags = table::<tables::ScriptFlags>(self.frame)?;
        Ok(flags
            .binary_search_by_key(&(kind, id, token), |row| {
                (row.scope_kind, row.scope_id, row.token)
            })
            .ok()
            .map(|at| flags[at].value))
    }

    /// Select the country's dynamic-modifier container.
    /// Recorded definition names compare by exact bytes.
    pub(super) fn has_country_modifier(
        &self,
        slot: u32,
        name: &[u8],
        target: Option<Scope>,
    ) -> Result<bool, String> {
        super::inputs::query(
            || super::inputs::Key::World(Query::HasCountryModifier(slot, name.to_vec(), target)),
            || self.has_country_modifier_unrecorded(slot, name, target),
        )
    }

    fn has_country_modifier_unrecorded(
        &self,
        slot: u32,
        name: &[u8],
        target: Option<Scope>,
    ) -> Result<bool, String> {
        let (modifiers, unlinked) = get(&self.country_modifiers, || {
            let mut modifiers: Modifiers<'a> = BTreeMap::new();
            let mut unlinked = BTreeMap::new();
            let mut rows: Vec<_> = table::<tables::DynamicModifiers>(self.frame)?
                .iter()
                .collect();
            rows.sort_by_key(|row| (row.country, row.ordinal));
            for row in rows {
                match self.slot(row.country) {
                    Some(slot) => {
                        modifiers
                            .entry((slot, row.token.as_slice()))
                            .or_default()
                            .push((row.scope_country, row.scope_state));
                    }
                    None => {
                        unlinked.insert(row.token.as_slice(), row.country);
                    }
                }
            }
            Ok((modifiers, unlinked))
        })?;
        let entries = modifiers.get(&(slot, name)).map_or(&[][..], Vec::as_slice);
        let found = self.modifier_matches(entries, target, "DynamicModifiers");
        // A row of an unlinked owner may belong to this country. With no
        // target it may precede the known first entry; with an explicit
        // target a proven match anywhere already answers true.
        if let Some(raw) = unlinked.get(name) {
            if target.is_some() && found == Ok(true) {
                return found;
            }
            return Err(self.unlinked("DynamicModifiers", *raw));
        }
        found
    }

    /// Select the state's dynamic-modifier container. StateDynamicModifiers keeps
    /// the same entry target fields and native order as the country table.
    pub(super) fn has_state_modifier(
        &self,
        state: u32,
        name: &[u8],
        target: Option<Scope>,
    ) -> Result<bool, String> {
        super::inputs::query(
            || super::inputs::Key::World(Query::HasStateModifier(state, name.to_vec(), target)),
            || self.has_state_modifier_unrecorded(state, name, target),
        )
    }

    fn has_state_modifier_unrecorded(
        &self,
        state: u32,
        name: &[u8],
        target: Option<Scope>,
    ) -> Result<bool, String> {
        let modifiers = get(&self.state_modifiers, || {
            let mut modifiers: Modifiers<'a> = BTreeMap::new();
            let mut rows: Vec<_> = table::<tables::StateDynamicModifiers>(self.frame)?
                .iter()
                .collect();
            rows.sort_by_key(|row| (row.state, row.ordinal));
            for row in rows {
                modifiers
                    .entry((row.state, row.token.as_slice()))
                    .or_default()
                    .push((row.scope_country, row.scope_state));
            }
            Ok(modifiers)
        })?;
        self.modifier_matches(
            modifiers.get(&(state, name)).map_or(&[][..], Vec::as_slice),
            target,
            "StateDynamicModifiers",
        )
    }

    /// Match a positive country target first,
    /// otherwise a positive state target, otherwise only the first entry.
    /// Country equality is raw equality or nonzero alias equality through
    /// CountryLinks; the other target field is ignored. Presence
    /// never tests enabled or remaining days.
    fn modifier_matches(
        &self,
        entries: &[(u32, i32)],
        target: Option<Scope>,
        table: &str,
    ) -> Result<bool, String> {
        let country = match target {
            Some(Scope::Country(slot) | Scope::CountryLeader { country: slot, .. }) => {
                let raw = self.raw_country(slot).ok_or_else(|| {
                    format!("has_dynamic_modifier target country slot {slot} has no Countries row")
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

    /// The state resource operands, or why the content lacks what they read.
    fn resource_amounts(&self) -> Result<&StateResources<'a>, String> {
        self.resource_amounts
            .as_ref()
            .map_err(|why| format!("the state resource amounts are unavailable: {why}"))
    }

    /// The resource count amount, fixed5
    /// (`crate::state_resources`), with the state's controller's overrides.
    pub fn state_resource(&self, state: u32, resource: u32) -> Result<i64, String> {
        super::inputs::query(
            || super::inputs::Key::World(Query::StateResource(state, resource)),
            || self.state_resource_unrecorded(state, resource),
        )
    }

    fn state_resource_unrecorded(&self, state: u32, resource: u32) -> Result<i64, String> {
        self.state_resource_amount(state, resource, self.state_controller(state))
    }

    /// [`Self::state_resource`] with the controller already resolved.
    pub(super) fn state_resource_amount(
        &self,
        state: u32,
        resource: u32,
        controller: Option<u32>,
    ) -> Result<i64, String> {
        super::inputs::query(
            || super::inputs::Key::World(Query::StateResourceAmount(state, resource, controller)),
            || self.state_resource_amount_unrecorded(state, resource, controller),
        )
    }

    fn state_resource_amount_unrecorded(
        &self,
        state: u32,
        resource: u32,
        controller: Option<u32>,
    ) -> Result<i64, String> {
        self.resource_amounts()?
            .amount(state, resource, controller, self)
    }

    /// A province (`province`) or state building's level, damage
    /// included; `None` when the location has none of that type.
    pub fn building_level(
        &self,
        province: bool,
        location: u32,
        building_type: u32,
    ) -> Result<Option<i16>, String> {
        super::inputs::query(
            || super::inputs::Key::World(Query::BuildingLevel(province, location, building_type)),
            || self.building_level_unrecorded(province, location, building_type),
        )
    }

    fn building_level_unrecorded(
        &self,
        province: bool,
        location: u32,
        building_type: u32,
    ) -> Result<Option<i16>, String> {
        let levels = get(&self.building_levels[usize::from(province)], || {
            let rows = if province {
                table::<tables::ProvinceBuildings>(self.frame)?
            } else {
                table::<tables::StateBuildings>(self.frame)?
            };
            let mut out = BTreeMap::new();
            for row in rows {
                // The first row of a key answers, as a scan would.
                out.entry((row.location, row.building_type))
                    .or_insert(row.level);
            }
            Ok(out)
        })?;
        Ok(levels.get(&(location, building_type)).copied())
    }

    /// Each state's recorded provinces, ascending: the runtime membership
    /// which can differ from the content's.
    pub(super) fn state_members(&self) -> Result<&BTreeMap<u32, Vec<u32>>, String> {
        super::inputs::query_debug(
            || super::inputs::Key::World(Query::StateMembers),
            || self.state_members_unrecorded(),
        )
    }

    fn state_members_unrecorded(&self) -> Result<&BTreeMap<u32, Vec<u32>>, String> {
        get(&self.state_members, || {
            let mut out: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
            for (&id, row) in self.province_map() {
                out.entry(row.state).or_default().push(id);
            }
            Ok(out)
        })
    }

    /// A state's provinces in their native order: the content's, which the
    /// runtime list keeps while its membership equals the content's.
    pub fn state_order(&self, state: u32) -> Result<&'a [u32], String> {
        super::inputs::query(
            || super::inputs::Key::World(Query::StateOrder(state)),
            || self.state_order_unrecorded(state),
        )
    }

    fn state_order_unrecorded(&self, state: u32) -> Result<&'a [u32], String> {
        self.state_content.map_err(str::to_owned)?;
        let content = self
            .state_provinces
            .get(&state)
            .ok_or_else(|| format!("state {state} is not in the content"))?;
        let recorded = self
            .state_members()?
            .get(&state)
            .map_or(&[][..], Vec::as_slice);
        if recorded.len() != content.len()
            || content.iter().any(|id| recorded.binary_search(id).is_err())
        {
            return Err(format!(
                "state {state}'s provinces differ from the content, so their native order is unknown"
            ));
        }
        Ok(content)
    }

    /// A country exists if it owns a state;
    /// the country database holds every tag with a
    /// country file, existing or not.
    pub(crate) fn exists(&self, slot: u32) -> Result<bool, String> {
        super::inputs::query(
            || super::inputs::Key::World(Query::Exists(slot)),
            || self.exists_unrecorded(slot),
        )
    }

    fn exists_unrecorded(&self, slot: u32) -> Result<bool, String> {
        self.owned_state_count(slot).map(|count| count > 0)
    }

    /// Stored signed count, used by the owned-state builtin.
    pub(crate) fn owned_state_count(&self, slot: u32) -> Result<i32, String> {
        super::inputs::query(
            || super::inputs::Key::World(Query::OwnedStateCount(slot)),
            || self.owned_state_count_unrecorded(slot),
        )
    }

    fn owned_state_count_unrecorded(&self, slot: u32) -> Result<i32, String> {
        if slot == 0 {
            return self.recorded().owned_states(0);
        }
        Ok(self.country_counter_inputs(slot)?.owned_state_count)
    }

    /// Stored country rosters, shared by iterators and collections.
    pub(crate) fn country_counter_inputs(
        &self,
        slot: u32,
    ) -> Result<&'a CountryCounterInputRow, String> {
        super::inputs::query_debug(
            || super::inputs::Key::World(Query::CountryCounterInputs(slot)),
            || self.country_counter_inputs_unrecorded(slot),
        )
    }

    fn country_counter_inputs_unrecorded(
        &self,
        slot: u32,
    ) -> Result<&'a CountryCounterInputRow, String> {
        let rows = get(&self.counter_inputs, || {
            Ok(table::<tables::CountryCounterInputs>(self.frame)?
                .iter()
                .map(|row| (row.country, row))
                .collect())
        })?;
        let raw = self
            .raw_country(slot)
            .ok_or_else(|| format!("country slot {slot} has no Countries row"))?;
        rows.get(&raw)
            .copied()
            .ok_or_else(|| format!("country {raw} has no CountryCounterInputs row"))
    }

    /// Stored country state-vector order.
    /// Only older recordings may derive order-independent membership.
    pub(super) fn country_state_roster(
        &self,
        slot: u32,
        controlled: bool,
    ) -> Result<Option<&'a [u32]>, String> {
        super::inputs::query(
            || super::inputs::Key::World(Query::CountryStateRoster(slot, controlled)),
            || self.country_state_roster_unrecorded(slot, controlled),
        )
    }

    fn country_state_roster_unrecorded(
        &self,
        slot: u32,
        controlled: bool,
    ) -> Result<Option<&'a [u32]>, String> {
        let inputs = self.country_counter_inputs(slot)?;
        let (recorded, name) = if controlled {
            (
                &inputs.controlled_states,
                "CountryCounterInputs.controlled_states",
            )
        } else {
            (&inputs.owned_states, "CountryCounterInputs.owned_states")
        };
        match recorded {
            replay::rows::Recorded::NotRecorded => Ok(None),
            replay::rows::Recorded::Value(Some(states)) => Ok(Some(states.as_slice())),
            replay::rows::Recorded::Value(None) => {
                Err(refused(format!("{name} capture unavailable")))
            }
        }
    }

    /// The type-3 (naval invasion) orders
    /// reached from a country: each of its theatres, each
    /// group of the theatre's regular roster (the army-group roster
    /// is not read), and per group the primary roots of type 3
    /// followed by every ordinary successor
    /// recursively, whatever its own type. Only whether one matches is asked,
    /// so traversal order and the native duplicate check do not matter.
    pub(super) fn invasion_orders(&self, slot: u32) -> Result<&[Invasion<'a>], String> {
        super::inputs::query_debug(
            || super::inputs::Key::World(Query::InvasionOrders(slot)),
            || self.invasion_orders_unrecorded(slot),
        )
    }

    fn invasion_orders_unrecorded(&self, slot: u32) -> Result<&[Invasion<'a>], String> {
        if slot == 0 {
            let [reserved] = table::<tables::ReservedCountryNaming>(self.frame)? else {
                return Err("ReservedCountryNaming does not hold one reserved country".into());
            };
            let count = reserved
                .land_theatre_count
                .require("ReservedCountryNaming.land_theatre_count")?
                .ok_or("ReservedCountryNaming.land_theatre_count capture unavailable")?;
            if count == 0 {
                // The fresh reserved root certifies an empty native vector.
                // Unrelated positive theatre/OOB refusals cannot change it.
                if let Ok(rows) = self.frame.complete::<tables::LandTheatres>()
                    && rows.iter().any(|row| row.owner == 0)
                {
                    return Err("reserved-country LandTheatres contradict their certificate".into());
                }
                return Ok(&[]);
            }
            let mut ids = BTreeSet::new();
            let mut ordinals = BTreeSet::new();
            for row in table::<tables::LandTheatres>(self.frame)?
                .iter()
                .filter(|row| row.owner == 0)
            {
                if row.ordinal >= count || !ids.insert(row.id) || !ordinals.insert(row.ordinal) {
                    return Err("reserved-country LandTheatres contradict their certificate".into());
                }
            }
            if ids.len() != count as usize {
                return Err(
                    "reserved-country LandTheatres do not match their certified count".into(),
                );
            }
        }
        let (countries, unresolved) = get(&self.invasions, || {
            let mut unresolved = None;
            let mut theatres = BTreeMap::new();
            for row in table::<tables::LandTheatres>(self.frame)? {
                // The optional Country0 suffix is separate from the positive
                // country census. Its literal owner is not a failed link.
                if row.owner == 0 {
                    theatres.insert(row.id, 0);
                    continue;
                }
                match self.slot(row.owner) {
                    Some(owner) => {
                        theatres.insert(row.id, owner);
                    }
                    None => unresolved = Some(self.unlinked("LandTheatres", row.owner)),
                }
            }
            let mut out: Invasions<'a> = BTreeMap::new();
            for group in table::<tables::OrdersGroups>(self.frame)? {
                // `theatre_ordinal` is the group's place in the roster
                // `is_army_group` selects; absent means on neither roster.
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
                // Evaluate only this owner's regular groups. An authenticated
                // empty roster never reaches OrdersGroupPlans.
                for &handle in &country.groups {
                    let roots = get(&self.group_order_roots, || {
                        Ok(table::<tables::OrdersGroupPlans>(self.frame)?
                            .iter()
                            .map(|row| (row.group, row.primary_orders.as_slice()))
                            .collect())
                    })?;
                    let first = roots.get(&handle).ok_or_else(|| {
                        format!("orders group {handle:#x} has no OrdersGroupPlans row")
                    })?;
                    let mut pending: Vec<u32> = first.iter().rev().copied().collect();
                    let mut seen = BTreeSet::new();
                    while let Some(instance) = pending.pop() {
                        if !seen.insert(instance) {
                            continue;
                        }
                        // Order collection reads no order payload
                        // when the group's primary-root count is zero.
                        let instances = get(&self.army_orders, || {
                            Ok(table::<tables::ArmyOrders>(self.frame)?
                                .iter()
                                .map(|row| ((row.owner_group, row.instance), row))
                                .collect())
                        })?;
                        let key = (handle, instance);
                        let order = instances.get(&key).ok_or_else(|| {
                            format!("order {instance} of group {handle:#x} has no ArmyOrders row")
                        })?;
                        if order.order_type == 3 {
                            collected.push(Invasion {
                                key,
                                active: order.active,
                                path: order.path.as_slice(),
                            });
                        }
                        let plans = get(&self.army_order_plans, || {
                            Ok(table::<tables::ArmyOrderPlans>(self.frame)?
                                .iter()
                                .map(|row| ((row.owner_group, row.instance), row))
                                .collect())
                        })?;
                        let plan = plans.get(&key).ok_or_else(|| {
                            format!(
                                "order {instance} of group {handle:#x} has no ArmyOrderPlans row"
                            )
                        })?;
                        pending.extend(plan.successors.iter().rev());
                    }
                }
                Ok(collected)
            })?
            .as_slice()
        } else {
            &[]
        };
        // Preserve the positive-owner absence refusal: an unlinked theatre
        // may hold an invasion that an otherwise empty traversal cannot see.
        if slot != 0
            && orders.is_empty()
            && let Some(reason) = unresolved
        {
            return Err(reason.clone());
        }
        Ok(orders)
    }

    /// An invasion order's preparation, fixed5 (1e5 is fully prepared), as
    /// the invasion trigger computes
    /// it: `trunc(progress / 1e5)` whole hours (an `i32`), times 1e10, over
    /// the required hours ([`required_invasion_hours`]) times 1e5; zero when
    /// the required hours are.
    pub(super) fn invasion_preparation(&self, key: (u64, u32)) -> Result<i64, String> {
        // The index below is built once per world; every evaluation reads the table.
        super::inputs::table(self.frame, replay::rows::TableId::InvasionPreparations);
        let rows = get(&self.invasion_preparations, || {
            Ok(table::<tables::InvasionPreparations>(self.frame)?
                .iter()
                .map(|row| ((row.owner_group, row.instance), row))
                .collect())
        })?;
        let row = rows.get(&key).ok_or_else(|| {
            format!(
                "invasion order {} of group {:#x} has no InvasionPreparations row",
                key.1, key.0
            )
        })?;
        let days = self.invasion_prepare_days.map_err(str::to_owned)?;
        let scale = i64::from(required_invasion_hours(row, days)).wrapping_mul(ONE);
        let whole = i64::from((row.progress / ONE) as i32);
        Ok(if scale == 0 {
            0
        } else if whole.wrapping_mul(ONE) < -0x7fff_ffff_ffff_fffe {
            0xffff_ffff
        } else {
            whole.wrapping_mul(10_000_000_000) / scale
        })
    }

    /// Friendship is false unless both countries own
    /// a state; then use full country visibility:
    /// the same country, else
    /// an active war vetoes, then a shared faction or a direct subject relation
    /// either way.
    pub(super) fn is_ally_with(&self, slot: u32, other: Option<u32>) -> Result<bool, String> {
        super::inputs::query(
            || super::inputs::Key::World(Query::IsAllyWith(slot, other)),
            || self.is_ally_with_unrecorded(slot, other),
        )
    }

    fn is_ally_with_unrecorded(&self, slot: u32, other: Option<u32>) -> Result<bool, String> {
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

    /// A store's value at `key`, `None` when the store does not hold it.
    /// Every complete `ScriptVariables` table holds a row for every scope
    /// the collector walks (the global store, every country, state and
    /// roster character), empty stores included, so a scope without a row
    /// is refused rather than read as empty. Effects write variables at any
    /// hour; the table is read every hour.
    pub(super) fn variable(&self, store: Store, key: &[u8]) -> Result<Option<i64>, String> {
        super::inputs::query(
            || super::inputs::Key::World(Query::Variable(store, key.to_vec())),
            || self.variable_unrecorded(store, key),
        )
    }

    fn variable_unrecorded(&self, store: Store, key: &[u8]) -> Result<Option<i64>, String> {
        // A name stored twice reads its last value.
        Ok(self
            .variable_store(store)?
            .entries
            .iter()
            .rev()
            .find(|entry| entry.name == key)
            .map(|entry| entry.value_raw))
    }

    /// Variable-random initialisation reads only the store's seed.
    pub(super) fn variable_seed(&self, store: Store) -> Result<u32, String> {
        super::inputs::query(
            || super::inputs::Key::World(Query::VariableSeed(store)),
            || self.variable_seed_unrecorded(store),
        )
    }

    fn variable_seed_unrecorded(&self, store: Store) -> Result<u32, String> {
        self.variable_store(store)?
            .seed
            .require("ScriptVariables.seed")?
            .ok_or_else(|| "ScriptVariables.seed capture unavailable".into())
    }

    /// The store's row: rows are in key order, scope kind then id.
    fn variable_store(&self, store: Store) -> Result<&'a ScriptVariablesRow, String> {
        let rows = table::<tables::ScriptVariables>(self.frame)?;
        // The scope kinds `collector::extended::intel` records.
        let (kind, id) = match store {
            Store::Global => (0, 0),
            Store::Country(slot) => (
                1,
                self.raw_country(slot)
                    .ok_or_else(|| format!("country slot {slot} has no Countries row"))?,
            ),
            Store::State(id) => (2, id),
            Store::Character(id) => (3, id),
        };
        rows.binary_search_by_key(&(kind, id), |row| (row.scope_kind, row.scope_id))
            .map(|at| &rows[at])
            .map_err(|_| {
                format!("the script variable store of scope {kind}:{id} has no ScriptVariables row")
            })
    }

    /// The script constants database, or why it is unread.
    pub(super) fn script_constants(&self) -> Result<&'a Constants, &'a str> {
        *self.script_constants
    }

    /// Whether a character with this id is recorded as a live reference.
    pub(super) fn character_exists(&self, id: u32) -> Result<bool, String> {
        super::inputs::query(
            || super::inputs::Key::World(Query::CharacterExists(id)),
            || self.character_exists_unrecorded(id),
        )
    }

    fn character_exists_unrecorded(&self, id: u32) -> Result<bool, String> {
        Ok(self.character(id)?.is_some())
    }

    /// The state a variable value names: its
    /// tagged state encoding, else whole-unit rounding
    /// of the value. The list keeps the game state's state pointer for that
    /// id, null for an id below one, past
    /// the array or in a hole, and a null entry
    /// becomes the null state [`NULL_STATE`]. The recorded `States` table
    /// holds every state (`World::new` refuses an incomplete one), so an id it
    /// lacks is a null entry, unless the content has that state: then the
    /// recording lacks it and the entry is unknown.
    pub(super) fn state_of_value(&self, raw: i64) -> Result<u32, String> {
        super::inputs::query(
            || super::inputs::Key::World(Query::StateOfValue(raw)),
            || self.state_of_value_unrecorded(raw),
        )
    }

    fn state_of_value_unrecorded(&self, raw: i64) -> Result<u32, String> {
        let bits = raw as u64;
        let id = if bits & 0xffff_ffff_f000_0000 == 0xffff_ffff_c000_0000 {
            (bits & 0x0fff_ffff) as i64
        } else {
            fixed_round(raw)
        };
        match u32::try_from(id) {
            Ok(id) if id > 0 && self.state_map().contains_key(&id) => Ok(id),
            Ok(id) if id > 0 && self.state_provinces.contains_key(&id) => {
                Err(format!("state {id} is in the content but not recorded"))
            }
            Ok(id) if id > 0 => match self.table_gap("States") {
                Some(why) => Err(format!("state {id} is not recorded: {why}")),
                None => Ok(NULL_STATE),
            },
            _ => Ok(NULL_STATE),
        }
    }

    /// A recorded unit leader by its reference handle.
    pub(crate) fn unit_leader(&self, reference: u64) -> Result<Option<&'a UnitLeaderRow>, String> {
        super::inputs::query_debug(
            || super::inputs::Key::World(Query::UnitLeader(reference)),
            || self.unit_leader_unrecorded(reference),
        )
    }

    fn unit_leader_unrecorded(&self, reference: u64) -> Result<Option<&'a UnitLeaderRow>, String> {
        let leaders = get(&self.leaders, || {
            Ok(table::<tables::UnitLeaders>(self.frame)?
                .iter()
                .map(|row| {
                    (
                        (u64::from(row.id) << 32) | u64::from(row.reference_type),
                        row,
                    )
                })
                .collect())
        })?;
        Ok(leaders.get(&reference).copied())
    }

    /// A leader's [`LeaderVisibility`], its character and tag spelling read
    /// from the hour's indexes.
    pub(super) fn leader_visibility(&self, row: &UnitLeaderRow) -> LeaderVisibility {
        self.leader_visibility_from(row, |id| self.character(id), |token| self.token(token))
    }

    /// [`Self::leader_visibility`] with the hour's Characters and LexicalTokens
    /// rows binary searched instead of indexed: the decoder validates a
    /// table's keys unique and ascending, so both find the same row.
    pub fn decoded_leader_visibility(&self, row: &UnitLeaderRow) -> LeaderVisibility {
        self.leader_visibility_from(
            row,
            |id| {
                let rows = table::<tables::Characters>(self.frame)?;
                Ok(rows
                    .binary_search_by_key(&id, |row| row.id)
                    .ok()
                    .map(|at| &rows[at]))
            },
            |token| {
                let rows = table::<tables::LexicalTokens>(self.frame)?;
                Ok(rows
                    .binary_search_by_key(&token, |row| row.token)
                    .ok()
                    .map(|at| rows[at].literal.as_slice()))
            },
        )
    }

    /// [`Self::leader_visibility`], `character` reading a Characters row by
    /// id and `literal` a LexicalTokens literal by token.
    fn leader_visibility_from(
        &self,
        row: &UnitLeaderRow,
        character: impl FnOnce(u32) -> Result<Option<&'a CharacterRow>, String>,
        literal: impl FnOnce(u32) -> Result<Option<&'a [u8]>, String>,
    ) -> LeaderVisibility {
        if row.visibility_choice == 0 && row.template_visibility == 0 {
            return LeaderVisibility::Decided(Eval::True);
        }
        let template = character(row.character_id).and_then(|character| match character {
            Some(character) => Ok(recorded_template(character)),
            None => Err(format!(
                "unit leader {}'s character {} is not recorded",
                row.id, row.character_id
            )),
        });
        self.template_visibility(row, template, literal)
    }

    /// The [`LeaderVisibility`] of a leader whose character's recorded
    /// template is `template`, `literal` spelling its tag token.
    pub(super) fn template_visibility(
        &self,
        row: &UnitLeaderRow,
        template: Result<Option<RecordedTemplate>, String>,
        literal: impl FnOnce(u32) -> Result<Option<&'a [u8]>, String>,
    ) -> LeaderVisibility {
        match template.and_then(|template| self.template_role(row, template, literal)) {
            Ok(Some((_, visible))) if visible.is_empty() => LeaderVisibility::Unconditioned,
            Ok(Some((index, _))) => LeaderVisibility::Conditioned(index),
            // A stored role choice needs a template role, so a template without one did not
            // build this leader.
            Ok(None) if row.visibility_choice == 0 => LeaderVisibility::Decided(Eval::True),
            Ok(None) => LeaderVisibility::Decided(Eval::unknown(format!(
                "unit leader {} has its own role choice, but its character's template has no unit leader role",
                row.id
            ))),
            Err(reason) => LeaderVisibility::Decided(Eval::unknown(reason)),
        }
    }

    /// The unit leader role of a recorded template, with its index in
    /// [`LeaderRoles`]: the one content template of its tag (folded as the
    /// lexer folds) that its file and line name.
    fn template_role(
        &self,
        row: &UnitLeaderRow,
        template: Option<RecordedTemplate>,
        literal: impl FnOnce(u32) -> Result<Option<&'a [u8]>, String>,
    ) -> Result<Option<(usize, &'a installed_content::triggers::Triggers)>, String> {
        let template = template.ok_or_else(|| {
            format!(
                "unit leader {} has a visibility input, but its character has no template",
                row.id
            )
        })?;
        if template.generated {
            return Err(format!(
                "unit leader {}'s character template was made by generate_character, which is not read",
                row.id
            ));
        }
        let roles = self.leader_roles.map_err(str::to_owned)?;
        let tag = literal(template.tag)?
            .and_then(|literal| std::str::from_utf8(literal).ok())
            .ok_or_else(|| {
                format!(
                    "character template tag token {} has no recorded text",
                    template.tag
                )
            })?;
        // The lexer fold (`fold_token`), compared without allocating.
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
                .map_err(|reason| format!("character template {} is refused: {reason}", where_())),
            (None, _) => Err(format!("no content character template is {}", where_())),
            (Some(_), Some(_)) => Err(format!(
                "several content character templates are {}",
                where_()
            )),
        }
    }

    /// The `visible` conjunction of a [`LeaderVisibility::Conditioned`] read
    /// with this world's content.
    pub(super) fn role_visible(&self, index: usize) -> &'a installed_content::triggers::Triggers {
        match self.leader_roles.map(|roles| &roles.templates[index].role) {
            Ok(Ok(Some(visible))) => visible,
            _ => unreachable!("leader role {index} is no content role"),
        }
    }

    /// The country's parties (PoliticalParties, by ideology ID) and their
    /// leaders (PartyLeaders, each party's leaders in native order); a party
    /// without leaders is listed with none.
    pub(crate) fn party_leaders(&self, slot: u32) -> Result<Vec<PartyLeaders>, String> {
        super::inputs::query_debug(
            || super::inputs::Key::World(Query::PartyLeaders(slot)),
            || self.party_leaders_unrecorded(slot),
        )
    }

    fn party_leaders_unrecorded(&self, slot: u32) -> Result<Vec<PartyLeaders>, String> {
        let party_rows = table::<tables::PoliticalParties>(self.frame)?;
        let Some(by_party) = self.country_rows(&self.party_rows, party_rows, |row| row.country)
        else {
            return self.party_leaders_scanned(slot);
        };
        if let Some((_, raw)) = by_party.first_unlinked() {
            return Err(self.unlinked("PoliticalParties", raw));
        }
        let raws = self.slot_links(slot);
        let mut parties = BTreeMap::new();
        for &raw in raws {
            for row in &party_rows[by_party.of(raw)] {
                parties.insert(
                    row.id,
                    PartyLeaders {
                        ruling: row.is_ruling != 0,
                        leaders: Vec::new(),
                    },
                );
            }
        }
        let leader_rows = table::<tables::PartyLeaders>(self.frame)?;
        let Some(by_leader) =
            self.country_rows(&self.party_leader_rows, leader_rows, |row| row.country)
        else {
            return self.party_leaders_scanned(slot);
        };
        // The first row in table order that fails decides, as a scan finds it.
        let unlinked = by_leader.first_unlinked();
        for &raw in raws {
            let range = by_leader.of(raw);
            for (at, row) in range.clone().zip(&leader_rows[range]) {
                if let Some((first, raw)) = unlinked
                    && first < at
                {
                    return Err(self.unlinked("PartyLeaders", raw));
                }
                parties
                    .get_mut(&row.party)
                    .ok_or_else(|| {
                        format!(
                            "PartyLeaders party {} has no PoliticalParties row",
                            row.party
                        )
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
            return Err(self.unlinked("PartyLeaders", raw));
        }
        Ok(parties.into_values().collect())
    }

    /// [`Self::party_leaders_unrecorded`] scanning every row, for rows that
    /// do not ascend by country.
    fn party_leaders_scanned(&self, slot: u32) -> Result<Vec<PartyLeaders>, String> {
        let mut parties = BTreeMap::new();
        for row in table::<tables::PoliticalParties>(self.frame)? {
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
                None => return Err(self.unlinked("PoliticalParties", row.country)),
            }
        }
        for row in table::<tables::PartyLeaders>(self.frame)? {
            match self.slot(row.country) {
                Some(country) if country == slot => parties
                    .get_mut(&row.party)
                    .ok_or_else(|| {
                        format!(
                            "PartyLeaders party {} has no PoliticalParties row",
                            row.party
                        )
                    })?
                    .leaders
                    .push(PartyLeader {
                        character: row.character,
                        ordinal: row.ordinal,
                        role: row.role,
                        id: row.leader_id,
                    }),
                Some(_) => {}
                None => return Err(self.unlinked("PartyLeaders", row.country)),
            }
        }
        Ok(parties.into_values().collect())
    }

    /// The country's active characters,
    /// sorted by their
    /// character's token), by character id in native order (CountryCharacters,
    /// keyed by the raw country; `None` for a null record).
    pub(crate) fn country_characters(&self, slot: u32) -> Result<Vec<Option<u32>>, String> {
        super::inputs::query(
            || super::inputs::Key::World(Query::CountryCharacters(slot)),
            || self.country_characters_unrecorded(slot),
        )
    }

    fn country_characters_unrecorded(&self, slot: u32) -> Result<Vec<Option<u32>>, String> {
        let rows = table::<tables::CountryCharacters>(self.frame)?;
        let Some(countries) = self.country_rows(&self.character_rows, rows, |row| row.country)
        else {
            return self.country_characters_scanned(slot);
        };
        if let Some((_, raw)) = countries.first_unlinked() {
            return Err(self.unlinked("CountryCharacters", raw));
        }
        Ok(self
            .slot_links(slot)
            .iter()
            .flat_map(|&raw| rows[countries.of(raw)].iter().map(|row| row.character))
            .collect())
    }

    /// [`Self::country_characters_unrecorded`] scanning every row, for rows
    /// that do not ascend by country.
    fn country_characters_scanned(&self, slot: u32) -> Result<Vec<Option<u32>>, String> {
        let mut out = Vec::new();
        for row in table::<tables::CountryCharacters>(self.frame)? {
            match self.slot(row.country) {
                Some(country) if country == slot => out.push(row.character),
                Some(_) => {}
                None => return Err(self.unlinked("CountryCharacters", row.country)),
            }
        }
        Ok(out)
    }

    /// Search the ruling party's first leader's trait pointers.
    /// PartyLeaders carries its role-map key even after the current ideology changes.
    pub(super) fn leader_has_trait(&self, slot: u32, name: &str) -> Result<bool, String> {
        super::inputs::query(
            || super::inputs::Key::World(Query::LeaderHasTrait(slot, name.to_owned())),
            || self.leader_has_trait_unrecorded(slot, name),
        )
    }

    fn leader_has_trait_unrecorded(&self, slot: u32, name: &str) -> Result<bool, String> {
        let (_, traits) = self
            .character_traits
            .map_err(|why| format!("the country-leader traits are unavailable: {why}"))?;
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
            return Err(format!("country {slot} has multiple ruling parties"));
        }
        let Some(leader) = party.leaders.first() else {
            return Ok(false);
        };
        if leader.ordinal != 0 {
            return Err(format!(
                "country {slot}'s party leaders do not start at zero"
            ));
        }
        let (Some(character), Some(role)) = (leader.character, leader.role) else {
            return Err(format!(
                "country {slot}'s first leader has no character role identity"
            ));
        };
        // Empty arrays have no CharacterRoleTraits row; the complete role census
        // distinguishes that from a missing leader role.
        if !table::<tables::CountryLeaderRoles>(self.frame)?
            .iter()
            .any(|row| row.character == character && row.ideology == role)
        {
            return Err(format!(
                "party leader {character}/{role} has no CountryLeaderRoles row"
            ));
        }
        character_scope::holds_country_leader_trait(
            table::<tables::CharacterRoleTraits>(self.frame)?
                .iter()
                .filter(|row| {
                    row.character == character
                        && row.kind == replay::rows::CharacterRoleTraitRow::COUNTRY_LEADER
                        && row.role == role as u32
                }),
            traits,
            name,
            ordinal,
            self.token_of(name.as_bytes())?,
        )
    }

    /// The country's civil-war countries (raw tag
    /// references; CountryCivilWars, keyed by the raw country).
    pub(super) fn civil_war_countries(&self, raw: u32) -> Result<Vec<u32>, String> {
        super::inputs::query(
            || super::inputs::Key::World(Query::CivilWarCountries(raw)),
            || self.civil_war_countries_unrecorded(raw),
        )
    }

    fn civil_war_countries_unrecorded(&self, raw: u32) -> Result<Vec<u32>, String> {
        Ok(table::<tables::CountryCivilWars>(self.frame)?
            .iter()
            .filter(|row| row.country == raw)
            .map(|row| row.target)
            .collect())
    }

    /// Whether an idea token is a registered advisor token
    /// (AdvisorTokens).
    pub(super) fn advisor_token(&self, name: &[u8]) -> Result<bool, String> {
        super::inputs::query(
            || super::inputs::Key::World(Query::AdvisorToken(name.to_vec())),
            || self.advisor_token_unrecorded(name),
        )
    }

    fn advisor_token_unrecorded(&self, name: &[u8]) -> Result<bool, String> {
        Ok(table::<tables::AdvisorTokens>(self.frame)?
            .iter()
            .any(|row| row.token == name))
    }

    /// The idea tokens of a faction's advisors (FactionAdvisors, keyed
    /// by the faction's full reference).
    pub(super) fn faction_advisors(&self, faction: u64) -> Result<Vec<Vec<u8>>, String> {
        super::inputs::query(
            || super::inputs::Key::World(Query::FactionAdvisors(faction)),
            || self.faction_advisors_unrecorded(faction),
        )
    }

    fn faction_advisors_unrecorded(&self, faction: u64) -> Result<Vec<Vec<u8>>, String> {
        Ok(table::<tables::FactionAdvisors>(self.frame)?
            .iter()
            .filter(|row| row.faction == faction)
            .map(|row| row.token.clone())
            .collect())
    }

    /// A country of its own civil-war list exists, or an
    /// existing enemy holds it in its own list.
    pub(super) fn has_civil_war(&self, slot: u32) -> Result<bool, String> {
        super::inputs::query(
            || super::inputs::Key::World(Query::HasCivilWar(slot)),
            || self.has_civil_war_unrecorded(slot),
        )
    }

    fn has_civil_war_unrecorded(&self, slot: u32) -> Result<bool, String> {
        let raw = self
            .raw_country(slot)
            .ok_or_else(|| format!("country slot {slot} has no Countries row"))?;
        for other in self.civil_war_countries(raw)? {
            let other = self
                .slot(other)
                .ok_or_else(|| self.unlinked("civil war", other))?;
            if self.exists(other)? {
                return Ok(true);
            }
        }
        for enemy in self.enemies(slot)? {
            if !self.exists(enemy)? {
                continue;
            }
            let enemy_raw = self
                .raw_country(enemy)
                .ok_or_else(|| format!("country slot {enemy} has no Countries row"))?;
            for listed in self.civil_war_countries(enemy_raw)? {
                if self.slot(listed) == Some(slot) {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    /// The country's enemy vector: as read where
    /// `CountryWars` read it, else the other endpoint of each active war
    /// relation, which wartime-allies rebuilding keeps it to.
    pub(super) fn enemies(&self, slot: u32) -> Result<Vec<u32>, String> {
        super::inputs::query(
            || super::inputs::Key::World(Query::Enemies(slot)),
            || self.enemies_unrecorded(slot),
        )
    }

    fn enemies_unrecorded(&self, slot: u32) -> Result<Vec<u32>, String> {
        for row in table::<tables::CountryWars>(self.frame)? {
            if self.slot(row.country) == Some(slot) && row.enemies_read != 0 {
                return row
                    .enemies
                    .iter()
                    .map(|&raw| {
                        self.slot(raw)
                            .ok_or_else(|| self.unlinked("CountryWars", raw))
                    })
                    .collect();
            }
        }
        let mut out = Vec::new();
        for row in table::<tables::WarRelations>(self.frame)?
            .iter()
            .filter(|row| ongoing(row.start_hour, row.end_hour, self.hour))
        {
            let (first, second) = (self.slot(row.first_country), self.slot(row.second_country));
            match (first, second) {
                (Some(a), Some(b)) if a == slot => out.push(b),
                (Some(a), Some(b)) if b == slot => out.push(a),
                (Some(_), Some(_)) => {}
                _ => return Err(self.unlinked("WarRelations", row.first_country)),
            }
        }
        Ok(out)
    }

    /// An advisor token holds when the country has hired its advisor
    /// (the HiredAdvisors rows) or its faction lists it; any other
    /// name holds when the country's active ideas
    /// (ActiveIdeas) hold the idea it names.
    pub(super) fn has_idea(&self, slot: u32, name: &str) -> Result<bool, String> {
        super::inputs::query(
            || super::inputs::Key::World(Query::HasIdea(slot, name.to_owned())),
            || self.has_idea_unrecorded(slot, name),
        )
    }

    fn has_idea_unrecorded(&self, slot: u32, name: &str) -> Result<bool, String> {
        let name = name.as_bytes();
        if self.advisor_token(name)? {
            for row in table::<tables::HiredAdvisors>(self.frame)? {
                if self.slot(row.country) == Some(slot) && row.idea == name {
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

    /// Whether the country's active ideas hold `name`: the first of its
    /// matching rows and the rows whose country does not resolve, in table
    /// order, decides.
    fn active_idea(&self, slot: u32, name: &[u8]) -> Result<bool, String> {
        let rows = table::<tables::ActiveIdeas>(self.frame)?;
        let Some(countries) = self.country_rows(&self.idea_rows, rows, |row| row.country) else {
            return self.active_idea_scanned(slot, name);
        };
        let found = self
            .slot_links(slot)
            .iter()
            .filter_map(|&raw| {
                let range = countries.of(raw);
                let start = range.start;
                rows[range]
                    .iter()
                    .position(|row| row.token == name)
                    .map(|at| start + at)
            })
            .min();
        match (found, countries.first_unlinked()) {
            (Some(at), Some((first, raw))) if first < at => Err(self.unlinked("ActiveIdeas", raw)),
            (Some(_), _) => Ok(true),
            (None, Some((_, raw))) => Err(self.unlinked("ActiveIdeas", raw)),
            (None, None) => Ok(false),
        }
    }

    /// [`Self::active_idea`] scanning every row, for rows that do not
    /// ascend by country.
    fn active_idea_scanned(&self, slot: u32, name: &[u8]) -> Result<bool, String> {
        for row in table::<tables::ActiveIdeas>(self.frame)? {
            match self.slot(row.country) {
                Some(country) if country == slot && row.token == name => return Ok(true),
                Some(_) => {}
                None => return Err(self.unlinked("ActiveIdeas", row.country)),
            }
        }
        Ok(false)
    }

    /// The country's controlled-state count
    /// (CountryCounterInputs.controlled_state_count).
    pub(super) fn controlled_state_count(&self, slot: u32) -> Result<i64, String> {
        super::inputs::query(
            || super::inputs::Key::World(Query::ControlledStateCount(slot)),
            || self.controlled_state_count_unrecorded(slot),
        )
    }

    fn controlled_state_count_unrecorded(&self, slot: u32) -> Result<i64, String> {
        let raw = self
            .raw_country(slot)
            .ok_or_else(|| format!("country slot {slot} has no Countries row"))?;
        table::<tables::CountryCounterInputs>(self.frame)?
            .iter()
            .find(|row| row.country == raw)
            .map(|row| i64::from(row.controlled_state_count))
            .ok_or_else(|| format!("country {raw} has no CountryCounterInputs row"))
    }

    /// The recorded opinion's signed dword times fixed5.
    pub(super) fn opinion(&self, owner: u32, target: Option<u32>) -> Result<i64, String> {
        super::inputs::query(
            || super::inputs::Key::World(Query::Opinion(owner, target)),
            || self.opinion_unrecorded(owner, target),
        )
    }

    fn opinion_unrecorded(&self, owner: u32, target: Option<u32>) -> Result<i64, String> {
        let Some(target) = target.filter(|&slot| slot != 0) else {
            return Err(refused(
                "opinion: country zero reads reserved relation slot zero, which RelationOpinions does not record".into(),
            ));
        };
        for slot in [owner, target] {
            self.raw_country(slot)
                .ok_or_else(|| format!("country slot {slot} has no Countries row"))?;
        }
        let (opinions, unresolved) = get(&self.opinions, || {
            table::<tables::CountryLinks>(self.frame)?;
            let mut out = BTreeMap::new();
            let mut unresolved = None;
            for row in table::<tables::RelationOpinions>(self.frame)? {
                match (self.slot(row.owner_country), self.slot(row.target_country)) {
                    (Some(owner), Some(target)) => {
                        out.insert((owner, target), row.opinion);
                    }
                    (None, _) => {
                        unresolved = Some(self.unlinked("RelationOpinions", row.owner_country))
                    }
                    (_, None) => {
                        unresolved = Some(self.unlinked("RelationOpinions", row.target_country))
                    }
                }
            }
            Ok((out, unresolved))
        })?;
        // The serializer and collector retain only nonzero opinions.
        Ok(
            i64::from(linked(opinions.get(&(owner, target)).copied(), unresolved)?.unwrap_or(0))
                * ONE,
        )
    }

    /// Scan the country's owned states and compare their controller.
    /// Ownership changes maintain that roster;
    /// its insertion rule excludes states without provinces.
    fn owned_controlled_state_count(&self, slot: u32) -> Result<i64, String> {
        self.raw_country(slot)
            .ok_or_else(|| format!("country slot {slot} has no Countries row"))?;
        let counts = get(&self.owned_controlled_states, || {
            table::<tables::CountryLinks>(self.frame)?;
            table::<tables::Provinces>(self.frame)?;
            let members = self.state_members()?;
            let mut out: BTreeMap<u32, i32> = BTreeMap::new();
            for row in table::<tables::States>(self.frame)? {
                if row.owner == 0 || !members.contains_key(&row.id) {
                    continue;
                }
                let owner = self
                    .slot(row.owner)
                    .ok_or_else(|| self.unlinked("States", row.owner))?;
                if row.controller == 0 {
                    continue;
                }
                let controller = self
                    .slot(row.controller)
                    .ok_or_else(|| self.unlinked("States", row.controller))?;
                if owner == controller {
                    let count = out.entry(owner).or_default();
                    *count = count.wrapping_add(1);
                }
            }
            Ok(out)
        })?;
        Ok(i64::from(*counts.get(&slot).unwrap_or(&0)))
    }

    /// Native dynamic-variable callbacks run before the persistent keys.
    pub(super) fn builtin_variable(
        &self,
        store: Store,
        name: &[u8],
    ) -> Result<Option<i64>, String> {
        super::inputs::query(
            || super::inputs::Key::World(Query::BuiltinVariable(store, name.to_vec())),
            || self.builtin_variable_unrecorded(store, name),
        )
    }

    fn builtin_variable_unrecorded(
        &self,
        store: Store,
        name: &[u8],
    ) -> Result<Option<i64>, String> {
        match (store, name) {
            (Store::Country(slot), b"num_owned_states") => Ok(Some(
                i64::from(self.owned_state_count(slot)?).wrapping_mul(ONE),
            )),
            (Store::Country(slot), b"num_owned_controlled_states") => {
                Ok(Some(self.owned_controlled_state_count(slot)? * ONE))
            }
            (Store::Country(slot), b"num_of_controlled_factories") => {
                let raw = self
                    .raw_country(slot)
                    .ok_or_else(|| format!("country slot {slot} has no Countries row"))?;
                let row = table::<tables::CountryFactories>(self.frame)?
                    .iter()
                    .find(|row| row.country == raw)
                    .ok_or_else(|| {
                        format!(
                            "num_of_controlled_factories: country {raw} has no CountryFactories row"
                        )
                    })?;
                // The value wraps in i32; its builtin callback
                // sign-extends the result, then multiplies by 100000.
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
        names: &'a OnceLock<Result<installed_content::localisation::Localisation, String>>,
        layers: &'a installed_content::ResolvedLayers,
        language: &'a str,
    ) -> Self {
        self.template_names = Some((names, layers, language));
        self
    }

    /// Division template name lookup compares display-name
    /// bytes exactly.
    pub(crate) fn has_template(&self, slot: u32, wanted: &[u8]) -> Result<bool, String> {
        super::inputs::query(
            || super::inputs::Key::World(Query::HasTemplate(slot, wanted.to_vec())),
            || self.has_template_unrecorded(slot, wanted),
        )
    }

    fn has_template_unrecorded(&self, slot: u32, wanted: &[u8]) -> Result<bool, String> {
        let rows = table::<tables::DivisionTemplates>(self.frame)?;
        let mut unknown = None;
        for row in rows {
            let Some(owner) = self.slot(row.owner) else {
                unknown = Some(self.unlinked("DivisionTemplates", row.owner));
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
            let name = (|| {
                let name_key = row.name_key.require("DivisionTemplates.name_key")?;
                if name_key.is_empty() {
                    return Ok(String::new());
                }
                let (names, layers, language) = self
                    .template_names
                    .ok_or("template display-name localisation is unavailable")?;
                let names = names
                    .get_or_init(|| {
                        installed_content::localisation::read(layers, language).map_err(|e| e.to_string())
                    })
                    .as_ref()
                    .map_err(Clone::clone)?;
                let icons = table::<tables::DivisionTemplateIcons>(self.frame)?;
                let icon = icons
                    .iter()
                    .find(|icon| icon.template == row.template)
                    .ok_or_else(|| {
                        format!(
                            "template {} has no DivisionTemplateIcons country",
                            row.template
                        )
                    })?;
                let key =
                    std::str::from_utf8(name_key).map_err(|_| "template name_key is not UTF-8")?;
                let country = match u32::try_from(icon.country).ok().filter(|raw| *raw > 0) {
                    Some(raw) => Some(
                        self.slot(raw)
                            .ok_or_else(|| self.unlinked("DivisionTemplateIcons", raw))?,
                    ),
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
                names.localise(key.as_bytes()).map_err(|e| e.to_string())
            })();
            match name {
                Ok(name) if name.as_bytes() == wanted => return Ok(true),
                Ok(_) => {}
                Err(why) => unknown = Some(why),
            }
        }
        unknown.map_or(Ok(false), Err)
    }

    /// A recorded character by its id.
    pub(crate) fn character(&self, id: u32) -> Result<Option<&'a CharacterRow>, String> {
        super::inputs::query_debug(
            || super::inputs::Key::World(Query::Character(id)),
            || self.character_unrecorded(id),
        )
    }

    fn character_unrecorded(&self, id: u32) -> Result<Option<&'a CharacterRow>, String> {
        let characters = get(&self.characters, || {
            Ok(table::<tables::Characters>(self.frame)?
                .iter()
                .map(|character| (character.id, character))
                .collect())
        })?;
        Ok(characters.get(&id).copied())
    }

    /// A canonical country's `CountryAppearance` row.
    pub(crate) fn appearance(&self, slot: u32) -> Result<&'a CountryAppearanceRow, String> {
        super::inputs::query_debug(
            || super::inputs::Key::World(Query::Appearance(slot)),
            || self.appearance_unrecorded(slot),
        )
    }

    fn appearance_unrecorded(&self, slot: u32) -> Result<&'a CountryAppearanceRow, String> {
        let (rows, unresolved) = get(&self.appearance, || {
            let mut out = BTreeMap::new();
            let mut unresolved = None;
            for row in table::<tables::CountryAppearance>(self.frame)? {
                match self.slot(row.country) {
                    Some(slot) => {
                        out.insert(slot, row);
                    }
                    None => unresolved = Some(self.unlinked("CountryAppearance", row.country)),
                }
            }
            Ok((out, unresolved))
        })?;
        linked(rows.get(&slot).copied(), unresolved)?
            .ok_or_else(|| format!("country slot {slot} has no CountryAppearance row"))
    }

    /// The core-owner trigger tests one of the state's cores (its core vector), by raw id or
    /// canonical slot. `StateCores` equals the game's at every hour (its writers are hooked;
    /// unmarked states are read once a day).
    pub(super) fn is_core_of(&self, state: u32, country: Option<u32>) -> Result<Eval, String> {
        super::inputs::query_debug(
            || super::inputs::Key::World(Query::IsCoreOf(state, country)),
            || self.is_core_of_unrecorded(state, country),
        )
    }

    fn is_core_of_unrecorded(&self, state: u32, country: Option<u32>) -> Result<Eval, String> {
        let cores = get(&self.cores, || {
            Ok(table::<tables::StateCores>(self.frame)?
                .iter()
                .map(|row| (row.state, row.cores.as_slice()))
                .collect())
        })?;
        let cores = cores
            .get(&state)
            .ok_or_else(|| format!("state {state} has no StateCores row"))?;
        Ok(self.lists(cores, country, "StateCores"))
    }

    /// The claimant trigger tests one of the state's claims (its claim vector) as the core-owner
    /// trigger tests cores: by raw id or canonical slot. Claims are hooked as cores are
    /// (`StateCores.claims`).
    pub(super) fn is_claimed_by(&self, state: u32, country: Option<u32>) -> Result<Eval, String> {
        super::inputs::query_debug(
            || super::inputs::Key::World(Query::IsClaimedBy(state, country)),
            || self.is_claimed_by_unrecorded(state, country),
        )
    }

    fn is_claimed_by_unrecorded(&self, state: u32, country: Option<u32>) -> Result<Eval, String> {
        let claims = get(&self.claims, || {
            Ok(table::<tables::StateCores>(self.frame)?
                .iter()
                .map(|row| match &row.claims {
                    Recorded::Value(claims) => (row.state, Some(claims.as_slice())),
                    Recorded::NotRecorded => (row.state, None),
                })
                .collect())
        })?;
        let claims = claims
            .get(&state)
            .ok_or_else(|| format!("state {state} has no StateCores row"))?;
        let Some(claims) = claims else {
            return Ok(Eval::unknown(
                "StateCores.claims is not recorded".to_owned(),
            ));
        };
        Ok(self.lists(claims, country, "StateCores.claims"))
    }

    /// Whether `country` is one of a state's raw country references, by
    /// canonical slot; an absent country is never listed.
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

    /// The canonical country's own raw reference (`Countries.id`).
    pub fn raw_country(&self, slot: u32) -> Option<u32> {
        super::inputs::query(
            || super::inputs::Key::World(Query::RawCountry(slot)),
            || self.raw_country_unrecorded(slot),
        )
    }

    fn raw_country_unrecorded(&self, slot: u32) -> Option<u32> {
        self.raw_ids.get(&slot).copied()
    }

    pub(super) fn nukes(&self, slot: u32) -> Result<i64, String> {
        super::inputs::query(
            || super::inputs::Key::World(Query::Nukes(slot)),
            || self.nukes_unrecorded(slot),
        )
    }

    fn nukes_unrecorded(&self, slot: u32) -> Result<i64, String> {
        // Every country has one row per nuke type.
        let row = table::<tables::Nukes>(self.frame)?
            .iter()
            .find(|row| row.nuke_type == 0 && self.slot(row.country) == Some(slot))
            .ok_or_else(|| format!("country slot {slot} has no Nukes row"))?;
        let raw = row.amount_raw;
        Ok(
            if (raw as u64).wrapping_add(0x7fff_ffff_ffff_fffe) < 0xffff_ffff_ffff_fffd {
                raw.wrapping_mul(100_000) / 10_000_000_000_000
            } else {
                0xa7c5
            },
        )
    }
}

/// Completed technologies by (slot, first definition name); slots with a
/// completed row whose definition is not recorded, and whether a row's
/// country did not resolve, so a missing name is never read as "not researched".
#[derive(Default)]
struct Techs {
    researched: BTreeSet<(u32, Vec<u8>)>,
    unnamed: BTreeSet<u32>,
    unresolved: bool,
}

/// Required invasion preparation hours over the
/// recorded operands, with wrapping arithmetic: `(i32)(A * B / 1e10) * 24`,
/// where `A = 1e5 + country_term + relation_term + relation_bonus` plus
/// `leader_term` when it applies, and
/// `B = NAVAL_INVASION_PREPARE_DAYS * 1e5 + days_term` (the define read as
/// an `i32`). Absent terms add nothing.
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

/// A fixed5 value to the nearest whole
/// number, halves away from zero, with C's truncating division.
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
        World::token_of(self, name)
    }
}

fn get<T>(cell: &Index<T>, build: impl FnOnce() -> Result<T, String>) -> Result<&T, String> {
    cell.get_or_init(build).as_ref().map_err(Clone::clone)
}

#[path = "world/dependence.rs"]
mod dependence;

#[cfg(test)]
#[path = "world/lookup_tests.rs"]
mod lookup_tests;

#[cfg(test)]
#[path = "world/zero_countries_tests.rs"]
mod zero_countries_tests;
