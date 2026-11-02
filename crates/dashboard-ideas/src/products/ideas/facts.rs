//! Idea facts, projected hour by hour from a columnar walk ([`replay::segments`]): at each hour
//! that changes what a holding is read from, each changed country's held ideas and appointed
//! advisors as the row scan reads them, with its naming at the hour, read from the walk's frame
//! ([`dashboard_world`]); the rows no country can hold; and the recorded modifier definitions
//! where they changed. A segment's first hour, an hour after one not walked, one at which the
//! inputs become complete, and one that changes the countries or restates a table read every
//! country. What a country held before the segment is the ordered emit's ([`super::emit`]). An
//! hour that changes nothing read carries nothing but its hour.

use dashboard_base::products::{idea_format, idea_history::refusal};
use dashboard_text::identity::{NamingCountry, built_template, character_script_token};
use dashboard_world::{Hold, Needs, Roster, Tagged, World};
use installed_content_core::characters::BuiltTemplate;
use replay::{
    columns,
    segments::{self, Item, Operator, Sink, State},
};
use replay_data::rows::{CountryRow, ModifierDefinitionRow, TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io,
    mem::size_of,
    sync::Arc,
};

/// The tables a projection reads.
pub const TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::ActiveIdeas,
    TableId::HiredAdvisors,
    TableId::Characters,
    TableId::LexicalTokens,
    TableId::CountryAppearance,
    TableId::CountryCivilWars,
    TableId::CountryWars,
    TableId::WarRelations,
    TableId::CountryCounterInputs,
    TableId::ModifierDefinitions,
];

/// The tables a read takes from the walk's frame: holdings, holders and their spellings, the
/// naming operands a holder is named by (its appearance and whether it is in a civil war), and
/// the modifier definitions.
fn asks() -> io::Result<Vec<(TableId, Hold)>> {
    let mut asks = dashboard_world::identity()?;
    for table in [
        TableId::ActiveIdeas,
        TableId::HiredAdvisors,
        TableId::Characters,
        TableId::LexicalTokens,
        TableId::CountryAppearance,
        TableId::CountryCivilWars,
        TableId::CountryWars,
        TableId::WarRelations,
        TableId::ModifierDefinitions,
    ] {
        asks.push((table, Hold::Whole));
    }
    let owned = columns::field(TableId::CountryCounterInputs, "owned_state_count")?;
    asks.push((TableId::CountryCounterInputs, Hold::Fields(1u64 << owned)));
    Ok(asks)
}

/// What a projection reads of its walk's world: the countries, and the frame of its tables.
pub fn needs() -> io::Result<Needs> {
    Ok(Needs {
        countries: true,
        frame: asks()?,
        ..Needs::default()
    })
}

/// Why a recorded row cannot be held, as the feed names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Refusal {
    Tag,
    IdeaHolder,
    IdeaLink,
    AdvisorHolder,
    AdvisorLink,
    Seat,
    Token,
}

impl Refusal {
    pub fn text(self) -> &'static str {
        match self {
            Self::Tag => refusal::TAG,
            Self::IdeaHolder => refusal::IDEA_HOLDER,
            Self::IdeaLink => refusal::IDEA_LINK,
            Self::AdvisorHolder => refusal::ADVISOR_HOLDER,
            Self::AdvisorLink => refusal::ADVISOR_LINK,
            Self::Seat => refusal::SEAT,
            Self::Token => refusal::TOKEN,
        }
    }
}

/// A refused row's identity: its country, ordinal and the bytes refused.
pub type Row = (u32, u32, Box<[u8]>);

/// The seat and character an appointment records for its idea.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Role {
    pub seat: String,
    pub id: u32,
    /// The character's script token; empty when it does not resolve.
    pub token: String,
    /// The character's saved name as recorded.
    pub name: String,
}

/// An idea a country holds, with the appointment holding it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Holding {
    pub token: Arc<str>,
    pub role: Option<Arc<Role>>,
}

/// An appointed character's template as its row gives it, or that it has no row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Template {
    Built(Result<BuiltTemplate, String>),
    Missing { complete: bool },
}

