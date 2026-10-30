//! The countries by their Countries slot, their links and tags, kept from the hour's changed
//! rows: a country is read again only when its tag or slot was written, and a link only when
//! its slot was.

use super::{Facts, Flags, invalid};
use replay::{columns, segments::State};
use replay_data::{
    format::FormatVersion,
    rows::{CountryLinkRow, CountryRow, TableId},
};
use std::{collections::HashMap, io};

/// A tag, numbered as the walk first read its bytes: equal tags are equal bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Tag(u32);

/// A Countries row's identity: its raw link id, canonical slot and tag, and whether the links
/// resolve the id to that slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Country {
    pub id: u32,
    pub slot: u32,
    pub tag: Tag,
    pub resolved: bool,
}

impl Country {
    /// (canonical slot, id), when the links resolve it.
    pub fn identity(&self) -> Option<(u32, u32)> {
        self.resolved.then_some((self.slot, self.id))
    }
}

/// What the countries say of a tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tagged<'r> {
    /// Its first holder by id whose identity the links resolve.
    Found(&'r Country),
    /// No country holds it.
    Absent,
    /// Countries hold it, but the links resolve none of them: who holds it is unknown.
    Unresolved,
}

/// The columns identity reads.
struct Fields {
    tag: usize,
    slot: usize,
    link: usize,
}

/// The countries of the hour. Countries and CountryLinks are held only while complete.
pub struct Roster {
    fields: Fields,
    countries_complete: bool,
    links_complete: bool,
    /// Each Countries slot's country.
    rows: Vec<Option<Country>>,
    /// Each country's id and Countries slot, by id.
    order: Vec<(u32, u32)>,
    /// By raw id.
    links: Vec<CountryLinkRow>,
    /// Each canonical slot's raw link ids, ascending.
    aliases: HashMap<u32, Vec<u32>>,
    /// Each canonical slot's countries' ids, ascending.
    slots: HashMap<u32, Vec<u32>>,
    tags: Vec<Box<[u8]>>,
    numbered: HashMap<Box<[u8]>, Tag>,
    /// Each tag's holders' ids, ascending.
    holders: Vec<Vec<u32>>,
    rebuilt: bool,
    changed: Vec<u32>,
    changed_tags: Vec<Tag>,
    changed_links: Vec<u32>,
    epoch: u64,
    /// Each country's flag inputs, when a lane reads them.
    flags: Option<Flags>,
    /// Rows read from the walker's columns.
    #[cfg(test)]
    pub(crate) reads: usize,
}

/// Inserts `value` into ascending `list`, once.
pub(crate) fn insert_sorted(list: &mut Vec<u32>, value: u32) -> io::Result<()> {
    if let Err(at) = list.binary_search(&value) {
        engine_allocation::reserve(list, 1)?;
        list.insert(at, value);
    }
    Ok(())
}

/// Removes `value` from ascending `list`; false when the list is then empty.
pub(crate) fn remove_sorted(list: &mut Vec<u32>, value: u32) -> bool {
    if let Ok(at) = list.binary_search(&value) {
        list.remove(at);
    }
    !list.is_empty()
}

/// The slots `changes` wrote any of `mask` at, or inserted, ascending.
pub(crate) fn edited(state: &State, table: TableId, mask: u64) -> io::Result<Vec<u32>> {
    let changes = columns::changes(state, table)?;
    let mut slots = engine_allocation::vec(
        changes.changed.len() + changes.fill_slots.len() + changes.inserted.len(),
    )?;
    slots.extend(
        changes
            .changed
            .iter()
            .zip(&changes.masks)
            .chain(changes.fill_slots.iter().zip(&changes.fill_masks))
            .filter(|&(_, &written)| written & mask != 0)
            .map(|(&slot, _)| slot)
            .chain(changes.inserted.iter().copied()),
    );
    slots.sort_unstable();
    slots.dedup();
    Ok(slots)
}

