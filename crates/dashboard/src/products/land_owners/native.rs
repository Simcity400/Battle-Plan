//! The catalogue's native backend. Each segment's operator follows the walk's world, whose owners
//! say which canonical slots began or stopped owning states and whose roster says which countries
//! changed: an hour that moved neither is read no further. It hands its branch only the countries
//! that began claiming owned land, every claimant at the segment's first hour, each named then.
//! The branch keeps each tag and slot's first witness and publishes them as its page; a reply
//! merges every branch's page as the row scan merges its frames ([`catalogue`]).

use super::{NAMES_UNRESOLVED, OWNERSHIP_INCOMPLETE};
use crate::{CountryMetadata, LandOwners, QueryError, metadata::short_text, products::style_runs};
use dashboard_base::prepared::{
    pages::Pages,
    roots::{Frontier, Held},
    series::{compress, decompress},
};
use dashboard_world::{Hold, Needs, Roster, Tag, World};
use installed_content_country_names::country_names::CountryNames;
use installed_content_localisation::localisation::Localisation;
use replay::{
    columns,
    segments::{self, Item, Operator, Sink, State},
};
use replay_data::{
    format::FormatVersion,
    rows::{StateRow, TableId},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io,
    mem::size_of,
    sync::Arc,
};

#[cfg(test)]
mod tests;

/// The job's name: its coverage, gate and pages are named after it.
pub(crate) const NAME: &str = "land-owners";

/// Where the catalogue stands on a branch.
pub(crate) enum Readiness {
    /// Still to prepare there, or to prepare again.
    Pending,
    /// Prepared through its last recorded hour, or ended where the recording could not be read.
    Done { incomplete: bool },
    Refused(QueryError),
}

/// Where the catalogue stands on `branch`, whose last recorded hour is `through`: refused as its
/// coverage page or its publication's end refuses it, else as its publication of the plan's
/// version reaches.
pub(crate) fn readiness(
    branch: u64,
    refusal: Option<(i32, QueryError)>,
    frontier: Option<&Frontier>,
    through: i32,
) -> Readiness {
    let ended = frontier
        .and_then(|frontier| frontier.end.as_ref())
        .filter(|end| end.held == Held::Refused)
        .map(|end| (end.hour, end.reason.clone()));
    if let Some((hour, error)) = refusal.or(ended) {
        return Readiness::Refused(error.context(&format!(
            "Land owners were refused on branch {branch} from hour {hour}"
        )));
    }
    let Some(frontier) = frontier else {
        return Readiness::Pending;
    };
    if frontier.end.is_none() && !(frontier.complete && frontier.last_hour >= through) {
        return Readiness::Pending;
    }
    Readiness::Done {
        incomplete: frontier.end.as_ref().is_some_and(|end| end.held == Held::Lost)
            || frontier.first_hole.is_some(),
    }
}

/// A branch's page of witnesses.
pub(crate) fn section(branch: u64) -> String {
    format!("{NAME}-{branch}")
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

const OWNED: &[TableId] = &[TableId::Countries, TableId::CountryLinks, TableId::States];
const NAMED: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::States,
    TableId::CountryAppearance,
    TableId::Politics,
    TableId::PoliticalParties,
    TableId::Autonomy,
    TableId::ReservedCountryNaming,
];

/// The tables its operators read: those names are read from too while countries are `named`.
pub(crate) fn tables(named: bool) -> &'static [TableId] {
    if named { NAMED } else { OWNED }
}

/// What its operators read of the walk's world: the countries and the states' owners, and, while
/// countries are `named`, the frame country styles name them from, held as they hold it.
pub(crate) fn needs(named: bool) -> io::Result<Needs> {
    Ok(Needs {
        countries: true,
        owners: true,
        frame: if named {
            style_runs::facts::asks()?
        } else {
            Vec::new()
        },
        ..Needs::default()
    })
}

/// A country that began claiming owned land: of an owning canonical slot's countries the links
/// resolve, the first by id holding a tag, with its display tag's name at that hour.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Found {
    pub slot: u32,
    pub tag: Box<[u8]>,
    pub name: String,
    /// Its name could not be resolved, and is its tag.
    pub unresolved: bool,
}

/// One hour of a segment.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Hour {
    pub hour: i32,
    /// Ascending by slot.
    pub found: Vec<Found>,
    /// What the hour read leaves some ownership unknown.
    pub incomplete: bool,
}

