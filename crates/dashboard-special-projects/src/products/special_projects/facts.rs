//! Special projects' facts, projected hour by hour from a columnar walk ([`replay::segments`]):
//! the observation of each country whose projects, project history or what names them changed,
//! every country's at a segment's first hour and where the countries or Projects' completeness
//! changed. An observation holds each project's facts with its presentation and person
//! ([`Look`]), its progress apart. Its owners' rows are read from the walk's shared frame
//! ([`dashboard_world`]) only at the hours they changed; presentations and people are resolved
//! again only when a field they read changed. A country whose rows changed in nothing its
//! observation reads but its progress sends that progress alone ([`Seen::Progress`]), and none
//! where that is the same too, unless its last observation was not at a read hour and this one
//! is: the completions take an observation only at read hours. Each hour also carries its read
//! marker and whether the dictionary changed. What a country held before the segment is the
//! ordered emit's ([`super::emit`]).

use super::{
    Content, Look, Naming, ProjectFacts, ProjectState, completions,
    history::{self, Name, NameKey},
    history_operand,
};
use dashboard_base::{
    Source, catalogue,
    products::{LoadContext, politics_content::recorded_politics_content},
};
use dashboard_text::{
    metadata::slot_read,
    triggers::inputs::{TableChange, TableChanges},
};
use dashboard_world::{Hold, Needs, Roster, World};
use replay::{
    columnar_frames::metrics,
    columns,
    segments::{self, Item, Operator, Sink, State},
};
use replay_data::{
    format::FormatVersion,
    rows::{LexicalTokenRow, ProjectHistoryRow, ProjectRow, TableId, tables},
};
use replay_frame::frame::DecodedFrame;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    mem::size_of,
    sync::{Arc, LazyLock, atomic::AtomicBool},
};

/// The tables an observation reads its projects, their history and what names them from.
const OWN: [TableId; 6] = [
    TableId::Projects,
    TableId::ProjectHistory,
    TableId::ProjectPrograms,
    TableId::Characters,
    TableId::CharacterPortraits,
    TableId::LexicalTokens,
];

/// What a presentation or person is resolved from ([`super::resolve_definition`],
/// [`super::resolve_person`]).
const LOOKS: [TableId; 4] = [
    TableId::ProjectPrograms,
    TableId::Characters,
    TableId::CharacterPortraits,
    TableId::LexicalTokens,
];

/// What a scoped project name reads ([`history::NameScan`]): the trigger kit with the rows it
/// observes.
fn scoped_tables() -> &'static [TableId] {
    static TABLES: LazyLock<Vec<TableId>> = LazyLock::new(|| {
        let mut tables = vec![
            TableId::Countries,
            TableId::CountryLinks,
            TableId::Projects,
            TableId::LexicalTokens,
        ];
        tables.extend_from_slice(dashboard_text::triggers::RECORDED_TABLES);
        tables.sort_unstable();
        tables.dedup();
        tables
    });
    TABLES.as_slice()
}

/// Every table a projection reads; the trigger kit too where a project name reads scoped text.
pub fn tables(scoped: bool) -> &'static [TableId] {
    fn of(scoped: bool) -> Vec<TableId> {
        let mut tables = vec![
            TableId::Campaign,
            TableId::Countries,
            TableId::CountryLinks,
        ];
        tables.extend(OWN);
        if scoped {
            tables.extend_from_slice(scoped_tables());
        }
        tables.sort_unstable();
        tables.dedup();
        tables
    }
    static PLAIN: LazyLock<Vec<TableId>> = LazyLock::new(|| of(false));
    static SCOPED: LazyLock<Vec<TableId>> = LazyLock::new(|| of(true));
    if scoped {
        SCOPED.as_slice()
    } else {
        PLAIN.as_slice()
    }
}

/// What a projection reads of its walk's world: the countries, and the frame of its own tables
/// (and of the trigger kit where a project name reads scoped text).
pub fn needs(scoped: bool) -> Needs {
    let mut frame: Vec<(TableId, Hold)> = OWN.iter().map(|&table| (table, Hold::Whole)).collect();
    if scoped {
        frame.extend(scoped_tables().iter().map(|&table| (table, Hold::Whole)));
    }
    Needs {
        countries: true,
        frame,
        ..Needs::default()
    }
}

/// The recorded content projects are named and drawn with, loaded once for a preparation; none,
/// with why, where the recorded stack gives none.
pub struct Binding {
    content: Option<Content>,
    error: Option<String>,
    /// Whether a project name reads scoped text ([`completions::scripted_names`]).
    scoped: bool,
}

impl Binding {
    pub fn load(source: &Source, context: &LoadContext) -> io::Result<Self> {
        // Built from the recording's own catalogue alone, so bound to the recorded content.
        let loaded = context
            .content()
            .and_then(|options| recorded_politics_content(source, options))
            .map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!("Project content, names and art unavailable: {error}"),
                )
            })
            .and_then(|politics| {
                Content::load(politics).map_err(|error| {
                    io::Error::new(
                        error.kind(),
                        format!("Special-project content unavailable: {error}"),
                    )
                })
            });
        match loaded {
            Ok(content) => {
                let scoped = completions::scripted_names(
                    &content.definitions,
                    &content.catalogue.localisation,
                );
                Ok(Self {
                    content: Some(content),
                    error: None,
                    scoped,
                })
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => Err(error),
            Err(error) => Ok(Self {
                content: None,
                error: Some(error.to_string()),
                scoped: false,
            }),
        }
    }

    pub fn scoped(&self) -> bool {
        self.scoped
    }

    /// The content, where it loaded.
    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn content(&self) -> Option<&Content> {
        self.content.as_ref()
    }

    /// What the emit reads of the content: its warnings and why there is none.
    pub fn emitted(&self) -> super::emit::Bound {
        let content = self.content.as_ref();
        let completion_warnings = content.map_or_else(Vec::new, |content| {
            let mut warnings = content.definitions.warnings.clone();
            warnings.extend(content.catalogue.verification.iter().cloned());
            warnings
        });
        let mut warnings = completion_warnings.clone();
        if let Some(content) = content {
            warnings.extend(content.catalogue.warnings.iter().cloned());
        }
        super::emit::Bound {
            content: self.content.clone(),
            catalogue: content.map(|content| Arc::clone(&content.catalogue)),
            warnings,
            completion_warnings,
            error: self.error.clone(),
            empty: content.is_some_and(|content| content.definitions.projects.is_empty()),
        }
    }
}

/// A project as an observation holds it: its facts (their progress zero), its presentation and
/// person, and its progress apart, which changes without changing the rest.
#[derive(Clone, Debug)]
pub struct Project {
    pub(super) facts: ProjectFacts,
    pub(super) look: Look,
    pub(super) progress: i32,
}