/// A country's holdings at an hour, read as the row scan observes it.
pub struct Read {
    pub tag: Arc<str>,
    /// Its holder's canonical slot and id.
    pub identity: (u32, u32),
    /// Its held ideas, in token order.
    pub held: Vec<Holding>,
    pub refused: Vec<(Refusal, Row)>,
    /// Appointments with no idea token: character and seat.
    pub tokenless: Vec<(u32, Box<str>)>,
    /// Each appointed character's template, in appointment order.
    pub templates: Vec<(u32, Template)>,
    pub naming: NamingCountry,
}

impl Read {
    fn heap(&self) -> usize {
        size_of::<Self>()
            + self.tag.len()
            + self.held.capacity() * size_of::<Holding>()
            + self
                .held
                .iter()
                .map(|holding| {
                    holding.token.len()
                        + holding.role.as_ref().map_or(0, |role| {
                            size_of::<Role>() + role.seat.len() + role.token.len() + role.name.len()
                        })
                })
                .sum::<usize>()
            + self.refused.capacity() * size_of::<(Refusal, Row)>()
            + self
                .refused
                .iter()
                .map(|(_, row)| row.2.len())
                .sum::<usize>()
            + self.tokenless.capacity() * size_of::<(u32, Box<str>)>()
            + self.templates.capacity() * size_of::<(u32, Template)>()
            + self
                .naming
                .look
                .as_ref()
                .map_or(0, |look| look.name_tag.len() + look.cosmetic_tag.len() + 64)
    }
}

/// The recorded modifier definitions, by the folded name of each.
pub type Definitions = BTreeMap<String, ModifierDefinitionRow>;

/// What the ordered emit reads of an hour that changes what a holding is read from.
pub struct Facts {
    /// Whether Countries, CountryLinks, ActiveIdeas and HiredAdvisors are complete.
    pub known: bool,
    /// Every UTF-8 tag the countries hold, in tag order, with its holder's canonical slot and
    /// id, none where no link resolves it: at an hour reading every country.
    pub roster: Option<Arc<[(Arc<str>, Option<(u32, u32)>)]>>,
    /// The countries read, in tag order: every resolved one with `roster`.
    pub reads: Vec<Read>,
    /// Rows no country holds, and countries whose tag is not UTF-8.
    pub refused: Vec<(Refusal, Row)>,
    /// The modifier definitions, where they may have changed.
    pub definitions: Option<Arc<Definitions>>,
}

/// One hour of a segment.
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    /// None when the hour changes nothing read.
    pub facts: Option<Box<Facts>>,
}

impl Item for Hour {
    fn heap(&self) -> usize {
        let Some(facts) = &self.facts else {
            return 0;
        };
        size_of::<Facts>()
            + facts.reads.capacity() * size_of::<Read>()
            + facts.reads.iter().map(Read::heap).sum::<usize>()
            + facts.refused.capacity() * size_of::<(Refusal, Row)>()
            + facts.refused.iter().map(|(_, row)| row.2.len()).sum::<usize>()
            + facts.definitions.as_ref().map_or(0, |definitions| {
                definitions
                    .iter()
                    .map(|(key, row)| {
                        64 + key.len()
                            + row.name_key.len()
                            + row.postfix_key.len()
                            + row.argument.len()
                    })
                    .sum::<usize>()
            })
    }
}

/// The fields of the tables read by fields whose change makes a holding read again.
struct Fields {
    /// A character's script token, saved name and template.
    characters: u64,
    /// A country's name and cosmetic tags and its original tag.
    appearance: u64,
}

fn mask(table: TableId, names: &[&str]) -> io::Result<u64> {
    names
        .iter()
        .try_fold(0u64, |mask, name| {
            Ok(mask | 1u64 << columns::field(table, name)?)
        })
}

/// The keys of the rows the hour inserted or wrote any of `mask` in, and of those it deleted.
fn changed(state: &State, table: TableId, mask: u64) -> io::Result<(Vec<u128>, Vec<u128>)> {
    let held = columns::table(state, table)?;
    let changes = columns::changes(state, table)?;
    let mut live = Vec::new();
    for &slot in &changes.inserted {
        live.extend(held.key(slot));
    }
    let written = changes
        .changed
        .iter()
        .zip(&changes.masks)
        .chain(changes.fill_slots.iter().zip(&changes.fill_masks));
    for (&slot, &fields) in written {
        if fields & mask != 0 {
            live.extend(held.key(slot));
        }
    }
    Ok((live, changes.deleted_keys.to_vec()))
}

