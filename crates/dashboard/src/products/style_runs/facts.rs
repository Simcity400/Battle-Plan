//! Country styles' facts, projected hour by hour on the walk's world ([`dashboard_world`]): each
//! tag's holder, its holder's map colour, the naming operands its name is built from and its
//! flag's candidate chain, handed only when one of them changed. The hour's changed rows reach
//! the tags they change: a country's rows reach the tag it holds, and a slot's naming operands
//! every tag whose name reaches that slot. A segment's first hour, one after a skipped hour, the
//! first known one after unknown hours and one whose inputs were read anew read every tag anew,
//! counted by cause ([`Rebuilds`]).

use crate::flag_choices::{self, Chain};
use crate::metadata::recorded_rows;
use crate::products::country_flags::FlagFolders;
use dashboard_world::{Flags, Hold, Needs, Roster, Tag, World};
use installed_content_country_names::country_names::{CountryNames, Operands};
use installed_content_localisation::localisation::Localisation;
use replay::{
    columns,
    segments::{self, Item, Operator, Sink, State},
};
use replay_data::rows::{Recorded, TableId, tables};
use replay_frame::frame::DecodedFrame;
use std::{
    collections::HashMap,
    io,
    mem::size_of,
    sync::{
        Arc, LazyLock,
        atomic::{AtomicU64, Ordering},
    },
};

/// The tables a projection reads.
pub const TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::Politics,
    TableId::PoliticalParties,
    TableId::Autonomy,
    TableId::ReservedCountryNaming,
];

/// The tables an hour's styles are known only while complete: the styles of an hour any of
/// them is not are a seek's.
const WHOLE: [TableId; 4] = [
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::ReservedCountryNaming,
];

/// The tables beside them whose completeness a name reads.
const PARTIAL: [TableId; 3] = [
    TableId::Politics,
    TableId::PoliticalParties,
    TableId::Autonomy,
];

/// The fields names and colours are read from, by table.
const FIELDS: [(TableId, &[&str]); 7] = [
    (TableId::Countries, &["canonical_slot", "tag"]),
    (TableId::CountryLinks, &["canonical_slot"]),
    (
        TableId::CountryAppearance,
        &[
            "red",
            "green",
            "blue",
            "cosmetic_tag",
            "name_tag",
            "leader_ideology",
            "overlord",
        ],
    ),
    (TableId::Politics, &["ruling_party"]),
    (TableId::PoliticalParties, &["token"]),
    (TableId::Autonomy, &["state"]),
    (
        TableId::ReservedCountryNaming,
        &[
            "tag",
            "name_tag",
            "cosmetic_tag",
            "leader_ideology",
            "group",
            "overlord",
            "autonomy",
            "built",
        ],
    ),
];

fn mask(table: TableId, names: &[&str]) -> io::Result<u64> {
    names.iter().try_fold(0, |mask, name| {
        Ok(mask | 1_u64 << columns::field(table, name)?)
    })
}

/// How the walk's frame holds the tables names and colours are read from.
pub(crate) fn asks() -> io::Result<Vec<(TableId, Hold)>> {
    FIELDS
        .iter()
        .map(|&(table, names)| Ok((table, Hold::Fields(mask(table, names)?).of(table))))
        .collect()
}

/// What a projection reads of its walk's world.
pub fn needs() -> io::Result<Needs> {
    Ok(Needs {
        countries: true,
        flags: true,
        frame: asks()?,
        ..Needs::default()
    })
}

/// Read for their operands alone: no name is built with it.
static UNNAMED: LazyLock<Localisation> = LazyLock::new(Localisation::default);

/// What a tag's name is built from: the naming operands of its holder's canonical slot and of
/// every overlord its name reaches, each once, and which of slot zero's names its reserved row
/// has built when the chain reaches slot zero.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Naming {
    pub chain: Vec<(u32, Result<Operands, String>)>,
    pub built: Option<u8>,
}