/// A country observed at the hour: its holder and what of it the hour sends.
#[derive(Clone, Debug)]
pub struct Observed {
    pub(super) tag: Arc<str>,
    /// Its canonical slot and country id.
    pub(super) identity: (u32, u32),
    pub(super) seen: Seen,
}

/// What an observation sends of a country.
#[derive(Clone, Debug)]
pub enum Seen {
    /// Its projects, none while Projects is not complete.
    Whole(Option<Vec<Project>>),
    /// Its last observation's projects, under the same holder at the hour before, but for
    /// their progress: each one's now, in that observation's order.
    Progress(Vec<i32>),
}

/// What the ordered emit reads of an hour that observed something.
#[derive(Debug, Default)]
pub struct Facts {
    /// Every country the countries resolve was observed: one not among `countries` holds none.
    pub(super) full: bool,
    /// The countries observed, in tag order.
    pub(super) countries: Vec<Observed>,
    /// The scoped project-name observations with refusals the hour counted, by tag and reason.
    pub(super) named: BTreeMap<String, BTreeMap<String, usize>>,
    /// The scoped names the hour changed: each one's name now, none where it names nothing.
    pub(super) names: Vec<(NameKey, Option<Name>)>,
    /// `names` holds every name the hour has: any other names nothing.
    pub(super) names_whole: bool,
    /// The projects named that an episode the projection did not see begin may hold open,
    /// with their names, which `names` leaves out ([`history::NameScan::uncertain`]).
    pub(super) provisional: Vec<(NameKey, (u32, u32), Name)>,
    /// The content refusals and readers the segment's reads noted so far, where they grew.
    pub(super) own: Option<catalogue::OwnRefusals>,
}

/// The dictionary as an hour changed it: its digest, an order-free sum that equal rows give,
/// and the spellings it changed, every one it holds where `whole`. A spelling that is not UTF-8
/// is none, as the row reader takes it.
#[derive(Debug)]
pub struct Dictionary {
    pub(super) digest: u128,
    pub(super) whole: bool,
    pub(super) tokens: Vec<(u32, Option<String>)>,
}

/// One hour of a segment.
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    /// Projects was read complete (`slot_read`).
    pub(super) read: bool,
    /// The dictionary, at a segment's first hour and where the hour changed its rows.
    pub(super) dictionary: Option<Box<Dictionary>>,
    /// None when the hour observed nothing.
    pub(super) facts: Option<Box<Facts>>,
}

/// A rough share of the heap a look's texts and refusals hold.
const LOOK_BYTES: usize = 512;

impl Item for Hour {
    fn heap(&self) -> usize {
        let dictionary = self.dictionary.as_ref().map_or(0, |dictionary| {
            size_of::<Dictionary>()
                + dictionary.tokens.capacity() * size_of::<(u32, Option<String>)>()
                + dictionary
                    .tokens
                    .iter()
                    .map(|(_, token)| token.as_ref().map_or(0, String::capacity))
                    .sum::<usize>()
        });
        let Some(facts) = &self.facts else {
            return dictionary;
        };
        size_of::<Facts>()
            + facts.countries.capacity() * size_of::<Observed>()
            + facts
                .countries
                .iter()
                .map(|country| {
                    country.tag.len()
                        + match &country.seen {
                            Seen::Whole(projects) => projects.as_ref().map_or(0, |projects| {
                                projects.capacity() * (size_of::<Project>() + LOOK_BYTES)
                            }),
                            Seen::Progress(progress) => progress.capacity() * size_of::<i32>(),
                        }
                })
                .sum::<usize>()
            + facts
                .named
                .iter()
                .map(|(tag, reasons)| {
                    tag.len() + reasons.keys().map(|reason| reason.len() + 32).sum::<usize>()
                })
                .sum::<usize>()
            + facts
                .names
                .iter()
                .map(|((tag, _, _), name)| {
                    tag.len()
                        + name.as_ref().map_or(0, |name| {
                            name.text.as_ref().map_or(0, String::len)
                                + name.warnings.iter().map(String::len).sum::<usize>()
                        })
                        + 64
                })
                .sum::<usize>()
            + facts
                .provisional
                .iter()
                .map(|((tag, _, _), _, name)| {
                    tag.len()
                        + name.text.as_ref().map_or(0, String::len)
                        + name.warnings.iter().map(String::len).sum::<usize>()
                        + 64
                })
                .sum::<usize>()
            + facts.own.as_ref().map_or(0, |_| LOOK_BYTES)
            + dictionary
    }
}

/// A project row's operands an observation reads, projected when the row was read.
#[derive(Clone)]
struct Input {
    facts: ProjectFacts,
    program_reference: u64,
}

impl Input {
    fn of(row: &ProjectRow, hour: i32, start: i32) -> Self {
        Self {
            facts: ProjectFacts::project(row, None, hour, start),
            program_reference: row.program_reference,
        }
    }

    fn row(&self, owner: u32) -> ProjectRow {
        presentation_row(&self.facts, self.program_reference, owner)
    }

    /// Whether an observation reads the same of `self` as of `other` but for its progress.
    fn same_but_progress(&self, other: &Self) -> bool {
        let (left, right) = (&self.facts, &other.facts);
        self.program_reference == other.program_reference
            && left.local_id == right.local_id
            && left.definition_id == right.definition_id
            && left.visible == right.visible
            && left.state == right.state
            && left.completion == right.completion
            && left.iterations == right.iterations
            && left.warning == right.warning
    }
}

/// The row a project's presentation and person are resolved from: its `facts` with
/// `program_reference`, owned by `owner`.
pub(super) fn presentation_row(facts: &ProjectFacts, program_reference: u64, owner: u32) -> ProjectRow {
    let mut row = ProjectRow {
        owner,
        local_id: facts.local_id,
        definition_id: facts.definition_id,
        program_reference,
        ..Default::default()
    };
    if facts.state == ProjectState::Completed {
        row.state.current_offset = 0xe8;
        row.state.not_completed = 0;
    }
    row
}

/// What a country's last observation read, which a change to it observes again.
#[derive(Default)]
struct Held {
    /// Each project's definition and program reference, whose looks the naming caches.
    looks: Vec<(u32, u64)>,
    programs: Vec<u32>,
    people: Vec<u32>,
    tokens: Vec<u32>,
    /// Each project's progress, in the observation's order, as last sent.
    progress: Vec<i32>,
    /// It was observed at a read hour, whose completions took it.
    fed: bool,
    /// No two of its projects share a local id and definition: observing them again as they
    /// were changes none of their episodes.
    distinct: bool,
    /// A completion of its history came due at an hour that did not observe it: its next
    /// observation is whole.
    due: bool,
}

/// The countries reading each program, character or token.
type Dependents = HashMap<u32, BTreeSet<Arc<str>>>;

/// The columns a dependency is read from.
struct Fields {
    scientist: usize,
    name_token: usize,
    /// The character's saved token, which names it.
    character_token: usize,
    literal: usize,
    /// The ProjectHistory fields a history operand reads ([`history_operand`]), a bit by column.
    history: u64,
}