/// The rows of `rows`, sorted by owner, that `raw` owns.
fn owned<R>(rows: &[R], raw: u32, owner: impl Fn(&R) -> u32) -> &[R] {
    let first = rows.partition_point(|row| owner(row) < raw);
    let end = rows.partition_point(|row| owner(row) <= raw);
    &rows[first..end]
}

/// What a projection kept of a country it read: the characters and spellings its holdings read.
#[derive(Default)]
struct Reader {
    characters: Vec<u32>,
    spellings: Vec<u32>,
}

/// One segment's projection: every segment starts cold.
pub struct Projector {
    asks: Vec<(TableId, Hold)>,
    fields: Fields,
    cold: bool,
    last: Option<i32>,
    known: bool,
    /// The countries as last read whole.
    roster: Arc<[(Arc<str>, Option<(u32, u32)>)]>,
    /// The resolved tags at each canonical slot, by index in `roster`.
    slots: HashMap<u32, Vec<usize>>,
    /// What each tag's last read read, by index in `roster`, and each character's and
    /// spelling's readers.
    readers: Vec<Reader>,
    characters: HashMap<u32, Vec<usize>>,
    spellings: HashMap<u32, Vec<usize>>,
    /// The modifier definitions last handed, and the spellings they name.
    definitions: Option<Arc<Definitions>>,
    named: HashSet<u32>,
}

/// What [`Projector::save`] writes: nothing but that it restarts, since everything it holds is
/// read again from the walker's tables, as at a segment's first hour.
const SAVED: &[u8] = b"ideas projector: cold";