impl Item for Hour {
    fn heap(&self) -> usize {
        self.found.capacity() * size_of::<Found>()
            + self
                .found
                .iter()
                .map(|found| found.tag.len() + found.name.capacity())
                .sum::<usize>()
    }
}

/// One segment's projection: every segment starts cold.
pub(crate) struct Projector {
    format: FormatVersion,
    names: Option<Arc<Localisation>>,
    asks: Vec<(TableId, Hold)>,
    owner: usize,
    cold: bool,
    /// The hour before read a truncated States: the next reads every owner again.
    partial: bool,
    /// Each owning canonical slot, with the tag of the country claiming it; none where none does.
    owning: HashMap<u32, Option<Tag>>,
}

impl Projector {
    /// A projection naming countries in `names`; without them each is named by its tag.
    pub(crate) fn new(format: FormatVersion, names: Option<Arc<Localisation>>) -> io::Result<Self> {
        Ok(Self {
            format,
            asks: needs(names.is_some())?.frame,
            names,
            owner: columns::field(TableId::States, "owner")?,
            cold: true,
            partial: false,
            owning: HashMap::new(),
        })
    }

    /// The owner of the state at truncated States `slot`, a raw link id.
    fn owner_at(&self, state: &State, slot: u32) -> io::Result<u32> {
        let held = columns::table(state, TableId::States)?;
        if !held.recorded(self.owner, slot) {
            let row: StateRow =
                columns::row(state, TableId::States, slot, self.format.version())?;
            return Ok(row.owner);
        }
        columns::plain::<u32>(held, self.owner)?
            .get(slot as usize)
            .copied()
            .ok_or_else(|| invalid(format!("States slot {slot} holds no cell")))
    }

    /// Claims `slot` again: whether a country now claims it that did not before, and whether
    /// none does.
    fn claim(&mut self, roster: &Roster, slot: u32, before: Option<Tag>) -> (Option<Tag>, bool) {
        let now = roster
            .at_slot(slot)
            .find(|country| country.resolved && !roster.tag(country.tag).is_empty())
            .map(|country| country.tag);
        self.owning.insert(slot, now);
        let found = now.filter(|&tag| before != Some(tag));
        (found, now.is_none())
    }

    /// Every owning slot read anew, from the truncated rows when States is `truncated`.
    fn whole(
        &mut self,
        state: &State,
        world: &World,
        truncated: Option<&[u32]>,
        found: &mut Vec<(u32, Tag)>,
    ) -> io::Result<bool> {
        let (roster, owners) = (world.countries()?, world.owners()?);
        let before = std::mem::take(&mut self.owning);
        let mut slots: Vec<u32> = Vec::new();
        let mut incomplete = roster.iter().any(|country| !country.resolved);
        match truncated {
            Some(rows) => {
                incomplete = true;
                for &row in rows {
                    let raw = self.owner_at(state, row)?;
                    match roster.slot_of(raw) {
                        Some(slot) => slots.push(slot),
                        None => incomplete |= raw != 0,
                    }
                }
            }
            None => {
                incomplete |= owners.unlinked() > 0;
                slots.extend(owners.slots());
            }
        }
        slots.sort_unstable();
        slots.dedup();
        for slot in slots {
            let before = before.get(&slot).copied().flatten();
            let (new, unclaimed) = self.claim(roster, slot, before);
            incomplete |= unclaimed;
            found.extend(new.map(|tag| (slot, tag)));
        }
        Ok(incomplete)
    }

    /// The hour's moved owners and changed countries: the slots they reach claimed again.
    fn follow(&mut self, world: &World, found: &mut Vec<(u32, Tag)>) -> io::Result<bool> {
        let (roster, owners) = (world.countries()?, world.owners()?);
        let mut incomplete = owners.unlinked() > 0;
        let mut slots = Vec::new();
        for &slot in owners.reached() {
            if owners.owned(slot) > 0 {
                self.owning.entry(slot).or_insert(None);
                slots.push(slot);
            } else {
                self.owning.remove(&slot);
            }
        }
        if roster.changed() {
            incomplete |= roster
                .changed_ids()
                .iter()
                .any(|&id| roster.get(id).is_some_and(|country| !country.resolved));
            slots = self.owning.keys().copied().collect();
            slots.sort_unstable();
        }
        for slot in slots {
            let before = self.owning.get(&slot).copied().flatten();
            let (new, unclaimed) = self.claim(roster, slot, before);
            incomplete |= unclaimed;
            found.extend(new.map(|tag| (slot, tag)));
        }
        Ok(incomplete)
    }