impl Fields {
    /// The fields of each dependency's table a look reads ([`super::resolve_person`]),
    /// a bit by column; every field (all bits) where one lies past a change mask's.
    fn read(&self, table: TableId) -> u64 {
        let columns: &[usize] = match table {
            TableId::ProjectPrograms => &[self.scientist],
            TableId::Characters => &[self.name_token, self.character_token],
            _ => return u64::MAX,
        };
        columns.iter().try_fold(0_u64, |mask, &column| {
            u32::try_from(column)
                .ok()
                .and_then(|column| 1_u64.checked_shl(column))
                .map(|bit| mask | bit)
        })
        .unwrap_or(u64::MAX)
    }
}

/// How a country whose rows changed in nothing its observation reads but its progress goes on.
enum Again {
    /// Observed whole.
    Whole,
    /// Nothing sent: its observation is its last.
    Same,
    /// Its progress sent, under its holder.
    Progress((u32, u32), Vec<i32>),
}

/// The digest of a dictionary that is not complete.
const INCOMPLETE: u128 = u128::MAX;

/// Nothing cancels a projection's scoped names: the walk stops between hours.
static RUNNING: AtomicBool = AtomicBool::new(false);

/// Whether the hour inserted, deleted or wrote a row of `table`, read it in part, or gave it
/// another status.
fn changed(state: &State, world: &World, table: TableId) -> io::Result<bool> {
    if world.restated(table) {
        return Ok(true);
    }
    let changes = columns::changes(state, table)?;
    Ok(changes.truncated
        || !changes.changed.is_empty()
        || !changes.fill_slots.is_empty()
        || !changes.inserted.is_empty()
        || !changes.deleted.is_empty())
}

/// Into `owners`, ascending, the owners of the Projects rows the hour inserted, deleted or
/// wrote: Projects rows are keyed by owner, then project.
fn moved_owners(state: &State, owners: &mut Vec<u32>) -> io::Result<()> {
    let table = TableId::Projects;
    let (changes, held) = (columns::changes(state, table)?, columns::table(state, table)?);
    owners.clear();
    for &slot in changes
        .changed
        .iter()
        .chain(&changes.fill_slots)
        .chain(&changes.inserted)
    {
        owners.extend(held.key(slot).map(|key| (key >> 32) as u32));
    }
    owners.extend(changes.deleted_keys.iter().map(|&key| (key >> 32) as u32));
    owners.sort_unstable();
    owners.dedup();
    Ok(())
}

/// The hour's changes as scoped names read them: a table the hour touched may have changed
/// anywhere, any other did not change. A segment's first hour reads every table anew.
struct Touched<'h> {
    state: &'h State,
    world: &'h World,
    opens: bool,
}

impl TableChanges for Touched<'_> {
    fn table(&self, table: TableId) -> TableChange<'_> {
        if self.opens {
            return TableChange::Any;
        }
        match changed(self.state, self.world, table) {
            Ok(false) => TableChange::Same,
            _ => TableChange::Any,
        }
    }
}

/// The hour's frames, each asked of the walk's world at its first need: the tables looks are
/// resolved from, or else one of the hour alone.
struct Frames<'h> {
    world: &'h World,
    state: &'h State,
    looks: Option<DecodedFrame>,
    hour: Option<DecodedFrame>,
}

impl Frames<'_> {
    fn looks(&mut self) -> io::Result<&DecodedFrame> {
        if self.looks.is_none() {
            self.looks = Some(self.world.frame(self.state, &LOOKS)?);
        }
        Ok(self.looks.as_ref().expect("the looks' frame"))
    }

    /// A frame dated the hour: what scoped names are found by.
    fn hour(&mut self) -> io::Result<&DecodedFrame> {
        if self.looks.is_none() && self.hour.is_none() {
            self.hour = Some(self.world.frame(self.state, &[])?);
        }
        Ok(self
            .looks
            .as_ref()
            .or(self.hour.as_ref())
            .expect("a frame of the hour"))
    }
}

/// The actors the complete countries and links resolve, as the row observer binds them
/// ([`history::Actors::bind`]); none while either is not complete.
fn actors(roster: &Roster) -> io::Result<history::Actors> {
    if !roster.complete() {
        return Ok(history::Actors::default());
    }
    let keys = dashboard_text::metadata::CountryKeys::read(&roster.country_rows()?);
    let links: Vec<(u32, u32)> = roster
        .links()
        .iter()
        .map(|link| (link.id, link.canonical_slot))
        .collect();
    Ok(history::Actors::bind(Some(&keys), Some(links.as_slice())))
}

/// One segment's projection: every segment starts cold.
pub struct Projector {
    format: FormatVersion,
    content: Option<Content>,
    scoped: bool,
    /// Each branch's first recorded hour, which completion dates are bounded by.
    starts: Arc<BTreeMap<u64, i32>>,
    fields: Fields,
    /// Nothing is held: the next hour is read whole, as a segment's first.
    cold: bool,
    last: Option<i32>,
    /// The branch's first recorded hour.
    start: i32,
    actors: Option<history::Actors>,
    /// Each raw owner id's tag, by the links.
    owners: HashMap<u32, Arc<str>>,
    /// Each owner's projects; none while Projects is not complete.
    projects: Option<BTreeMap<u32, Vec<Input>>>,
    /// The owners whose Projects rows the hour moved, kept from hour to hour.
    moved: Vec<u32>,
    /// Each owner's and project's history operands, by ordinal; none while ProjectHistory is not
    /// complete.
    history: Option<BTreeMap<(u32, u32), Vec<(u32, bool, i32)>>>,
    /// Every completion operand of `history`: its date, owner, project and ordinal.
    completions: BTreeSet<(i32, u32, u32, u32)>,
    /// The ProjectHistory keys an hour edited, and the owners whose operands it changed, kept
    /// from hour to hour.
    history_keys: Vec<u128>,
    history_moved: Vec<u32>,
    naming: Naming,
    /// What each country's last observation read.
    observed: HashMap<Arc<str>, Held>,
    programs: Dependents,
    people: Dependents,
    tokens: Dependents,
    /// The countries whose last observation holds a project.
    holders: BTreeSet<Arc<str>>,
    names: history::NameScan,
    /// The scoped names last sent.
    sent_names: HashMap<NameKey, Name>,
    /// Each dictionary row's hash by key, and their sum: a digest equal rows give whatever
    /// their order.
    spelled: HashMap<u128, u128>,
    digest: u128,
    own: catalogue::OwnRefusals,
    /// What `own` held when an hour last carried it.
    sent: catalogue::OwnRefusals,
}