/// A tag's facts at an hour, while a country holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fact {
    /// The raw link id and canonical slot of its first country row, by id.
    pub holder: (u32, u32),
    /// The holder's recorded map colour; None without an appearance row.
    pub colour: Option<[u8; 3]>,
    pub naming: Arc<Naming>,
    pub flag: Arc<Chain>,
}

/// A tag's facts, None while no country holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Country {
    /// Its index in the projection ([`Hour::named`]).
    pub index: u32,
    pub fact: Option<Fact>,
}

/// One projected hour.
#[derive(Clone)]
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    /// Whether the tables of [`WHOLE`] were complete: unknown, the hour hands nothing.
    pub known: bool,
    /// Every named tag is handed: one left out is no country.
    pub whole: bool,
    /// Tags the projection numbers from this hour on, by index.
    pub named: Vec<(u32, Arc<str>)>,
    /// Tags whose facts changed, ascending by index.
    pub countries: Vec<Country>,
}

impl Item for Hour {
    fn heap(&self) -> usize {
        self.named.capacity() * size_of::<(u32, Arc<str>)>()
            + self.named.iter().map(|(_, tag)| tag.len()).sum::<usize>()
            + self.countries.capacity() * size_of::<Country>()
    }
}

struct Named {
    tag: Tag,
    /// What the projection last handed of it.
    handed: Option<Option<Fact>>,
    /// The slots its name reaches.
    reaches: Vec<u32>,
    /// The country object holding it.
    holder: Option<u32>,
}

/// The change masks of the fields read.
struct Fields {
    colour: u64,
    naming: u64,
    ruling: u64,
    token: u64,
    autonomy: u64,
    reserved: u64,
}

/// Why a projection reads every tag anew.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Anew {
    /// A segment's first hour, a later run's or one after a skipped hour.
    Start,
    /// The first known hour after unknown ones.
    Unknown,
    /// Politics, PoliticalParties or Autonomy became complete or stopped being.
    Partial,
    /// Links moved, or the roster or flag inputs were read anew.
    World,
}

/// The full rebuilds of every projection sharing it, by cause, for a run's measurement.
#[derive(Debug, Default)]
pub struct Rebuilds {
    starts: AtomicU64,
    unknown: AtomicU64,
    partial: AtomicU64,
    world: AtomicU64,
}