impl Roster {
    /// A roster keeping its countries' flag inputs too when `flags`.
    pub(crate) fn new(flags: bool) -> io::Result<Self> {
        Ok(Self {
            fields: Fields {
                tag: columns::field(TableId::Countries, "tag")?,
                slot: columns::field(TableId::Countries, "canonical_slot")?,
                link: columns::field(TableId::CountryLinks, "canonical_slot")?,
            },
            countries_complete: false,
            links_complete: false,
            rows: Vec::new(),
            order: Vec::new(),
            links: Vec::new(),
            aliases: HashMap::new(),
            slots: HashMap::new(),
            tags: Vec::new(),
            numbered: HashMap::new(),
            holders: Vec::new(),
            rebuilt: false,
            changed: Vec::new(),
            changed_tags: Vec::new(),
            changed_links: Vec::new(),
            epoch: 0,
            flags: flags.then(Flags::new).transpose()?,
            #[cfg(test)]
            reads: 0,
        })
    }

    /// The countries' flag inputs; None unless a lane of the walk reads them.
    pub fn flags(&self) -> Option<&Flags> {
        self.flags.as_ref()
    }

    /// Whether Countries and CountryLinks both hold every row this hour.
    pub fn complete(&self) -> bool {
        self.countries_complete && self.links_complete
    }

    pub fn countries_complete(&self) -> bool {
        self.countries_complete
    }

    pub fn links_complete(&self) -> bool {
        self.links_complete
    }

    /// Whether the hour changed a country, a link or a tag's holders, or read them anew.
    pub fn changed(&self) -> bool {
        self.rebuilt || !self.changed.is_empty() || !self.changed_links.is_empty()
    }

    /// Whether the hour read every country and link anew: the walk's first hour, or one
    /// either table became complete or stopped being.
    pub fn rebuilt(&self) -> bool {
        self.rebuilt
    }

    /// Counts the hours that changed the countries: equal epochs, equal countries.
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// The ids whose country the hour added, removed or changed, ascending; every id held
    /// before or after when [`Self::rebuilt`].
    pub fn changed_ids(&self) -> &[u32] {
        &self.changed
    }

    /// The tags whose holders, or a holder's resolution, the hour changed, ascending.
    pub fn changed_tags(&self) -> &[Tag] {
        &self.changed_tags
    }