impl Projector {
    pub fn new(
        format: FormatVersion,
        binding: &Binding,
        starts: Arc<BTreeMap<u64, i32>>,
    ) -> io::Result<Self> {
        Ok(Self {
            format,
            content: binding.content.clone(),
            scoped: binding.scoped,
            starts,
            fields: Fields {
                scientist: columns::field(TableId::ProjectPrograms, "scientist")?,
                name_token: columns::field(TableId::Characters, "name_token_id")?,
                character_token: columns::field(TableId::Characters, "token")?,
                literal: columns::field(TableId::LexicalTokens, "literal")?,
                history: ["prototype_reward_handle", "option_handle", "date"]
                    .into_iter()
                    .try_fold(0_u64, |mask, name| {
                        let column = columns::field(TableId::ProjectHistory, name)?;
                        u32::try_from(column)
                            .ok()
                            .and_then(|column| 1_u64.checked_shl(column))
                            .map(|bit| mask | bit)
                            .ok_or_else(|| io::Error::other("A history field lies past a mask"))
                    })?,
            },
            cold: true,
            last: None,
            start: 0,
            actors: None,
            owners: HashMap::new(),
            projects: None,
            moved: Vec::new(),
            history: None,
            completions: BTreeSet::new(),
            history_keys: Vec::new(),
            history_moved: Vec::new(),
            naming: Naming::default(),
            observed: HashMap::new(),
            programs: HashMap::new(),
            people: HashMap::new(),
            tokens: HashMap::new(),
            holders: BTreeSet::new(),
            names: history::NameScan::default(),
            sent_names: HashMap::new(),
            spelled: HashMap::new(),
            digest: INCOMPLETE,
            own: catalogue::OwnRefusals::default(),
            sent: catalogue::OwnRefusals::default(),
        })
    }

    /// Forgets everything held: the hour is read whole.
    fn reset(&mut self, branch: u64) {
        self.start = self.starts.get(&branch).copied().unwrap_or(0);
        self.actors = None;
        self.owners.clear();
        self.projects = None;
        self.history = None;
        self.completions.clear();
        self.naming = Naming::default();
        self.observed.clear();
        self.programs.clear();
        self.people.clear();
        self.tokens.clear();
        self.holders.clear();
        self.names = history::NameScan::default();
        self.sent_names.clear();
        self.spelled.clear();
        self.digest = INCOMPLETE;
        if let Some(content) = &mut self.content {
            content.scoped.clear();
        }
    }

    /// The dictionary row at `slot`: its key, its hash and its spelling.
    fn token(&self, state: &State, slot: u32) -> io::Result<(u128, u128, Option<String>)> {
        let table = TableId::LexicalTokens;
        let held = columns::table(state, table)?;
        let key = held
            .key(slot)
            .ok_or_else(|| io::Error::other("A dictionary slot holds no row"))?;
        let mut hasher = blake3::Hasher::new();
        hasher.update(&key.to_le_bytes());
        let spelling;
        if held.recorded(self.fields.literal, slot) {
            let text = *columns::text(held, self.fields.literal)?
                .get(slot as usize)
                .ok_or_else(|| io::Error::other("A token's slot holds no cell"))?;
            let literal = state
                .dictionary()
                .get(text)
                .ok_or_else(|| io::Error::other("A token names no dictionary entry"))?;
            hasher.update(literal);
            spelling = std::str::from_utf8(literal).ok().map(str::to_owned);
        } else {
            let row: LexicalTokenRow = columns::row(state, table, slot, self.format.version())?;
            hasher.update(&row.literal);
            spelling = String::from_utf8(row.literal).ok();
        }
        let mut hash = [0; 16];
        hash.copy_from_slice(&hasher.finalize().as_bytes()[..16]);
        Ok((key, u128::from_le_bytes(hash), spelling))
    }

    /// The dictionary at a segment's first hour (`opens`) and where the hour changed its rows:
    /// read whole then, else kept from the rows the hour wrote.
    fn dictionary(
        &mut self,
        state: &State,
        world: &World,
        opens: bool,
    ) -> io::Result<Option<Box<Dictionary>>> {
        let table = TableId::LexicalTokens;
        if !opens && !changed(state, world, table)? {
            return Ok(None);
        }
        let mut tokens = Vec::new();
        if !columns::complete(state, table) {
            self.spelled.clear();
            self.digest = INCOMPLETE;
            return Ok(Some(Box::new(Dictionary {
                digest: INCOMPLETE,
                whole: true,
                tokens,
            })));
        }
        let edits = columns::changes(state, table)?;
        if opens || world.restated(table) || edits.truncated || self.digest == INCOMPLETE {
            self.spelled.clear();
            self.digest = 0;
            let live: Vec<u32> = columns::table(state, table)?.live().ones().collect();
            tokens.reserve(live.len());
            for slot in live {
                let (key, hash, spelling) = self.token(state, slot)?;
                self.spelled.insert(key, hash);
                self.digest = self.digest.wrapping_add(hash);
                tokens.push((key as u32, spelling));
            }
            return Ok(Some(Box::new(Dictionary {
                digest: self.digest,
                whole: true,
                tokens,
            })));
        }
        for &key in &edits.deleted_keys {
            if let Some(hash) = self.spelled.remove(&key) {
                self.digest = self.digest.wrapping_sub(hash);
                tokens.push((key as u32, None));
            }
        }
        for &slot in edits
            .inserted
            .iter()
            .chain(&edits.changed)
            .chain(&edits.fill_slots)
        {
            let (key, hash, spelling) = self.token(state, slot)?;
            if let Some(old) = self.spelled.insert(key, hash) {
                self.digest = self.digest.wrapping_sub(old);
            }
            self.digest = self.digest.wrapping_add(hash);
            tokens.retain(|&(held, _)| held != key as u32);
            tokens.push((key as u32, spelling));
        }
        Ok(Some(Box::new(Dictionary {
            digest: self.digest,
            whole: false,
            tokens,
        })))
    }

    /// Takes up the countries `actors` resolve: their owners' tags, and nothing their last
    /// observations read.
    fn bind(&mut self, actors: history::Actors) {
        self.owners.clear();
        for (tag, &(_, slot)) in &actors.tags {
            let Ok(tag) = std::str::from_utf8(tag) else {
                continue;
            };
            let tag: Arc<str> = Arc::from(tag);
            for link in actors.links.get(&slot).into_iter().flatten() {
                self.owners.insert(link.id, Arc::clone(&tag));
            }
        }
        self.actors = Some(actors);
        self.naming = Naming::default();
        self.observed.clear();
        self.programs.clear();
        self.people.clear();
        self.tokens.clear();
    }

    /// Every owner's projects, read whole.
    fn read_projects(&mut self, world: &World, state: &State, hour: i32) -> io::Result<()> {
        let frame = world.frame(state, &[TableId::Projects])?;
        let rows = frame
            .complete::<tables::Projects>()
            .map_err(|_| io::Error::other("A complete Projects table has no rows"))?;
        let mut inputs = BTreeMap::<u32, Vec<Input>>::new();
        for row in rows {
            inputs
                .entry(row.owner)
                .or_default()
                .push(Input::of(row, hour, self.start));
        }
        self.projects = Some(inputs);
        Ok(())
    }

