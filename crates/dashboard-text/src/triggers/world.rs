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
#[derive(Clone, Copy)]
pub(super) struct Invasion<'a> {
    pub key: (u64, u32),
    pub active: u8,
    pub path: &'a [u32],
}
/// One store's seed row and variable values by exact key bytes.
type VariableStore<'a> = (&'a ScriptVariablesRow, BTreeMap<&'a [u8], i64>);
/// Each store's variables by (scope kind, id), then by key.
type Variables<'a> = BTreeMap<(u8, u32), VariableStore<'a>>;
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
    pub hour: Hour,
    /// Which of the tables the indexes below are built from is not complete
    /// this hour, and why ([`World::read`]); the triggers that read them are
    /// then unknown.
    core_gaps: Vec<(&'static str, String)>,
    /// Raw country reference to canonical slot.
    pub links: BTreeMap<u32, u32>,
    tags: BTreeMap<u32, &'a [u8]>,
    /// Canonical slot to the country's own raw id, as script flags key it.
    raw_ids: BTreeMap<u32, u32>,
    pub provinces: BTreeMap<u32, &'a ProvinceRow>,
    pub states: BTreeMap<u32, &'a StateRow>,
    /// Content: every state's provinces in its own order, and impassability.
    pub state_provinces: PreparedOperand<'a, &'a BTreeMap<u32, Vec<u32>>>,
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
    pub(super) character_tables: character_scope::Tables<'a>,
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
    /// Each store's script variables (`ScriptVariables`) by scope kind and
    /// id, by their exact key.
    variables: Index<Variables<'a>>,
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
            provinces: provinces.iter().map(|row| (row.id, row)).collect(),
            states: states.iter().map(|row| (row.id, row)).collect(),
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
            character_tables: character_scope::Tables::read(frame),
            aliases: content.tag_aliases,
            recorded: OnceLock::new(),
            names: OnceLock::new(),
            literals: OnceLock::new(),
            factions: OnceLock::new(),
            wars: OnceLock::new(),
            overlords: OnceLock::new(),
            focuses: OnceLock::new(),
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
            variables: OnceLock::new(),
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
        self.core_gaps.first().map(|(_, why)| why.clone())
    }

    /// Why one core table (`CountryLinks`, `Countries`, `Provinces` or
    /// `States`) is not complete this hour, if it is not.
    pub(super) fn table_gap(&self, table: &str) -> Option<String> {
        self.core_gaps
            .iter()
            .find(|(name, _)| *name == table)
            .map(|(_, why)| why.clone())
    }

    /// The hour's scope facts ([`RecordedScopes`]), read on first use.
    pub fn recorded(&self) -> &RecordedScopes {
        self.recorded
            .get_or_init(|| RecordedScopes::read(self.frame, Arc::clone(self.aliases)))
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
        crate::air_wing_names::raid_name(self.frame, reference)
    }

    pub(crate) fn industrial_organisation(
        &self,
        reference: u64,
    ) -> Result<&'a replay::rows::IndustrialOrganisationRow, String> {
        table::<tables::IndustrialOrganisations>(self.frame)?
            .iter()
            .find(|row| row.organisation == reference)
            .ok_or_else(|| format!("IndustrialOrganisations lacks organisation {reference:#x}"))
    }

    pub fn slot(&self, raw: u32) -> Option<u32> {
        self.links.get(&raw).copied()
    }

    pub fn tag(&self, slot: u32) -> Option<&'a [u8]> {
        self.tags.get(&slot).copied()
    }

    /// A recorded state's raw owner and controller.
    pub fn state_row(&self, state: u32) -> Option<(u32, u32)> {
        {
            self.states
                .get(&state)
                .map(|row| (row.owner, row.controller))
        }
    }

    /// A recorded province's raw owner and controller.
    pub fn province_row(&self, province: u32) -> Option<(u32, u32)> {
        {
            self.provinces
                .get(&province)
                .map(|row| (row.owner, row.controller))
        }
    }

    pub fn state_controller(&self, state: u32) -> Option<u32> {
        self.slot(self.states.get(&state)?.controller)
    }

    pub fn province_controller(&self, province: u32) -> Option<u32> {
        self.slot(self.provinces.get(&province)?.controller)
    }

    /// A lexer token's literal.
    pub fn token(&self, token: u32) -> Result<Option<&'a [u8]>, String> {
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
        match self.names()?.get(&name.to_ascii_lowercase()) {
            None => Ok(None),
            Some(Ok(token)) => Ok(Some(*token)),
            Some(Err(reason)) => Err(reason.clone()),
        }
    }

    pub(super) fn names(&self) -> Result<&Folds, String> {
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
        format!("a {table} row's country {raw} has no CountryLinks row")
    }

    /// The country's faction, if any.
    pub fn faction(&self, slot: u32) -> Result<Option<u64>, String> {
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
        Ok(matches!((self.faction(a)?, self.faction(b)?), (Some(x), Some(y)) if x == y))
    }

    pub fn at_war(&self, a: u32, b: u32) -> Result<bool, String> {
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
        installed_content::country_names::original_or_own_reference(self.frame, slot)
    }

    /// Country-tag equality compares raw words first, then their canonical
    /// indices when both differ and neither is zero. Missing mappings refuse.
    pub(crate) fn same_tag(&self, a: i32, b: i32) -> Result<bool, String> {
        if a == b {
            return Ok(true);
        }
        if a == 0 || b == 0 {
            return Ok(false);
        }
        Ok(self.recorded().raw_slot(a as u32)? == self.recorded().raw_slot(b as u32)?)
    }

    pub(super) fn has_focus(&self, slot: u32, name: &[u8]) -> Result<bool, String> {
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
        let parties = self.political_parties()?;
        linked(parties.ruling.get(&slot).copied(), &parties.unresolved)
    }

    pub(crate) fn named_ideology(&self, token: &[u8]) -> Result<(u32, &'a [u8]), String> {
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
        Ok(self.political_parties()?.ids.get(&id).copied())
    }

    pub(super) fn scoped_ruling_ideology(&self, slot: u32) -> Result<(u32, &'a [u8]), String> {
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
        self.state_resource_amount(state, resource, self.state_controller(state))
    }

    /// [`Self::state_resource`] with the controller already resolved.
    pub(super) fn state_resource_amount(
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
        get(&self.state_members, || {
            let mut out: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
            for (&id, row) in &self.provinces {
                out.entry(row.state).or_default().push(id);
            }
            Ok(out)
        })
    }

    /// A state's provinces in their native order: the content's, which the
    /// runtime list keeps while its membership equals the content's.
    pub fn state_order(&self, state: u32) -> Result<&'a [u32], String> {
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
        self.owned_state_count(slot).map(|count| count > 0)
    }

    /// Stored signed count, used by the owned-state builtin.
    pub(crate) fn owned_state_count(&self, slot: u32) -> Result<i32, String> {
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
        let (_, entries) = self.variable_store(store)?;
        Ok(entries.get(key).copied())
    }

    /// Variable-random initialisation reads only the store's seed.
    pub(super) fn variable_seed(&self, store: Store) -> Result<u32, String> {
        let (row, _) = self.variable_store(store)?;
        row.seed
            .require("ScriptVariables.seed")?
            .ok_or_else(|| "ScriptVariables.seed capture unavailable".into())
    }

    fn variable_store(&self, store: Store) -> Result<&VariableStore<'a>, String> {
        let rows = get(&self.variables, || {
            Ok(table::<tables::ScriptVariables>(self.frame)?
                .iter()
                .map(|row| {
                    let entries = row
                        .entries
                        .iter()
                        .map(|entry| (entry.name.as_slice(), entry.value_raw))
                        .collect();
                    ((row.scope_kind, row.scope_id), (row, entries))
                })
                .collect())
        })?;
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
        rows.get(&(kind, id)).ok_or_else(|| {
            format!("the script variable store of scope {kind}:{id} has no ScriptVariables row")
        })
    }

    /// The script constants database, or why it is unread.
    pub(super) fn script_constants(&self) -> Result<&'a Constants, &'a str> {
        *self.script_constants
    }

    /// Whether a character with this id is recorded as a live reference.
    pub(super) fn character_exists(&self, id: u32) -> Result<bool, String> {
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
        let bits = raw as u64;
        let id = if bits & 0xffff_ffff_f000_0000 == 0xffff_ffff_c000_0000 {
            (bits & 0x0fff_ffff) as i64
        } else {
            fixed_round(raw)
        };
        match u32::try_from(id) {
            Ok(id) if id > 0 && self.states.contains_key(&id) => Ok(id),
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
        Ok(table::<tables::CountryCivilWars>(self.frame)?
            .iter()
            .filter(|row| row.country == raw)
            .map(|row| row.target)
            .collect())
    }

    /// Whether an idea token is a registered advisor token
    /// (AdvisorTokens).
    pub(super) fn advisor_token(&self, name: &[u8]) -> Result<bool, String> {
        Ok(table::<tables::AdvisorTokens>(self.frame)?
            .iter()
            .any(|row| row.token == name))
    }

    /// The idea tokens of a faction's advisors (FactionAdvisors, keyed
    /// by the faction's full reference).
    pub(super) fn faction_advisors(&self, faction: u64) -> Result<Vec<Vec<u8>>, String> {
        Ok(table::<tables::FactionAdvisors>(self.frame)?
            .iter()
            .filter(|row| row.faction == faction)
            .map(|row| row.token.clone())
            .collect())
    }

    /// A country of its own civil-war list exists, or an
    /// existing enemy holds it in its own list.
    pub(super) fn has_civil_war(&self, slot: u32) -> Result<bool, String> {
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

    /// The core-owner trigger tests one of the state's cores
    /// (vector +0xB0, count +0xBC), by raw id or canonical
    /// slot. `StateCores` equals the game's at every hour (its writers are
    /// hooked; unmarked states are read once a day).
    pub(super) fn is_core_of(&self, state: u32, country: Option<u32>) -> Result<Eval, String> {
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

    /// The claimant trigger tests one of the state's claims (vector +0x98,
    /// count +0xA4) as the core-owner trigger tests cores: by raw id or
    /// canonical slot. Claims are hooked as cores are (`StateCores.claims`).
    pub(super) fn is_claimed_by(&self, state: u32, country: Option<u32>) -> Result<Eval, String> {
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
        self.raw_ids.get(&slot).copied()
    }

    pub(super) fn nukes(&self, slot: u32) -> Result<i64, String> {
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

#[cfg(test)]
#[path = "world/zero_countries_tests.rs"]
mod zero_countries_tests;