impl Rebuilds {
    fn count(&self, cause: Anew) {
        let counter = match cause {
            Anew::Start => &self.starts,
            Anew::Unknown => &self.unknown,
            Anew::Partial => &self.partial,
            Anew::World => &self.world,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    /// Each cause's rebuilds so far, by name.
    pub fn counts(&self) -> [(&'static str, u64); 4] {
        let load = |counter: &AtomicU64| counter.load(Ordering::Relaxed);
        [
            ("rebuilds at a start", load(&self.starts)),
            ("rebuilds after unknown hours", load(&self.unknown)),
            ("rebuilds as partial tables flip", load(&self.partial)),
            ("rebuilds as the world rebuilds", load(&self.world)),
        ]
    }
}

/// One segment's projection.
pub struct Projector {
    fields: Fields,
    asks: Vec<(TableId, Hold)>,
    numbers: HashMap<Tag, u32>,
    tags: Vec<Named>,
    /// Each canonical slot's naming operands.
    operands: HashMap<u32, Result<Operands, String>>,
    built: Option<u8>,
    /// Each overlord's subjects, by canonical slot.
    subjects: HashMap<u32, Vec<u32>>,
    /// Each country object's recorded map colour.
    colours: HashMap<u32, [u8; 3]>,
    /// The tags whose name reaches each slot.
    reaching: HashMap<u32, Vec<u32>>,
    /// The tag each country object holds.
    holding: HashMap<u32, u32>,
    /// Each country object's canonical slot as last read, so a removed one names its slot.
    slots: HashMap<u32, u32>,
    partial: Option<[bool; 3]>,
    last: Option<i32>,
    /// Why the next known hour reads every tag anew, the first cause met.
    anew: Option<Anew>,
    /// The next known hour hands every tag.
    fresh: bool,
    rebuilds: Arc<Rebuilds>,
    /// The recorded flag folders, whose choices this worker makes for the facts it hands on.
    flags: Option<Arc<FlagFolders>>,
}

/// Inserts `value` into `list` once.
fn add(list: &mut Vec<u32>, value: u32) {
    if !list.contains(&value) {
        list.push(value);
    }
}

/// Removes `value` from `map`'s list at `key`, and the list once empty.
fn remove(map: &mut HashMap<u32, Vec<u32>>, key: u32, value: u32) {
    if let Some(list) = map.get_mut(&key) {
        list.retain(|&held| held != value);
        if list.is_empty() {
            map.remove(&key);
        }
    }
}

/// The keys of `table`'s rows whose `mask` fields the hour wrote, or which it inserted or
/// deleted.
fn edited(state: &State, table: TableId, mask: u64) -> io::Result<Vec<u128>> {
    let changes = columns::changes(state, table)?;
    let held = columns::table(state, table)?;
    let mut keys = Vec::new();
    for (&slot, &written) in changes
        .changed
        .iter()
        .zip(&changes.masks)
        .chain(changes.fill_slots.iter().zip(&changes.fill_masks))
    {
        if written & mask != 0 {
            engine_allocation::reserve(&mut keys, 1)?;
            keys.extend(held.key(slot));
        }
    }
    engine_allocation::reserve(
        &mut keys,
        changes.inserted.len() + changes.deleted_keys.len(),
    )?;
    keys.extend(changes.inserted.iter().filter_map(|&slot| held.key(slot)));
    keys.extend_from_slice(&changes.deleted_keys);
    Ok(keys)
}

/// Which of slot zero's names its reserved row has built, as names read it.
fn built(frame: &DecodedFrame) -> Option<u8> {
    match frame.complete::<tables::ReservedCountryNaming>() {
        Ok([row]) => Some(row.built),
        _ => None,
    }
}

impl Projector {
    pub(super) fn new(
        rebuilds: Arc<Rebuilds>,
        flags: Option<Arc<FlagFolders>>,
    ) -> io::Result<Self> {
        let appearance = |names: &[&str]| mask(TableId::CountryAppearance, names);
        Ok(Self {
            fields: Fields {
                colour: appearance(&["red", "green", "blue"])?,
                naming: appearance(&["cosmetic_tag", "name_tag", "leader_ideology", "overlord"])?,
                ruling: mask(TableId::Politics, &["ruling_party"])?,
                token: mask(TableId::PoliticalParties, &["token"])?,
                autonomy: mask(TableId::Autonomy, &["state"])?,
                reserved: mask(TableId::ReservedCountryNaming, FIELDS[6].1)?,
            },
            asks: asks()?,
            numbers: HashMap::new(),
            tags: Vec::new(),
            operands: HashMap::new(),
            built: None,
            subjects: HashMap::new(),
            colours: HashMap::new(),
            reaching: HashMap::new(),
            holding: HashMap::new(),
            slots: HashMap::new(),
            partial: None,
            last: None,
            anew: Some(Anew::Start),
            fresh: true,
            rebuilds,
            flags,
        })
    }

    /// The index of `tag`, named in `named` when new; None for a tag that is not UTF-8, which
    /// no request can name.
    fn number(
        &mut self,
        roster: &Roster,
        tag: Tag,
        named: &mut Vec<(u32, Arc<str>)>,
    ) -> io::Result<Option<u32>> {
        if let Some(&index) = self.numbers.get(&tag) {
            return Ok(Some(index));
        }
        let Ok(text) = std::str::from_utf8(roster.tag(tag)) else {
            return Ok(None);
        };
        let index = u32::try_from(self.tags.len())
            .map_err(|_| crate::invalid("Country styles' tags exceed 32 bits"))?;
        let text: Arc<str> = Arc::from(text);
        engine_allocation::reserve(named, 1)?;
        named.push((index, text));
        engine_allocation::reserve_map(&mut self.numbers, 1)?;
        self.numbers.insert(tag, index);
        engine_allocation::reserve(&mut self.tags, 1)?;
        self.tags.push(Named {
            tag,
            handed: None,
            reaches: Vec::new(),
            holder: None,
        });
        Ok(Some(index))
    }

    /// Sets `slot`'s operands to `next`, keeping its overlord's subjects: whether they changed.
    fn set_operands(&mut self, slot: u32, next: Option<Result<Operands, String>>) -> bool {
        let previous = self.operands.get(&slot);
        if previous == next.as_ref() {
            return false;
        }
        let overlord = |operands: Option<&Result<Operands, String>>| {
            operands.and_then(|operands| operands.as_ref().ok()?.overlord)
        };
        let (old, new) = (overlord(previous), overlord(next.as_ref()));
        if old != new {
            if let Some(old) = old {
                remove(&mut self.subjects, old, slot);
            }
            if let Some(new) = new {
                add(self.subjects.entry(new).or_default(), slot);
            }
        }
        match next {
            Some(next) => self.operands.insert(slot, next),
            None => self.operands.remove(&slot),
        };
        true
    }

    /// Every slot's operands, colour and tag read anew from the hour's frame: every tag reached.
    fn rebuild(
        &mut self,
        world: &World,
        state: &State,
        roster: &Roster,
        named: &mut Vec<(u32, Arc<str>)>,
    ) -> io::Result<Vec<u32>> {
        let frame = world.frame_held(state, &self.asks)?;
        let (operands, built) = CountryNames::recorded_operands(&frame);
        self.operands.clear();
        self.subjects.clear();
        for (slot, operands) in operands {
            self.set_operands(slot, Some(operands));
        }
        self.built = built;
        self.colours.clear();
        for row in recorded_rows::<tables::CountryAppearance>(&frame)? {
            engine_allocation::reserve_map(&mut self.colours, 1)?;
            self.colours.insert(row.country, row.map_color());
        }
        self.slots.clear();
        for country in roster.iter() {
            engine_allocation::reserve_map(&mut self.slots, 1)?;
            self.slots.insert(country.id, country.slot);
        }
        for tag in roster.held() {
            self.number(roster, tag, named)?;
        }
        Ok((0..self.tags.len() as u32).collect())
    }

    /// The hour's changed rows read: the tags they reach.
    fn follow(
        &mut self,
        world: &World,
        state: &State,
        roster: &Roster,
        flags: &Flags,
        named: &mut Vec<(u32, Arc<str>)>,
    ) -> io::Result<Vec<u32>> {
        let mut reached = Vec::new();
        for &tag in roster.changed_tags() {
            if let Some(index) = self.number(roster, tag, named)? {
                reached.push(index);
            }
        }
        let keys = |table: TableId, mask: u64| -> io::Result<Vec<u128>> {
            if world.touched(table, mask) {
                edited(state, table, mask)
            } else {
                Ok(Vec::new())
            }
        };
        let colour = keys(TableId::CountryAppearance, self.fields.colour)?;
        let mut renamed: Vec<u32> = keys(TableId::CountryAppearance, self.fields.naming)?
            .into_iter()
            .chain(keys(TableId::Politics, self.fields.ruling)?)
            .chain(keys(TableId::Autonomy, self.fields.autonomy)?)
            .map(|key| key as u32)
            .chain(
                keys(TableId::PoliticalParties, self.fields.token)?
                    .into_iter()
                    .map(|key| (key >> 32) as u32),
            )
            .collect();
        let reserved = world.touched(TableId::ReservedCountryNaming, self.fields.reserved);
        // A country's own slot, its slot before and the subjects naming it read its rows.
        let mut dirty: Vec<u32> = Vec::new();
        for &id in roster.changed_ids() {
            let now = roster.get(id).map(|country| country.slot);
            let before = match now {
                Some(slot) => self.slots.insert(id, slot),
                None => self.slots.remove(&id),
            };
            for slot in [before, now].into_iter().flatten() {
                dirty.push(slot);
                dirty.extend(self.subjects.get(&slot).into_iter().flatten());
            }
        }
        renamed.sort_unstable();
        renamed.dedup();
        dirty.extend(renamed.iter().filter_map(|id| self.slots.get(id)));
        if reserved {
            dirty.push(0);
        }
        dirty.sort_unstable();
        dirty.dedup();
        let mut changed = Vec::new();
        if !dirty.is_empty() || !colour.is_empty() {
            let frame = world.frame_held(state, &self.asks)?;
            if !dirty.is_empty() {
                let names = CountryNames::read_selected(&frame, &UNNAMED, dirty.iter().copied());
                for &slot in &dirty {
                    if self.set_operands(slot, names.operands(slot).cloned()) {
                        changed.push(slot);
                    }
                }
            }
            if reserved {
                let built = built(&frame);
                if built != self.built {
                    self.built = built;
                    changed.push(0);
                }
            }
            let rows = recorded_rows::<tables::CountryAppearance>(&frame)?;
            for id in colour.into_iter().map(|key| key as u32) {
                let next = rows
                    .binary_search_by_key(&id, |row| row.country)
                    .ok()
                    .map(|at| rows[at].map_color());
                let previous = match next {
                    Some(next) => self.colours.insert(id, next),
                    None => self.colours.remove(&id),
                };
                if previous != next {
                    reached.extend(self.holding.get(&id));
                }
            }
        }
        for slot in changed {
            reached.extend(self.reaching.get(&slot).into_iter().flatten());
        }
        reached.extend(flags.changed().iter().filter_map(|id| self.holding.get(id)));
        Ok(reached)
    }

    /// The naming operands `slot`'s name reaches, and every slot it reaches.
    fn naming(&self, slot: u32) -> (Naming, Vec<u32>) {
        let mut chain = Vec::new();
        let mut reaches = Vec::new();
        let mut next = Some(slot);
        while let Some(slot) = next.filter(|slot| !reaches.contains(slot)) {
            reaches.push(slot);
            next = None;
            if let Some(operands) = self.operands.get(&slot) {
                next = operands
                    .as_ref()
                    .ok()
                    .and_then(|operands| operands.overlord);
                chain.push((slot, operands.clone()));
            }
        }
        let built = reaches.contains(&0).then_some(self.built).flatten();
        (Naming { chain, built }, reaches)
    }

    /// `tag`'s facts this hour, and the slots its name reaches.
    fn fact(&self, roster: &Roster, flags: &Flags, tag: Tag) -> (Option<Fact>, Vec<u32>) {
        let Some(country) = roster.holders(tag).next() else {
            return (None, Vec::new());
        };
        let (naming, reaches) = self.naming(country.slot);
        let flag = if (country.id as i32) <= 0 {
            // Atlas population tests the object's own country reference.
            Chain {
                candidates: Vec::new(),
                refusal: Some(format!("country {} names no country object", country.id)),
            }
        } else {
            let inputs = flags.inputs(country.id);
            let appearance = inputs
                .appearance
                .map(|appearance| flag_choices::Appearance {
                    cosmetic: roster.tag(appearance.cosmetic),
                    reference: appearance.reference,
                    original: match appearance.original {
                        Recorded::NotRecorded => Recorded::NotRecorded,
                        Recorded::Value(original) => {
                            Recorded::Value(original.map(|tag| roster.tag(tag)))
                        }
                    },
                });
            let token = inputs.group.map(|group| roster.tag(group));
            flag_choices::Operands::of(
                country.id,
                roster.tag(country.tag),
                appearance,
                inputs.ruling,
                token,
            )
            .chain()
        };
        let fact = Fact {
            holder: (country.id, country.slot),
            colour: self.colours.get(&country.id).copied(),
            naming: Arc::new(naming),
            flag: Arc::new(flag),
        };
        (Some(fact), reaches)
    }

    /// Keeps what tag `index` reaches and who holds it.
    fn track(&mut self, index: u32, fact: Option<&Fact>, reaches: Vec<u32>) -> io::Result<()> {
        let named = &mut self.tags[index as usize];
        let previous = std::mem::replace(&mut named.reaches, reaches);
        let holder = fact.map(|fact| fact.holder.0);
        let held = std::mem::replace(&mut named.holder, holder);
        for slot in previous {
            remove(&mut self.reaching, slot, index);
        }
        for &slot in &self.tags[index as usize].reaches {
            engine_allocation::reserve_map(&mut self.reaching, 1)?;
            add(self.reaching.entry(slot).or_default(), index);
        }
        if held != holder {
            if let Some(held) = held
                && self.holding.get(&held) == Some(&index)
            {
                self.holding.remove(&held);
            }
            if let Some(holder) = holder {
                engine_allocation::reserve_map(&mut self.holding, 1)?;
                self.holding.insert(holder, index);
            }
        }
        Ok(())
    }
}

impl Operator<World> for Projector {
    type Output = Hour;

    fn cross(&mut self, _: i32) -> io::Result<()> {
        self.anew.get_or_insert(Anew::Start);
        Ok(())
    }

    fn hour(
        &mut self,
        hour: &segments::Hour<'_, World>,
        out: &mut dyn Sink<Hour>,
    ) -> io::Result<()> {
        let (state, world) = (hour.state, hour.world);
        if hour.starts || self.last.and_then(|last| last.checked_add(1)) != Some(hour.hour) {
            self.anew.get_or_insert(Anew::Start);
            self.fresh = true;
        }
        self.last = Some(hour.hour);
        let mut projected = Hour {
            branch: hour.branch,
            hour: hour.hour,
            known: WHOLE.iter().all(|&table| columns::complete(state, table)),
            whole: false,
            named: Vec::new(),
            countries: Vec::new(),
        };
        if !projected.known {
            self.anew.get_or_insert(Anew::Unknown);
            return out.push(projected);
        }
        let roster = world.countries()?;
        let flags = world.flags()?;
        let partial = PARTIAL.map(|table| columns::complete(state, table));
        if self.partial != Some(partial) {
            self.anew.get_or_insert(Anew::Partial);
        }
        self.partial = Some(partial);
        // Every slot's links, and every input a rebuilt part read, may have changed.
        if roster.rebuilt() || flags.rebuilt() || !roster.changed_links().is_empty() {
            self.anew.get_or_insert(Anew::World);
        }
        let mut reached = match self.anew.take() {
            Some(cause) => {
                self.rebuilds.count(cause);
                self.rebuild(world, state, roster, &mut projected.named)?
            }
            None => self.follow(world, state, roster, flags, &mut projected.named)?,
        };
        projected.whole = std::mem::take(&mut self.fresh);
        reached.sort_unstable();
        reached.dedup();
        for index in reached {
            let tag = self.tags[index as usize].tag;
            let (fact, reaches) = self.fact(roster, flags, tag);
            self.track(index, fact.as_ref(), reaches)?;
            let named = &mut self.tags[index as usize];
            if projected.whole || named.handed.as_ref() != Some(&fact) {
                if let (Some(folders), Some(fact)) = (&self.flags, &fact) {
                    folders.warm(&fact.flag)?;
                }
                named.handed = Some(fact.clone());
                engine_allocation::reserve(&mut projected.countries, 1)?;
                projected.countries.push(Country { index, fact });
            }
        }
        out.push(projected)
    }
}