impl Projector {
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            asks: asks()?,
            fields: Fields {
                characters: mask(
                    TableId::Characters,
                    &[
                        "name_token_id",
                        "token",
                        "template_present",
                        "template_tag",
                        "template_file",
                        "template_line",
                        "template_generated",
                    ],
                )?,
                appearance: mask(
                    TableId::CountryAppearance,
                    &[
                        "cosmetic_tag",
                        "name_tag",
                        "original_tag",
                        "original_tag_reference",
                    ],
                )?,
            },
            cold: true,
            last: None,
            known: false,
            roster: Arc::from(Vec::new()),
            slots: HashMap::new(),
            readers: Vec::new(),
            characters: HashMap::new(),
            spellings: HashMap::new(),
            definitions: None,
            named: HashSet::new(),
        })
    }

    /// The countries `roster` holds, read anew: every tag's readers are forgotten.
    fn countries(&mut self, roster: &Roster) {
        let mut tags = Vec::new();
        for tag in roster.held() {
            let Ok(text) = std::str::from_utf8(roster.tag(tag)) else {
                continue;
            };
            let holder = match roster.tagged(tag) {
                Tagged::Found(country) => Some((country.slot, country.id)),
                Tagged::Absent | Tagged::Unresolved => None,
            };
            tags.push((Arc::<str>::from(text), holder));
        }
        tags.sort_unstable_by(|left, right| left.0.cmp(&right.0));
        self.slots.clear();
        for (index, (_, holder)) in tags.iter().enumerate() {
            if let Some((slot, _)) = holder {
                self.slots.entry(*slot).or_default().push(index);
            }
        }
        self.readers = tags.iter().map(|_| Reader::default()).collect();
        self.characters.clear();
        self.spellings.clear();
        self.roster = Arc::from(tags);
    }

    /// The tags whose holder the links resolve `raw` to.
    fn owners(&self, roster: &Roster, raw: u32, dirty: &mut BTreeSet<usize>) {
        if let Some(tags) = roster.slot_of(raw).and_then(|slot| self.slots.get(&slot)) {
            dirty.extend(tags.iter().copied());
        }
    }

    /// Why a row owned by `raw` cannot be held, if it cannot.
    fn unheld(&self, roster: &Roster, raw: u32, holder: Refusal, link: Refusal) -> Option<Refusal> {
        match roster.slot_of(raw) {
            Some(slot) if self.slots.contains_key(&slot) => None,
            Some(_) => Some(holder),
            None => Some(link),
        }
    }

    /// The tag at `index`'s holdings, read from `frame` as the row scan observes them.
    fn read(&mut self, frame: &DecodedFrame, roster: &Roster, index: usize) -> io::Result<Read> {
        let (tag, holder) = &self.roster[index];
        let (slot, id) = holder.ok_or_else(|| io::Error::other("An unresolved tag was read"))?;
        let tag = Arc::clone(tag);
        let ideas = frame.complete::<tables::ActiveIdeas>()?;
        let advisors = frame.complete::<tables::HiredAdvisors>()?;
        let characters = frame.complete::<tables::Characters>().ok();
        let lexical = frame.complete::<tables::LexicalTokens>().ok();
        let mut reader = Reader::default();
        let mut refused = Vec::new();
        let mut tokenless = Vec::new();
        let mut templates: Vec<(u32, Template)> = Vec::new();
        let mut roles: BTreeMap<&[u8], Arc<Role>> = BTreeMap::new();
        let mut held: BTreeSet<&[u8]> = BTreeSet::new();
        for &raw in roster.aliases(slot) {
            for advisor in owned(advisors, raw, |row| row.country) {
                let Ok(seat) = std::str::from_utf8(&advisor.slot) else {
                    refused.push((Refusal::Seat, (id, advisor.ordinal, advisor.slot.clone().into())));
                    continue;
                };
                if advisor.idea.is_empty() {
                    tokenless.push((advisor.character, seat.into()));
                    continue;
                }
                let row = characters.and_then(|rows| {
                    rows.binary_search_by_key(&advisor.character, |character| character.id)
                        .ok()
                        .map(|at| &rows[at])
                });
                reader.characters.push(advisor.character);
                if let Some(row) = row {
                    reader.spellings.extend(u32::try_from(row.name_token_id).ok());
                    if row.template_present != 0 {
                        reader.spellings.push(row.template_tag);
                    }
                }
                let token = row
                    .zip(lexical)
                    .and_then(|(row, lexical)| character_script_token(row, lexical))
                    .unwrap_or_default();
                let name = row
                    .and_then(|row| std::str::from_utf8(&row.token).ok())
                    .unwrap_or_default();
                if !templates.iter().any(|(held, _)| *held == advisor.character) {
                    let template = match row {
                        Some(row) => Template::Built(built_template(row, lexical)),
                        None => Template::Missing {
                            complete: characters.is_some(),
                        },
                    };
                    templates.push((advisor.character, template));
                }
                roles.insert(
                    advisor.idea.as_slice(),
                    Arc::new(Role {
                        seat: seat.to_owned(),
                        id: advisor.character,
                        token: token.to_owned(),
                        name: name.to_owned(),
                    }),
                );
            }
            for idea in owned(ideas, raw, |row| row.country) {
                if !idea.token.is_empty() {
                    held.insert(idea.token.as_slice());
                }
            }
        }
        held.extend(roles.keys().copied());
        let mut holdings = Vec::with_capacity(held.len());
        for token in held {
            match std::str::from_utf8(token) {
                Ok(text) => holdings.push(Holding {
                    token: Arc::from(text),
                    role: roles.get(token).cloned(),
                }),
                Err(_) => refused.push((Refusal::Token, (id, 0, token.into()))),
            }
        }
        let country = CountryRow {
            id,
            canonical_slot: slot,
            tag: tag.as_bytes().to_vec(),
            ..Default::default()
        };
        let appearance = frame
            .complete::<tables::CountryAppearance>()
            .ok()
            .and_then(|rows| {
                rows.binary_search_by_key(&id, |row| row.country)
                    .ok()
                    .map(|at| &rows[at])
            });
        let naming = NamingCountry::read_with_appearance(frame, &country, appearance);
        self.keep(index, reader);
        Ok(Read {
            tag,
            identity: (slot, id),
            held: holdings,
            refused,
            tokenless,
            templates,
            naming,
        })
    }

    /// Keeps `reader` as what the tag at `index` reads.
    fn keep(&mut self, index: usize, reader: Reader) {
        let held = std::mem::replace(&mut self.readers[index], reader);
        for (users, keys) in [
            (&mut self.characters, &held.characters),
            (&mut self.spellings, &held.spellings),
        ] {
            for key in keys {
                if let Some(readers) = users.get_mut(key) {
                    readers.retain(|&reader| reader != index);
                    if readers.is_empty() {
                        users.remove(key);
                    }
                }
            }
        }
        let now = &self.readers[index];
        for (users, keys) in [
            (&mut self.characters, &now.characters),
            (&mut self.spellings, &now.spellings),
        ] {
            for &key in keys {
                let readers = users.entry(key).or_default();
                if !readers.contains(&index) {
                    readers.push(index);
                }
            }
        }
    }

    /// The modifier definitions `frame` records, where they differ from those last handed.
    fn definitions(&mut self, frame: &DecodedFrame) -> Option<Arc<Definitions>> {
        let (Ok(rows), Ok(lexical)) = (
            frame.complete::<tables::ModifierDefinitions>(),
            frame.complete::<tables::LexicalTokens>(),
        ) else {
            return None;
        };
        self.named = rows.iter().map(|row| row.token).collect();
        let index = idea_format::index_recorded(rows, lexical);
        if self.definitions.as_deref() == Some(&index) {
            return None;
        }
        let index = Arc::new(index);
        self.definitions = Some(Arc::clone(&index));
        Some(index)
    }

    fn heap(&self) -> usize {
        self.roster.len() * size_of::<(Arc<str>, Option<(u32, u32)>)>()
            + self.roster.iter().map(|(tag, _)| tag.len()).sum::<usize>()
            + self.slots.capacity() * (size_of::<(u32, Vec<usize>)>() + size_of::<usize>())
            + self.readers.capacity() * size_of::<Reader>()
            + self
                .readers
                .iter()
                .map(|reader| (reader.characters.capacity() + reader.spellings.capacity()) * 4)
                .sum::<usize>()
            + (self.characters.capacity() + self.spellings.capacity())
                * (size_of::<(u32, Vec<usize>)>() + size_of::<usize>())
            + self.named.capacity() * 4
            + self.definitions.as_ref().map_or(0, |definitions| definitions.len() * 160)
    }
}