    /// The projects of `owners` read again, in place where they hold as many; every other
    /// owner's are the hour before's. Into `reshaped`, the owners whose projects changed in
    /// more than their progress.
    fn reread_projects(
        &mut self,
        world: &World,
        state: &State,
        hour: i32,
        owners: &[u32],
        reshaped: &mut BTreeSet<u32>,
    ) -> io::Result<()> {
        let frame = world.frame(state, &[TableId::Projects])?;
        let rows = frame
            .complete::<tables::Projects>()
            .map_err(|_| io::Error::other("A complete Projects table has no rows"))?;
        let start = self.start;
        let inputs = self
            .projects
            .as_mut()
            .ok_or_else(|| io::Error::other("Projects read again before it was read"))?;
        // Projects rows are keyed by owner, then project.
        for &owner in owners {
            let first = rows.partition_point(|row| row.owner < owner);
            let count = rows[first..].partition_point(|row| row.owner == owner);
            let rows = &rows[first..first + count];
            match inputs.get_mut(&owner) {
                None if rows.is_empty() => {}
                Some(held) if held.len() == rows.len() => {
                    let mut same = true;
                    for (held, row) in held.iter_mut().zip(rows) {
                        let input = Input::of(row, hour, start);
                        same &= held.same_but_progress(&input);
                        *held = input;
                    }
                    if !same {
                        reshaped.insert(owner);
                    }
                }
                _ if rows.is_empty() => {
                    inputs.remove(&owner);
                    reshaped.insert(owner);
                }
                _ => {
                    inputs.insert(
                        owner,
                        rows.iter().map(|row| Input::of(row, hour, start)).collect(),
                    );
                    reshaped.insert(owner);
                }
            }
        }
        Ok(())
    }

    /// Every project's history operands, read whole, and their completions.
    fn read_history(&mut self, world: &World, state: &State) -> io::Result<()> {
        let frame = world.frame(state, &[TableId::ProjectHistory])?;
        let rows = frame
            .complete::<tables::ProjectHistory>()
            .map_err(|_| io::Error::other("A complete ProjectHistory table has no rows"))?;
        let mut inputs = BTreeMap::<(u32, u32), Vec<(u32, bool, i32)>>::new();
        self.completions.clear();
        for row in rows {
            let operand = history_operand(row);
            if operand.1 {
                self.completions
                    .insert((operand.2, row.owner, row.project, row.ordinal));
            }
            inputs
                .entry((row.owner, row.project))
                .or_default()
                .push(operand);
        }
        self.history = Some(inputs);
        Ok(())
    }

    /// The history operands of the rows the hour inserted, deleted or wrote a field of an
    /// operand in, edited in place; into `moved`, ascending, the owners whose operands changed.
    fn edit_history(&mut self, state: &State, moved: &mut Vec<u32>) -> io::Result<()> {
        let table = TableId::ProjectHistory;
        let (changes, held) = (columns::changes(state, table)?, columns::table(state, table)?);
        let history = self
            .history
            .as_mut()
            .ok_or_else(|| io::Error::other("ProjectHistory edited before it was read"))?;
        let read = self.fields.history;
        let masked = changes.masks.len() == changes.changed.len()
            && changes.fill_masks.len() == changes.fill_slots.len();
        let keys = &mut self.history_keys;
        keys.clear();
        keys.extend_from_slice(&changes.deleted_keys);
        let written = changes
            .changed
            .iter()
            .zip(changes.masks.iter().map(Some).chain(std::iter::repeat(None)))
            .chain(
                changes
                    .fill_slots
                    .iter()
                    .zip(changes.fill_masks.iter().map(Some).chain(std::iter::repeat(None))),
            );
        for (&slot, mask) in written {
            if !masked || mask.is_none_or(|&mask| mask & read != 0) {
                keys.extend(held.key(slot));
            }
        }
        keys.extend(changes.inserted.iter().filter_map(|&slot| held.key(slot)));
        keys.sort_unstable();
        keys.dedup();
        moved.clear();
        // ProjectHistory rows are keyed by owner, project, then ordinal.
        for &key in keys.iter() {
            let (owner, project, ordinal) = ((key >> 64) as u32, (key >> 32) as u32, key as u32);
            let now = match held.slot(key) {
                Some(slot) => {
                    let row: ProjectHistoryRow =
                        columns::row(state, table, slot, self.format.version())?;
                    Some(history_operand(&row))
                }
                None => None,
            };
            let rows = history.get(&(owner, project));
            let at = rows.map(|rows| rows.binary_search_by_key(&ordinal, |&(held, ..)| held));
            let was = rows.zip(at).and_then(|(rows, at)| at.ok().map(|at| rows[at]));
            if was == now {
                continue;
            }
            if let Some((_, true, date)) = was {
                self.completions.remove(&(date, owner, project, ordinal));
            }
            if let Some((_, true, date)) = now {
                self.completions.insert((date, owner, project, ordinal));
            }
            let rows = history.entry((owner, project)).or_default();
            match (at.unwrap_or(Err(0)), now) {
                (Ok(at), Some(now)) => rows[at] = now,
                (Ok(at), None) => {
                    rows.remove(at);
                }
                (Err(at), Some(now)) => rows.insert(at, now),
                (Err(_), None) => {}
            }
            if rows.is_empty() {
                history.remove(&(owner, project));
            }
            moved.push(owner);
        }
        moved.sort_unstable();
        moved.dedup();
        Ok(())
    }

    /// The owners with a completion dated after `previous` and by `hour`.
    fn completed(&self, previous: Option<i32>, hour: i32) -> BTreeSet<u32> {
        let from = match previous {
            None => i32::MIN,
            Some(previous) => match previous.checked_add(1) {
                Some(from) => from,
                None => return BTreeSet::new(),
            },
        };
        if from > hour {
            return BTreeSet::new();
        }
        self.completions
            .range((from, 0, 0, 0)..=(hour, u32::MAX, u32::MAX, u32::MAX))
            .map(|&(_, owner, ..)| owner)
            .collect()
    }