    /// `found`'s countries with their names at the hour.
    fn named(
        &self,
        world: &World,
        state: &State,
        found: Vec<(u32, Tag)>,
    ) -> io::Result<Vec<Found>> {
        let roster = world.countries()?;
        let frame = match &self.names {
            Some(_) if !found.is_empty() => Some(world.frame_held(state, &self.asks)?),
            _ => None,
        };
        let names = match (&self.names, &frame) {
            (Some(localisation), Some(frame)) => Some(CountryNames::read_selected(
                frame,
                localisation,
                found.iter().map(|&(slot, _)| slot),
            )),
            _ => None,
        };
        let mut named = engine_allocation::vec(found.len())?;
        for (slot, tag) in found {
            let bytes = roster.tag(tag);
            let shown = short_text(&String::from_utf8_lossy(bytes), 512);
            let (name, unresolved) = match names.as_ref().map(|names| names.name(slot)) {
                Some(Ok(name)) => (short_text(&name, 512), false),
                Some(Err(_)) => (shown, true),
                None => (shown, false),
            };
            named.push(Found {
                slot,
                tag: engine_allocation::copy_vec(bytes)?.into_boxed_slice(),
                name,
                unresolved,
            });
        }
        Ok(named)
    }
}

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
        let world = hour.world;
        let (roster, owners) = (world.countries()?, world.owners()?);
        let truncated = columns::truncated(hour.state, TableId::States);
        // Nothing is witnessed while the countries, their links or the states are not held.
        if !roster.complete() || !(owners.complete() || truncated.is_some()) {
            self.owning.clear();
            self.cold = true;
            return out.push(Hour {
                hour: hour.hour,
                found: Vec::new(),
                incomplete: true,
            });
        }
        let whole = std::mem::take(&mut self.cold)
            | std::mem::replace(&mut self.partial, truncated.is_some())
            | truncated.is_some()
            | roster.rebuilt()
            | owners.rebuilt();
        let mut found = Vec::new();
        let incomplete = if whole {
            self.whole(hour.state, world, truncated, &mut found)?
        } else if owners.moved().is_empty() && owners.reached().is_empty() && !roster.changed() {
            false
        } else {
            self.follow(world, &mut found)?
        };
        found.sort_unstable_by_key(|&(slot, _)| slot);
        let found = self.named(world, hour.state, found)?;
        out.push(Hour {
            hour: hour.hour,
            found,
            incomplete,
        })
    }
}

/// A country's first ownership observed on a branch, named then.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Witness {
    pub slot: u32,
    pub tag: Vec<u8>,
    pub hour: i32,
    pub name: String,
    pub unresolved: bool,
}

/// A branch's published page: its witnesses in the order first seen, and whether ownership went
/// unknown somewhere.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Page {
    pub incomplete: bool,
    pub witnesses: Vec<Witness>,
}

impl Page {
    pub(crate) fn bytes(&self) -> io::Result<Vec<u8>> {
        compress(serde_json::to_vec(self)?.as_slice(), 3)
    }

    pub(crate) fn read(bytes: &[u8]) -> io::Result<Self> {
        Ok(serde_json::from_slice(&decompress(bytes)?)?)
    }
}

/// A branch's state: its page, each witness's tag and slot, and whether the page changed since
/// it was last published.
pub(crate) struct Branch {
    page: Page,
    known: HashSet<(Box<[u8]>, u32)>,
    dirty: bool,
}

impl Branch {
    pub(crate) fn new() -> Self {
        Self {
            page: Page::default(),
            known: HashSet::new(),
            dirty: true,
        }
    }

    /// The branch going on from its published page.
    pub(crate) fn resume(pages: &mut dyn Pages, branch: u64) -> io::Result<Self> {
        let bytes = pages
            .read_at(&section(branch), 0)?
            .ok_or_else(|| invalid("Its published page is missing"))?;
        let page = Page::read(&bytes)?;
        let known = page
            .witnesses
            .iter()
            .map(|witness| (witness.tag.clone().into_boxed_slice(), witness.slot))
            .collect();
        Ok(Self {
            page,
            known,
            dirty: false,
        })
    }

    pub(crate) fn observe(&mut self, hour: Hour) {
        if hour.incomplete && !self.page.incomplete {
            self.page.incomplete = true;
            self.dirty = true;
        }
        for found in hour.found {
            if self.known.contains(&(found.tag.clone(), found.slot)) {
                continue;
            }
            self.page.witnesses.push(Witness {
                slot: found.slot,
                tag: found.tag.to_vec(),
                hour: hour.hour,
                name: found.name,
                unresolved: found.unresolved,
            });
            self.known.insert((found.tag, found.slot));
            self.dirty = true;
        }
    }