/// The tables whose restatement makes every country read again.
const RESTATING: [TableId; 5] = [
    TableId::ActiveIdeas,
    TableId::HiredAdvisors,
    TableId::Characters,
    TableId::LexicalTokens,
    TableId::CountryAppearance,
];

// Every segment starts cold: its first hour reads every country again; what came before is the
// emit's.
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
        let _cpu = replay_frame::cpu::scope("ideas.columnar", Some("ideas"));
        let (state, world, at) = (hour.state, hour.world, hour.hour);
        let roster = world.countries()?;
        let opens = self.cold
            || hour.starts
            || hour.opens
            || self.last.and_then(|last| last.checked_add(1)) != Some(at);
        if opens {
            self.definitions = None;
            self.named.clear();
        }
        self.cold = false;
        self.last = Some(at);
        let known = roster.complete()
            && columns::complete(state, TableId::ActiveIdeas)
            && columns::complete(state, TableId::HiredAdvisors);
        let restated = opens || known != self.known;
        let whole = known
            && (restated
                || roster.changed()
                || RESTATING.iter().any(|&table| world.restated(table)));
        self.known = known;
        let lexical = TableId::LexicalTokens;
        let mut spellings = Vec::new();
        if !opens && world.touched(lexical, u64::MAX) {
            let (live, deleted) = changed(state, lexical, u64::MAX)?;
            spellings = live;
            spellings.extend(deleted);
        }
        let definitions = opens
            || world.restated(TableId::ModifierDefinitions)
            || world.restated(lexical)
            || world.touched(TableId::ModifierDefinitions, u64::MAX)
            || spellings
                .iter()
                .any(|&key| u32::try_from(key).is_ok_and(|token| self.named.contains(&token)));
        let mut dirty = BTreeSet::new();
        let mut written = [Vec::new(), Vec::new()];
        if known && !whole {
            for (which, table) in [TableId::ActiveIdeas, TableId::HiredAdvisors]
                .into_iter()
                .enumerate()
            {
                if !world.touched(table, u64::MAX) {
                    continue;
                }
                let (live, deleted) = changed(state, table, u64::MAX)?;
                for &key in live.iter().chain(&deleted) {
                    self.owners(roster, (key >> 32) as u32, &mut dirty);
                }
                written[which] = live;
            }
            if world.touched(TableId::Characters, self.fields.characters) {
                let (live, deleted) = changed(state, TableId::Characters, self.fields.characters)?;
                for key in live.iter().chain(&deleted) {
                    if let Some(readers) = u32::try_from(*key).ok().and_then(|id| self.characters.get(&id))
                    {
                        dirty.extend(readers.iter().copied());
                    }
                }
            }
            for key in &spellings {
                if let Some(readers) = u32::try_from(*key).ok().and_then(|token| self.spellings.get(&token))
                {
                    dirty.extend(readers.iter().copied());
                }
            }
            if world.touched(TableId::CountryAppearance, self.fields.appearance) {
                let (live, deleted) =
                    changed(state, TableId::CountryAppearance, self.fields.appearance)?;
                for key in live.iter().chain(&deleted) {
                    let Some(country) = u32::try_from(*key).ok().and_then(|id| roster.get(id))
                    else {
                        continue;
                    };
                    if let Some(tags) = self.slots.get(&country.slot) {
                        dirty.extend(tags.iter().copied());
                    }
                }
            }
        }
        let busy = whole || !dirty.is_empty() || written.iter().any(|keys| !keys.is_empty());
        if !restated && !busy && !definitions {
            return out.push(Hour {
                branch: hour.branch,
                hour: at,
                facts: None,
            });
        }
        let frame = world.frame_held(state, &self.asks)?;
        let definitions = if definitions {
            self.definitions(&frame)
        } else {
            None
        };
        let mut facts = Facts {
            known,
            roster: None,
            reads: Vec::new(),
            refused: Vec::new(),
            definitions,
        };
        if known {
            if whole {
                self.countries(roster);
                facts.roster = Some(Arc::clone(&self.roster));
                for country in roster.iter() {
                    let tag = roster.tag(country.tag);
                    if std::str::from_utf8(tag).is_err() {
                        facts
                            .refused
                            .push((Refusal::Tag, (country.id, country.slot, tag.into())));
                    }
                }
            }
            let ideas = frame.complete::<tables::ActiveIdeas>()?;
            let advisors = frame.complete::<tables::HiredAdvisors>()?;
            let idea = |row: &replay_data::rows::ActiveIdeaRow| (row.country, row.ordinal);
            let advisor = |row: &replay_data::rows::HiredAdvisorRow| (row.country, row.ordinal);
            let refused = &mut facts.refused;
            let mut refuse_idea = |this: &Self, row: &replay_data::rows::ActiveIdeaRow| {
                if let Some(why) =
                    this.unheld(roster, row.country, Refusal::IdeaHolder, Refusal::IdeaLink)
                {
                    refused.push((why, (row.country, row.ordinal, row.token.clone().into())));
                }
            };
            if whole {
                for row in ideas {
                    refuse_idea(self, row);
                }
            } else {
                for &key in &written[0] {
                    let key = ((key >> 32) as u32, key as u32);
                    if let Ok(at) = ideas.binary_search_by_key(&key, idea) {
                        refuse_idea(self, &ideas[at]);
                    }
                }
            }
            let mut refuse_advisor = |this: &Self, row: &replay_data::rows::HiredAdvisorRow| {
                if let Some(why) = this.unheld(
                    roster,
                    row.country,
                    Refusal::AdvisorHolder,
                    Refusal::AdvisorLink,
                ) {
                    refused.push((why, (row.country, row.ordinal, row.idea.clone().into())));
                }
            };
            if whole {
                for row in advisors {
                    refuse_advisor(self, row);
                }
            } else {
                for &key in &written[1] {
                    let key = ((key >> 32) as u32, key as u32);
                    if let Ok(at) = advisors.binary_search_by_key(&key, advisor) {
                        refuse_advisor(self, &advisors[at]);
                    }
                }
            }
            let indices: Vec<usize> = if whole {
                (0..self.roster.len()).collect()
            } else {
                dirty.into_iter().collect()
            };
            for index in indices {
                if self.roster[index].1.is_some() {
                    facts.reads.push(self.read(&frame, roster, index)?);
                }
            }
        }
        drop(frame);
        out.push(Hour {
            branch: hour.branch,
            hour: at,
            facts: Some(Box::new(facts)),
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
                "Not a saved ideas projector",
            ));
        }
        *self = Self::new()?;
        Ok(())
    }
}