    /// The countries reading a row of `table` the hour changed, by `dependents`: into
    /// `renamed` where it inserted or deleted the row or wrote a field of `read`, else into
    /// `touched`.
    fn dependents(
        (state, world): (&State, &World),
        table: TableId,
        read: u64,
        dependents: &Dependents,
        renamed: &mut BTreeSet<Arc<str>>,
        touched: &mut BTreeSet<Arc<str>>,
    ) -> io::Result<()> {
        if dependents.is_empty() || !changed(state, world, table)? {
            return Ok(());
        }
        if world.restated(table) {
            renamed.extend(dependents.values().flatten().cloned());
            return Ok(());
        }
        let (changes, held) = (columns::changes(state, table)?, columns::table(state, table)?);
        let masked = read != u64::MAX
            && changes.masks.len() == changes.changed.len()
            && changes.fill_masks.len() == changes.fill_slots.len();
        let mut note = |key: u128, relevant: bool| {
            if let Some(tags) = dependents.get(&(key as u32)) {
                let into = if relevant { &mut *renamed } else { &mut *touched };
                into.extend(tags.iter().cloned());
            }
        };
        let written = changes
            .changed
            .iter()
            .zip(changes.masks.iter().map(Some).chain(std::iter::repeat(None)))
            .chain(
                changes
                    .fill_slots
                    .iter()
                    .zip(changes.fill_masks.iter().map(Some).chain(std::iter::repeat(None))),
            );
        for (&slot, mask) in written {
            if let Some(key) = held.key(slot) {
                note(key, !masked || mask.is_none_or(|&mask| mask & read != 0));
            }
        }
        for &slot in &changes.inserted {
            if let Some(key) = held.key(slot) {
                note(key, true);
            }
        }
        for &key in &changes.deleted_keys {
            note(key, true);
        }
        Ok(())
    }

    /// The scientist reference program `id` holds, where ProjectPrograms is complete.
    fn scientist(&self, state: &State, id: u32) -> io::Result<Option<u64>> {
        let table = TableId::ProjectPrograms;
        if !columns::complete(state, table) {
            return Ok(None);
        }
        let held = columns::table(state, table)?;
        let Some(slot) = held.slot(u128::from(id)) else {
            return Ok(None);
        };
        if !held.recorded(self.fields.scientist, slot) {
            let row: replay_data::rows::ProjectProgramRow =
                columns::row(state, table, slot, self.format.version())?;
            return Ok(Some(row.scientist));
        }
        Ok(columns::plain::<u64>(held, self.fields.scientist)?
            .get(slot as usize)
            .copied())
    }

    /// The name token character `id` holds, where Characters is complete.
    fn name_token(&self, state: &State, id: u32) -> io::Result<Option<u32>> {
        let table = TableId::Characters;
        if !columns::complete(state, table) {
            return Ok(None);
        }
        let held = columns::table(state, table)?;
        let Some(slot) = held.slot(u128::from(id)) else {
            return Ok(None);
        };
        let token = if held.recorded(self.fields.name_token, slot) {
            columns::plain::<i32>(held, self.fields.name_token)?
                .get(slot as usize)
                .copied()
        } else {
            let row: replay_data::rows::CharacterRow =
                columns::row(state, table, slot, self.format.version())?;
            Some(row.name_token_id)
        };
        Ok(token.and_then(|token| u32::try_from(token).ok()))
    }

    /// What `tag`'s observation of `projects` reads: each project's definition token, its
    /// program, the program's scientist and that character's name token.
    fn held(&self, state: &State, projects: &[Project]) -> io::Result<Held> {
        let mut held = Held::default();
        for project in projects {
            held.progress.push(project.progress);
            let reference = project.look.program_reference;
            held.looks.push((project.facts.definition_id, reference));
            held.tokens.push(project.facts.definition_id);
            if reference as u32 != engine_projects::nproject_cprogram::REFERENCE_TYPE {
                continue;
            }
            let program = (reference >> 32) as u32;
            held.programs.push(program);
            let Some(scientist) = self.scientist(state, program)? else {
                continue;
            };
            if scientist as u32 != engine_native::ccharacter::REFERENCE_TYPE {
                continue;
            }
            let person = (scientist >> 32) as u32;
            held.people.push(person);
            held.tokens.extend(self.name_token(state, person)?);
        }
        let mut keys: Vec<(u32, u32)> = projects
            .iter()
            .map(|project| (project.facts.local_id, project.facts.definition_id))
            .collect();
        keys.sort_unstable();
        held.distinct = keys.windows(2).all(|pair| pair[0] != pair[1]);
        for list in [&mut held.programs, &mut held.people, &mut held.tokens] {
            list.sort_unstable();
            list.dedup();
        }
        Ok(held)
    }

    /// Indexes what `tag` reads now, in place of what it read before.
    fn index(&mut self, tag: &Arc<str>, held: Held) {
        if let Some(before) = self.observed.remove(tag) {
            for (ids, dependents) in [
                (&before.programs, &mut self.programs),
                (&before.people, &mut self.people),
                (&before.tokens, &mut self.tokens),
            ] {
                for id in ids {
                    if let Some(tags) = dependents.get_mut(id) {
                        tags.remove(tag);
                        if tags.is_empty() {
                            dependents.remove(id);
                        }
                    }
                }
            }
            // The looks it no longer reads.
            for &(definition, program) in &before.looks {
                if !held.looks.iter().any(|&(held, _)| held == definition) {
                    self.naming
                        .definitions
                        .remove(&(tag.to_string(), definition));
                }
                if !held.looks.iter().any(|&(_, held)| held == program) {
                    self.naming.people.remove(&program);
                }
            }
        }
        for (ids, dependents) in [
            (&held.programs, &mut self.programs),
            (&held.people, &mut self.people),
            (&held.tokens, &mut self.tokens),
        ] {
            for &id in ids {
                dependents.entry(id).or_default().insert(Arc::clone(tag));
            }
        }
        self.observed.insert(Arc::clone(tag), held);
    }

    /// `tag`'s observation at `hour`, a read hour where `read`, with its looks resolved anew
    /// where `renamed`.
    fn observe(
        &mut self,
        frames: &mut Frames<'_>,
        tag: &Arc<str>,
        (hour, read): (i32, bool),
        renamed: bool,
    ) -> io::Result<Option<Observed>> {
        let Some(&(country_id, slot)) = self
            .actors
            .as_ref()
            .and_then(|actors| actors.tags.get(tag.as_bytes()))
        else {
            return Ok(None);
        };
        if renamed && let Some(held) = self.observed.get(tag) {
            for &(definition, program) in &held.looks {
                self.naming
                    .definitions
                    .remove(&(tag.to_string(), definition));
                self.naming.people.remove(&program);
            }
        }
        let links = self
            .actors
            .as_ref()
            .and_then(|actors| actors.links.get(&slot))
            .cloned()
            .unwrap_or_default();
        let projects = match &self.projects {
            None => None,
            Some(inputs) => {
                let mut observed = Vec::new();
                for link in &links {
                    for input in inputs.get(&link.id).into_iter().flatten() {
                        let row = input.row(link.id);
                        let operands: Vec<(u32, bool, i32)> = links
                            .iter()
                            .flat_map(|owner| {
                                self.history
                                    .as_ref()
                                    .and_then(|history| history.get(&(owner.id, row.local_id)))
                            })
                            .flatten()
                            .copied()
                            .collect();
                        let mut facts = input.facts.clone();
                        facts.observe_history(
                            self.history.as_ref().map(|_| operands.as_slice()),
                            hour,
                            self.start,
                        );
                        let progress = std::mem::take(&mut facts.progress);
                        let cached = self.naming.people.contains_key(&row.program_reference)
                            && self
                                .naming
                                .definitions
                                .contains_key(&(tag.to_string(), row.definition_id));
                        let frame = if cached {
                            frames.hour()?
                        } else {
                            frames.looks()?
                        };
                        let look = self.naming.look(
                            frame,
                            tag,
                            slot,
                            &row,
                            facts.iterations,
                            self.content.as_mut(),
                        );
                        observed.push(Project {
                            facts,
                            look,
                            progress,
                        });
                    }
                }
                Some(observed)
            }
        };
        let mut held = self.held(frames.state, projects.as_deref().unwrap_or_default())?;
        held.fed = read;
        self.index(tag, held);
        if projects.as_ref().is_some_and(|projects| !projects.is_empty()) {
            self.holders.insert(Arc::clone(tag));
        } else {
            self.holders.remove(tag);
        }
        Ok(Some(Observed {
            tag: Arc::clone(tag),
            identity: (slot, country_id),
            seen: Seen::Whole(projects),
        }))
    }