    /// The raw link ids whose link the hour added, removed or moved, ascending.
    pub fn changed_links(&self) -> &[u32] {
        &self.changed_links
    }

    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// Every country, by id.
    pub fn iter(&self) -> impl Iterator<Item = &Country> + '_ {
        self.order
            .iter()
            .filter_map(|&(_, slot)| self.rows[slot as usize].as_ref())
    }

    /// The country of raw id `id`.
    pub fn get(&self, id: u32) -> Option<&Country> {
        let at = self.order.binary_search_by_key(&id, |&(id, _)| id).ok()?;
        self.rows[self.order[at].1 as usize].as_ref()
    }

    /// The canonical slot raw link id `raw` names, when a link holds it.
    pub fn slot_of(&self, raw: u32) -> Option<u32> {
        self.links
            .binary_search_by_key(&raw, |link| link.id)
            .ok()
            .map(|at| self.links[at].canonical_slot)
    }

    /// Every link, by raw id.
    pub fn links(&self) -> &[CountryLinkRow] {
        &self.links
    }

    /// The raw link ids naming canonical slot `slot`, ascending.
    pub fn aliases(&self, slot: u32) -> &[u32] {
        self.aliases.get(&slot).map_or(&[][..], Vec::as_slice)
    }

    /// The countries whose canonical slot is `slot`, by id.
    pub fn at_slot(&self, slot: u32) -> impl Iterator<Item = &Country> + '_ {
        self.slots
            .get(&slot)
            .into_iter()
            .flatten()
            .filter_map(|&id| self.get(id))
    }

    pub fn tag(&self, tag: Tag) -> &[u8] {
        &self.tags[tag.0 as usize]
    }

    /// The tag of `bytes`, once the walk read it.
    pub fn tag_of(&self, bytes: &[u8]) -> Option<Tag> {
        self.numbered.get(bytes).copied()
    }

    /// The countries holding `tag`, by id.
    pub fn holders(&self, tag: Tag) -> impl Iterator<Item = &Country> + '_ {
        self.holders
            .get(tag.0 as usize)
            .into_iter()
            .flatten()
            .filter_map(|&id| self.get(id))
    }

    /// The tags some country holds.
    pub fn held(&self) -> impl Iterator<Item = Tag> + '_ {
        (0..self.holders.len() as u32)
            .map(Tag)
            .filter(|tag| !self.holders[tag.0 as usize].is_empty())
    }

    /// What the countries say of `tag`.
    pub fn tagged(&self, tag: Tag) -> Tagged<'_> {
        let mut holders = self.holders(tag).peekable();
        if holders.peek().is_none() {
            return Tagged::Absent;
        }
        holders
            .find(|country| country.resolved)
            .map_or(Tagged::Unresolved, Tagged::Found)
    }

    /// Every country as a Countries row of its id, slot and tag alone, by id.
    pub fn country_rows(&self) -> io::Result<Vec<CountryRow>> {
        let mut rows = engine_allocation::vec(self.order.len())?;
        for country in self.iter() {
            rows.push(CountryRow {
                id: country.id,
                canonical_slot: country.slot,
                tag: engine_allocation::copy_vec(self.tag(country.tag))?,
                ..Default::default()
            });
        }
        Ok(rows)
    }

    fn number(&mut self, bytes: &[u8]) -> io::Result<Tag> {
        if let Some(&tag) = self.numbered.get(bytes) {
            return Ok(tag);
        }
        let tag = Tag(u32::try_from(self.tags.len()).map_err(|_| invalid("Tags exceed 32 bits"))?);
        let owned: Box<[u8]> = engine_allocation::copy_vec(bytes)?.into_boxed_slice();
        engine_allocation::reserve(&mut self.tags, 1)?;
        engine_allocation::reserve(&mut self.holders, 1)?;
        self.numbered
            .try_reserve(1)
            .map_err(|_| engine_allocation::oom())?;
        self.tags.push(owned.clone());
        self.holders.push(Vec::new());
        self.numbered.insert(owned, tag);
        Ok(tag)
    }

    /// The country at Countries `slot`: its id, canonical slot and tag.
    fn read(
        &mut self,
        state: &State,
        slot: u32,
        format: FormatVersion,
    ) -> io::Result<(u32, u32, Tag)> {
        #[cfg(test)]
        {
            self.reads += 1;
        }
        let held = columns::table(state, TableId::Countries)?;
        let id = held
            .key(slot)
            .ok_or_else(|| invalid(format!("Countries slot {slot} holds no row")))?
            as u32;
        let Fields {
            tag, slot: field, ..
        } = self.fields;
        if !held.recorded(field, slot) || !held.recorded(tag, slot) {
            // An unrecorded cell reads as the row decoder reads it.
            let row: CountryRow = columns::row(state, TableId::Countries, slot, format.version())?;
            return Ok((id, row.canonical_slot, self.number(&row.tag)?));
        }
        let missing = || invalid(format!("Countries slot {slot} holds no cell"));
        let canonical = *columns::plain::<u32>(held, field)?
            .get(slot as usize)
            .ok_or_else(missing)?;
        let text = *columns::text(held, tag)?
            .get(slot as usize)
            .ok_or_else(missing)?;
        let bytes = state
            .dictionary()
            .get(text)
            .ok_or_else(|| invalid("A country's tag names no dictionary entry"))?;
        Ok((id, canonical, self.number(bytes)?))
    }

    /// The link at CountryLinks `slot`: its raw id and canonical slot.
    fn read_link(
        &mut self,
        state: &State,
        slot: u32,
        format: FormatVersion,
    ) -> io::Result<(u32, u32)> {
        #[cfg(test)]
        {
            self.reads += 1;
        }
        let held = columns::table(state, TableId::CountryLinks)?;
        let raw = held
            .key(slot)
            .ok_or_else(|| invalid(format!("CountryLinks slot {slot} holds no row")))?
            as u32;
        if !held.recorded(self.fields.link, slot) {
            let row: CountryLinkRow =
                columns::row(state, TableId::CountryLinks, slot, format.version())?;
            return Ok((raw, row.canonical_slot));
        }
        let canonical = *columns::plain::<u32>(held, self.fields.link)?
            .get(slot as usize)
            .ok_or_else(|| invalid(format!("CountryLinks slot {slot} holds no cell")))?;
        Ok((raw, canonical))
    }

    /// Links `raw` to `canonical`: whether that moved it.
    fn link(&mut self, raw: u32, canonical: u32) -> io::Result<bool> {
        match self.links.binary_search_by_key(&raw, |link| link.id) {
            Ok(at) if self.links[at].canonical_slot == canonical => return Ok(false),
            Ok(at) => {
                let old = std::mem::replace(&mut self.links[at].canonical_slot, canonical);
                self.unalias(old, raw);
            }
            Err(at) => {
                engine_allocation::reserve(&mut self.links, 1)?;
                self.links.insert(
                    at,
                    CountryLinkRow {
                        id: raw,
                        canonical_slot: canonical,
                    },
                );
            }
        }
        insert_sorted(self.aliases.entry(canonical).or_default(), raw)?;
        Ok(true)
    }

    /// Removes `raw`'s link: whether it had one.
    fn unlink(&mut self, raw: u32) -> bool {
        let Ok(at) = self.links.binary_search_by_key(&raw, |link| link.id) else {
            return false;
        };
        let link = self.links.remove(at);
        self.unalias(link.canonical_slot, raw);
        true
    }

    fn unalias(&mut self, canonical: u32, raw: u32) {
        if let Some(raws) = self.aliases.get_mut(&canonical)
            && !remove_sorted(raws, raw)
        {
            self.aliases.remove(&canonical);
        }
    }

    fn note(&mut self, country: Country) -> io::Result<()> {
        engine_allocation::reserve(&mut self.changed, 1)?;
        engine_allocation::reserve(&mut self.changed_tags, 1)?;
        self.changed.push(country.id);
        self.changed_tags.push(country.tag);
        Ok(())
    }

    fn place(&mut self, country: Country, at: u32) -> io::Result<()> {
        let Err(index) = self.order.binary_search_by_key(&country.id, |&(id, _)| id) else {
            return Err(invalid(format!(
                "Countries holds raw id {} at two slots",
                country.id
            )));
        };
        engine_allocation::reserve(&mut self.order, 1)?;
        self.order.insert(index, (country.id, at));
        insert_sorted(self.slots.entry(country.slot).or_default(), country.id)?;
        insert_sorted(&mut self.holders[country.tag.0 as usize], country.id)?;
        self.note(country)
    }

    fn unplace(&mut self, country: Country) -> io::Result<()> {
        if let Ok(index) = self.order.binary_search_by_key(&country.id, |&(id, _)| id) {
            self.order.remove(index);
        }
        if let Some(ids) = self.slots.get_mut(&country.slot)
            && !remove_sorted(ids, country.id)
        {
            self.slots.remove(&country.slot);
        }
        remove_sorted(&mut self.holders[country.tag.0 as usize], country.id);
        self.note(country)
    }

    /// Sets Countries `at` to the country of `id`, slot `canonical` and `tag`.
    fn set(&mut self, at: u32, id: u32, canonical: u32, tag: Tag) -> io::Result<()> {
        let country = Country {
            id,
            slot: canonical,
            tag,
            resolved: self.slot_of(id) == Some(canonical),
        };
        let index = at as usize;
        if index >= self.rows.len() {
            let more = index + 1 - self.rows.len();
            engine_allocation::reserve(&mut self.rows, more)?;
            self.rows.resize(index + 1, None);
        }
        let old = self.rows[index];
        if old == Some(country) {
            return Ok(());
        }
        if let Some(old) = old {
            self.unplace(old)?;
        }
        self.rows[index] = Some(country);
        self.place(country, at)
    }

    /// Reads every country and link anew; all of them before and after count as changed.
    fn rebuild(
        &mut self,
        state: &State,
        (countries_complete, links_complete): (bool, bool),
        format: FormatVersion,
    ) -> io::Result<()> {
        engine_allocation::reserve(&mut self.changed, self.order.len())?;
        self.changed.extend(self.order.iter().map(|&(id, _)| id));
        let held: Vec<Tag> = self.held().collect();
        engine_allocation::reserve(&mut self.changed_tags, held.len())?;
        self.changed_tags.extend(held);
        engine_allocation::reserve(&mut self.changed_links, self.links.len())?;
        self.changed_links
            .extend(self.links.iter().map(|link| link.id));
        self.rows.clear();
        self.order.clear();
        self.links.clear();
        self.aliases.clear();
        self.slots.clear();
        for holders in &mut self.holders {
            holders.clear();
        }
        self.countries_complete = countries_complete;
        self.links_complete = links_complete;
        self.rebuilt = true;
        if links_complete {
            let live: Vec<u32> = columns::table(state, TableId::CountryLinks)?
                .live()
                .ones()
                .collect();
            for slot in live {
                let (raw, canonical) = self.read_link(state, slot, format)?;
                self.link(raw, canonical)?;
                engine_allocation::reserve(&mut self.changed_links, 1)?;
                self.changed_links.push(raw);
            }
        }
        if countries_complete {
            let live: Vec<u32> = columns::table(state, TableId::Countries)?
                .live()
                .ones()
                .collect();
            for slot in live {
                let (id, canonical, tag) = self.read(state, slot, format)?;
                self.set(slot, id, canonical, tag)?;
            }
        }
        Ok(())
    }

    pub(crate) fn hour(
        &mut self,
        state: &State,
        facts: &Facts<'_>,
        format: FormatVersion,
    ) -> io::Result<()> {
        self.rebuilt = false;
        self.changed.clear();
        self.changed_tags.clear();
        self.changed_links.clear();
        let complete = (
            columns::complete(state, TableId::Countries),
            columns::complete(state, TableId::CountryLinks),
        );
        if facts.first() || complete != (self.countries_complete, self.links_complete) {
            self.rebuild(state, complete, format)?;
        } else {
            self.follow(state, facts, format)?;
        }
        if let Some(mut flags) = self.flags.take() {
            let followed = flags.hour(state, facts, format, &mut |bytes| self.number(bytes));
            flags.keep(|id| self.get(id).is_some());
            self.flags = Some(flags);
            followed?;
        }
        self.changed.sort_unstable();
        self.changed.dedup();
        self.changed_tags.sort_unstable();
        self.changed_tags.dedup();
        self.changed_links.sort_unstable();
        self.changed_links.dedup();
        if self.changed() {
            self.epoch += 1;
        }
        Ok(())
    }

    /// The hour's changed links and countries, and the countries whose links moved.
    fn follow(
        &mut self,
        state: &State,
        facts: &Facts<'_>,
        format: FormatVersion,
    ) -> io::Result<()> {
        let link_mask = 1_u64 << self.fields.link;
        if self.links_complete && facts.touched(TableId::CountryLinks, link_mask) {
            let changes = columns::changes(state, TableId::CountryLinks)?;
            for &key in &changes.deleted_keys {
                if self.unlink(key as u32) {
                    engine_allocation::reserve(&mut self.changed_links, 1)?;
                    self.changed_links.push(key as u32);
                }
            }
            for slot in edited(state, TableId::CountryLinks, link_mask)? {
                let (raw, canonical) = self.read_link(state, slot, format)?;
                if self.link(raw, canonical)? {
                    engine_allocation::reserve(&mut self.changed_links, 1)?;
                    self.changed_links.push(raw);
                }
            }
        }
        let country_mask = 1_u64 << self.fields.tag | 1_u64 << self.fields.slot;
        if self.countries_complete && facts.touched(TableId::Countries, country_mask) {
            let deleted = columns::changes(state, TableId::Countries)?.deleted.clone();
            for slot in deleted {
                if let Some(country) = self.rows.get_mut(slot as usize).and_then(Option::take) {
                    self.unplace(country)?;
                }
            }
            for slot in edited(state, TableId::Countries, country_mask)? {
                let (id, canonical, tag) = self.read(state, slot, format)?;
                self.set(slot, id, canonical, tag)?;
            }
        }
        // A moved link resolves its country again.
        for at in 0..self.changed_links.len() {
            let raw = self.changed_links[at];
            let Ok(index) = self.order.binary_search_by_key(&raw, |&(id, _)| id) else {
                continue;
            };
            let slot = self.order[index].1 as usize;
            let Some(country) = self.rows[slot] else {
                continue;
            };
            let resolved = self.slot_of(raw) == Some(country.slot);
            if resolved != country.resolved {
                self.rows[slot] = Some(Country {
                    resolved,
                    ..country
                });
                self.note(country)?;
            }
        }
        Ok(())
    }
}