    /// Writes its page when it changed.
    pub(crate) fn publish(&mut self, pages: &mut dyn Pages, branch: u64) -> io::Result<()> {
        if !self.dirty {
            return Ok(());
        }
        pages.write_at(&section(branch), 0, &self.page.bytes()?)?;
        self.dirty = false;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn page(&self) -> &Page {
        &self.page
    }
}

/// The catalogue of every branch's page, as the row scan builds it from the recording's blocks
/// in their order (`blocks`, each block's branch and hours): a tag and slot is listed once, as
/// its lowest branch first saw it own land, and every branch's witness that replaced an earlier
/// one in that order counts towards the unnamed. `incomplete` adds what the pages lack: gaps,
/// and hours a branch lost or refused; `tags` is why the content names countries by their tags.
pub(crate) fn catalogue(
    blocks: impl IntoIterator<Item = (u64, i32, i32)>,
    pages: &[(u64, Page)],
    incomplete: bool,
    tags: Option<String>,
) -> LandOwners {
    let mut spans: BTreeMap<u64, Vec<(i32, i32, usize)>> = BTreeMap::new();
    for (at, (branch, first, last)) in blocks.into_iter().enumerate() {
        spans.entry(branch).or_default().push((first, last, at));
    }
    for held in spans.values_mut() {
        held.sort_unstable();
    }
    // The block a branch's hour was read in; none past every block, read last.
    let scanned = |branch: u64, hour: i32| {
        spans
            .get(&branch)
            .and_then(|held| {
                let after = held.partition_point(|&(first, _, _)| first <= hour);
                held.get(after.checked_sub(1)?)
            })
            .filter(|&&(_, last, _)| hour <= last)
            .map_or(usize::MAX, |&(_, _, at)| at)
    };
    let mut seen: Vec<(usize, i32, u64, &Witness)> = pages
        .iter()
        .flat_map(|(branch, page)| {
            page.witnesses
                .iter()
                .map(move |witness| (scanned(*branch, witness.hour), witness.hour, *branch, witness))
        })
        .collect();
    seen.sort_by(|a, b| {
        (a.0, a.1, a.2, a.3.slot, &a.3.tag).cmp(&(b.0, b.1, b.2, b.3.slot, &b.3.tag))
    });
    // Each witness kept, with its slot's ordinal of the tag: tags shortened alike keep their
    // slot's first-seen order, a lower branch's replacement taking the ordinal it replaces.
    let mut kept: HashMap<(&[u8], u32), (u64, usize, &Witness)> = HashMap::new();
    let mut ordinals: HashMap<u32, usize> = HashMap::new();
    let mut unnamed = 0_usize;
    for (_, _, branch, witness) in seen {
        let key = (witness.tag.as_slice(), witness.slot);
        let ordinal = match kept.get(&key) {
            Some(&(held, _, _)) if held <= branch => continue,
            Some(&(_, ordinal, _)) => ordinal,
            None => {
                let next = ordinals.entry(witness.slot).or_default();
                *next += 1;
                *next - 1
            }
        };
        unnamed += usize::from(witness.unresolved);
        kept.insert(key, (branch, ordinal, witness));
    }
    let mut countries: Vec<(String, u64, u32, usize, &Witness)> = kept
        .into_iter()
        .map(|((tag, slot), (branch, ordinal, witness))| {
            let shown = short_text(&String::from_utf8_lossy(tag), 512);
            (shown, branch, slot, ordinal, witness)
        })
        .collect();
    countries.sort_unstable_by(|a, b| (&a.0, a.1, a.2, a.3).cmp(&(&b.0, b.1, b.2, b.3)));
    let mut warnings = Vec::new();
    if incomplete || pages.iter().any(|(_, page)| page.incomplete) {
        warnings.push(OWNERSHIP_INCOMPLETE.to_owned());
    }
    if unnamed > 0 {
        warnings.push(format!("{unnamed} {NAMES_UNRESOLVED}"));
    }
    warnings.extend(tags);
    LandOwners {
        countries: countries
            .into_iter()
            .map(|(tag, branch, slot, _, witness)| CountryMetadata {
                id: slot.to_string(),
                tag,
                name: witness.name.clone(),
                observed_branch_id: branch.to_string(),
                observed_hour: witness.hour,
            })
            .collect(),
        warnings,
    }
}