    /// How `tag` goes on where its rows changed in nothing its last observation read but its
    /// progress, at a read hour where `read`: observed whole where that observation was not at
    /// a read hour and this one is, as the completions take an observation only at read hours.
    fn again(&mut self, tag: &Arc<str>, read: bool) -> Again {
        let Self {
            observed,
            projects,
            actors,
            ..
        } = self;
        let (Some(held), Some(actors)) = (observed.get_mut(tag), actors.as_ref()) else {
            return Again::Whole;
        };
        let Some(&(country_id, slot)) = actors.tags.get(tag.as_bytes()) else {
            return Again::Whole;
        };
        // Its last observation, too, held none: Projects' completeness did not change since.
        let Some(inputs) = projects.as_ref() else {
            return Again::Same;
        };
        if (read && !held.fed) || !held.distinct || held.due {
            return Again::Whole;
        }
        let mut at = 0;
        let mut moved = false;
        for link in actors.links.get(&slot).into_iter().flatten() {
            for input in inputs.get(&link.id).into_iter().flatten() {
                let Some(progress) = held.progress.get_mut(at) else {
                    return Again::Whole;
                };
                moved |= *progress != input.facts.progress;
                *progress = input.facts.progress;
                at += 1;
            }
        }
        if at != held.progress.len() {
            Again::Whole
        } else if moved {
            Again::Progress((slot, country_id), held.progress.clone())
        } else {
            Again::Same
        }
    }

    /// The hour's read flag, the dictionary's digest where sent, and what it observed.
    fn project(
        &mut self,
        hour: &segments::Hour<'_, World>,
    ) -> io::Result<(bool, Option<Box<Dictionary>>, Option<Box<Facts>>)> {
        let state = hour.state;
        let world = hour.world;
        let at = hour.hour;
        let roster = world.countries()?;
        let opens = self.cold
            || hour.starts
            || hour.opens
            || self.last.and_then(|last| last.checked_add(1)) != Some(at);
        self.cold = false;
        let previous = self.last.replace(at);
        if opens {
            self.reset(hour.branch);
            // Before the branch's first hour nothing was open that the scan did not see.
            self.names.uncertain = at != self.start;
        }
        let complete = columns::complete(state, TableId::Projects);
        let read = complete
            && slot_read(
                self.format,
                &metrics(hour.manifest),
                TableId::Projects,
                at,
            );
        let dictionary = self.dictionary(state, world, opens)?;
        // The countries.
        let mut identity_changed = opens;
        if opens || roster.changed() {
            let actors = actors(roster)?;
            if self.actors.as_ref() != Some(&actors) {
                self.bind(actors);
                identity_changed = true;
            }
        }
        // The projects.
        let known = self.projects.is_some();
        let mut owners = std::mem::take(&mut self.moved);
        owners.clear();
        let mut reshaped = BTreeSet::new();
        if !complete {
            self.projects = None;
        } else if !known || world.restated(TableId::Projects) {
            self.read_projects(world, state, at)?;
        } else if changed(state, world, TableId::Projects)? {
            moved_owners(state, &mut owners)?;
            self.reread_projects(world, state, at, &owners, &mut reshaped)?;
        }
        // Their history: read whole where it opens or is complete anew, else edited at the rows
        // whose operands the hour may have changed.
        let history_known = self.history.is_some();
        let history_change = opens || changed(state, world, TableId::ProjectHistory)?;
        let mut before = None;
        let mut history_moved = std::mem::take(&mut self.history_moved);
        history_moved.clear();
        let mut reached = BTreeSet::new();
        // The owners whose completion the hour reached where its history changed: those it
        // otherwise leaves alone are observed where their rows changed.
        let mut due = BTreeSet::new();
        if history_change {
            let table = TableId::ProjectHistory;
            if !columns::complete(state, table) {
                before = self.history.take();
                self.completions.clear();
            } else if opens
                || self.history.is_none()
                || world.restated(table)
                || columns::changes(state, table)?.truncated
            {
                before = self.history.take();
                self.read_history(world, state)?;
            } else {
                self.edit_history(state, &mut history_moved)?;
            }
            if !opens {
                due = self.completed(previous, at);
            }
        } else {
            reached = self.completed(previous, at);
        }
        let history_flipped = history_known != self.history.is_some();
        // What names their projects: a field a look reads, or else a row it reads.
        let mut renamed = BTreeSet::new();
        let mut touched = BTreeSet::new();
        if !identity_changed {
            for (table, dependents) in [
                (TableId::ProjectPrograms, &self.programs),
                (TableId::Characters, &self.people),
                (TableId::CharacterPortraits, &self.people),
                (TableId::LexicalTokens, &self.tokens),
            ] {
                let fields = self.fields.read(table);
                Self::dependents(
                    (state, world),
                    table,
                    fields,
                    dependents,
                    &mut renamed,
                    &mut touched,
                )?;
            }
        }
        // Scoped names read the hour's world: every one is evaluated each hour, reused where
        // nothing it read changed, and only the countries whose names changed are observed.
        let mut named = BTreeMap::new();
        let mut names = Vec::new();
        let mut provisional = Vec::new();
        let mut scoped_countries = BTreeSet::new();
        if self.scoped
            && let (Some(content), Some(actors)) = (self.content.as_mut(), self.actors.as_ref())
        {
            let frame = world.frame(state, scoped_tables())?;
            let changes = Touched {
                state,
                world,
                opens,
            };
            let reuse = history::Reuse::Confirm(&changes);
            scoped_countries = self
                .names
                .observe(&frame, actors, read, content, reuse, &RUNNING)?
                .into_iter()
                .map(Arc::from)
                .collect();
            provisional = self.names.take_provisional();
            let except: BTreeSet<NameKey> =
                provisional.iter().map(|(key, _, _)| key.clone()).collect();
            let current = self.names.take_current();
            names = current.changes(&mut self.sent_names, &except);
            content.scoped.clear();
            content.scoped.push((hour.branch, Arc::new(current)));
            named = self.names.names.take_counts();
        }
        let full = identity_changed || known != self.projects.is_some();
        let mut affected: BTreeSet<Arc<str>> = if full {
            self.actors
                .iter()
                .flat_map(|actors| actors.tags.keys())
                .filter_map(|tag| std::str::from_utf8(tag).ok().map(Arc::from))
                .collect()
        } else {
            let mut affected = BTreeSet::new();
            let tag_of = |owner: &u32| self.owners.get(owner).cloned();
            if history_flipped {
                affected.extend(self.holders.iter().cloned());
                affected.extend(
                    self.projects
                        .iter()
                        .flat_map(|inputs| inputs.keys())
                        .filter_map(tag_of),
                );
            } else {
                affected.extend(reshaped.iter().filter_map(tag_of));
                affected.extend(history_moved.iter().filter_map(tag_of));
                touched.extend(owners.iter().filter_map(tag_of));
                if let (Some(before), Some(after)) = (&before, &self.history) {
                    let keys: BTreeSet<&(u32, u32)> = before.keys().chain(after.keys()).collect();
                    for key in keys {
                        if before.get(key) != after.get(key) {
                            affected.extend(tag_of(&key.0));
                        }
                    }
                }
            }
            affected.extend(reached.iter().filter_map(tag_of));
            affected.extend(renamed.iter().cloned());
            affected.extend(scoped_countries);
            affected
        };
        self.moved = owners;
        self.history_moved = history_moved;
        // The countries whose rows changed in nothing their last observations read but their
        // progress, in tag order.
        let mut progressed = Vec::new();
        if !full {
            let due: BTreeSet<Arc<str>> = due
                .iter()
                .filter_map(|owner| self.owners.get(owner).cloned())
                .collect();
            // Those this hour leaves alone are observed whole when next touched.
            for tag in due.iter().filter(|tag| !affected.contains(*tag) && !touched.contains(*tag)) {
                if let Some(held) = self.observed.get_mut(tag) {
                    held.due = true;
                }
            }
            for tag in &touched {
                if affected.contains(tag) {
                    continue;
                }
                let again = if due.contains(tag) {
                    Again::Whole
                } else {
                    self.again(tag, read)
                };
                match again {
                    Again::Whole => {
                        affected.insert(Arc::clone(tag));
                    }
                    Again::Same => {}
                    Again::Progress(identity, progress) => progressed.push(Observed {
                        tag: Arc::clone(tag),
                        identity,
                        seen: Seen::Progress(progress),
                    }),
                }
            }
        }
        let names_whole = opens && self.scoped;
        if affected.is_empty()
            && progressed.is_empty()
            && !full
            && named.is_empty()
            && names.is_empty()
            && provisional.is_empty()
            && !names_whole
        {
            return Ok((read, dictionary, None));
        }
        let mut frames = Frames {
            world,
            state,
            looks: None,
            hour: None,
        };
        let mut countries = Vec::with_capacity(affected.len() + progressed.len());
        let mut progressed = progressed.into_iter().peekable();
        for tag in &affected {
            while let Some(observed) = progressed.next_if(|observed| observed.tag < *tag) {
                countries.push(observed);
            }
            let renamed = renamed.contains(tag);
            countries.extend(self.observe(&mut frames, tag, (at, read), renamed)?);
        }
        countries.extend(progressed);
        Ok((
            read,
            dictionary,
            Some(Box::new(Facts {
                full,
                countries,
                named,
                names,
                names_whole,
                provisional,
                own: None,
            })),
        ))
    }

    fn heap(&self) -> usize {
        let projects = self.projects.as_ref().map_or(0, |inputs| {
            inputs
                .values()
                .map(|inputs| inputs.capacity() * size_of::<Input>())
                .sum::<usize>()
        });
        let history = self.history.as_ref().map_or(0, |history| {
            history
                .values()
                .map(|rows| 32 + rows.capacity() * size_of::<(u32, bool, i32)>())
                .sum::<usize>()
        });
        let dependents = |dependents: &Dependents| {
            dependents
                .values()
                .map(|tags| 32 + tags.len() * size_of::<Arc<str>>())
                .sum::<usize>()
        };
        projects
            + history
            + self.completions.len() * (size_of::<(i32, u32, u32, u32)>() + 16)
            + self.history_keys.capacity() * size_of::<u128>()
            + self.history_moved.capacity() * size_of::<u32>()
            + self.owners.len() * (8 + size_of::<Arc<str>>())
            + (self.naming.definitions.len() + self.naming.people.len()) * LOOK_BYTES
            + self.observed.len() * size_of::<Held>()
            + dependents(&self.programs)
            + dependents(&self.people)
            + dependents(&self.tokens)
            + self.sent_names.len() * 128
            + self.spelled.capacity() * 32
    }
}

/// What [`Projector::save`] writes: nothing but that it restarts, since everything it holds is
/// read again from the walker's tables, as at a segment's first hour.
const SAVED: &[u8] = b"projects projector: cold";

// Every segment starts cold: its first hour reads every country again; what came before is
// the emit's.
impl Operator<World> for Projector {
    type Output = Hour;

    fn start(&mut self, _: segments::Start<'_>, _: u64, _: i32) -> io::Result<()> {
        self.cold = true;
        Ok(())
    }

    fn hour(
        &mut self,
        hour: &segments::Hour<'_, World>,
        out: &mut dyn Sink<Hour>,
    ) -> io::Result<()> {
        let _cpu = replay_frame::cpu::scope("projects.columnar", Some("projects"));
        // The family's own consumer of the shared content ([`catalogue::own_refusals`]).
        let mut own = std::mem::take(&mut self.own);
        let projected = catalogue::own_refusals(&mut own, || self.project(hour));
        self.own = own;
        let (read, dictionary, mut facts) = projected?;
        // Only an observation or the name scan reads the content.
        if (facts.is_some() || self.scoped) && self.own != self.sent {
            self.sent = self.own.clone();
            facts.get_or_insert_with(Box::default).own = Some(self.own.clone());
        }
        out.push(Hour {
            branch: hour.branch,
            hour: hour.hour,
            read,
            dictionary,
            facts,
        })
    }

    fn carried(&self) -> Option<usize> {
        Some(self.heap())
    }

    fn save(&self, out: &mut Vec<u8>) -> io::Result<bool> {
        out.extend_from_slice(SAVED);
        Ok(true)
    }

    fn load(&mut self, bytes: &[u8]) -> io::Result<()> {
        if bytes != SAVED {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Not a saved projects projector",
            ));
        }
        self.cold = true;
        Ok(())
    }
}
