//! Doctrine mastery observed from its keeper's hours ([`super::kept`]): the tables as the hours
//! kept hold them, in the fields the observer reads, and what each row gave the last hour
//! observed. An hour observes again only the rows it changed, and those whose owner, folder or
//! spellings it changed, in the order a whole observation reads them, so it names, numbers,
//! interns and refuses exactly as one; each name is spelled once and then given by its key.

use super::kept::{
    Country, Folder, FolderKey, Kept, Missing, Part, Shape, Status, Track, TrackKey,
};
use super::observer::{DoctrineMasteryObserver, InternedToken};
use dashboard_keyed::{Delta, Values, delta::Names};
use replay_data::rows::TableId;
use std::{
    collections::{BTreeMap, HashMap, btree_map},
    io,
};

/// The track measures, in the order an observation reads them.
const MEASURES: [&str; 6] = ["mastery", "bank", "daily", "effective", "rewards", "sub"];

/// One name a row gives, beside its owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Name {
    Present,
    Grand(u32),
    Completed(u32, u32),
    Track(u32, u32, u8),
}

fn spell(owner: &str, name: Name) -> String {
    match name {
        Name::Present => format!("{owner}|present"),
        Name::Grand(folder) => format!("{owner}|{folder}|grand"),
        Name::Completed(folder, at) => format!("{owner}|{folder}|completed|{at}"),
        Name::Track(folder, track, measure) => {
            format!(
                "{owner}|{folder}|{track}|{}",
                MEASURES[usize::from(measure)]
            )
        }
    }
}

/// A value a row gave: its name, the name's key where known (zero before), and the value.
#[derive(Clone, Copy, Debug)]
struct Entry {
    name: Name,
    key: u64,
    value: i128,
}

/// What a row gave the last hour observed: its owner's values.
#[derive(Debug)]
struct Out {
    owner: u32,
    entries: Vec<Entry>,
}

/// A status row's country, as its slot's name part reads it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Owner {
    /// Its slot holds no country.
    #[default]
    None,
    /// The status names another country than the slot's.
    Refused(u32),
    Resolved(u32),
}

/// A row as the hours kept hold it, and what it gave.
#[derive(Debug)]
struct Row<F> {
    fields: F,
    owner: Owner,
    out: Option<Out>,
    /// The refusals an observation of it makes, by id, once per call.
    refusals: Vec<u32>,
}

impl<F> Row<F> {
    fn new(fields: F) -> Self {
        Self {
            fields,
            owner: Owner::None,
            out: None,
            refusals: Vec::new(),
        }
    }
}

/// What a part did to a row.
enum Touch<F> {
    Added,
    Changed,
    Removed(Row<F>),
}

/// Applies `part` to `rows`, giving each row it added, changed or removed to `touched`;
/// `missing` holds why the table is not complete, with the refusal's text.
fn apply_part<K: Ord + Copy, F: Clone + PartialEq>(
    rows: &mut BTreeMap<K, Row<F>>,
    missing: &mut Option<(Missing, String)>,
    table: TableId,
    part: &Part<K, F>,
    mut touched: impl FnMut(K, Touch<F>),
) {
    fn upsert<K: Ord + Copy, F: Clone + PartialEq>(
        rows: &mut BTreeMap<K, Row<F>>,
        key: K,
        fields: &F,
        touched: &mut impl FnMut(K, Touch<F>),
    ) {
        match rows.entry(key) {
            btree_map::Entry::Occupied(mut held) => {
                if held.get().fields != *fields {
                    held.get_mut().fields = fields.clone();
                    touched(key, Touch::Changed);
                }
            }
            btree_map::Entry::Vacant(slot) => {
                slot.insert(Row::new(fields.clone()));
                touched(key, Touch::Added);
            }
        }
    }
    match part {
        Part::Same => {}
        Part::Changed(changed) => {
            for (key, fields) in changed {
                match fields {
                    Some(fields) => upsert(rows, *key, fields, &mut touched),
                    None => {
                        if let Some(row) = rows.remove(key) {
                            touched(*key, Touch::Removed(row));
                        }
                    }
                }
            }
        }
        Part::Whole(whole) => {
            *missing = None;
            let gone: Vec<K> = rows
                .keys()
                .filter(|&&key| whole.binary_search_by(|(held, _)| held.cmp(&key)).is_err())
                .copied()
                .collect();
            for key in gone {
                if let Some(row) = rows.remove(&key) {
                    touched(key, Touch::Removed(row));
                }
            }
            for (key, fields) in whole {
                upsert(rows, *key, fields, &mut touched);
            }
        }
        Part::Missing(view) => {
            let text = format!("{table:?}: {}", view.why);
            *missing = Some(((**view).clone(), text));
            for (key, row) in std::mem::take(rows) {
                touched(key, Touch::Removed(row));
            }
        }
    }
}

/// Whether `part` restates `rows` as they are held.
fn restated<K: PartialEq, F: PartialEq>(
    part: &Part<K, F>,
    rows: &BTreeMap<K, Row<F>>,
    missing: &Option<(Missing, String)>,
) -> bool {
    match (part, missing) {
        (Part::Whole(whole), None) => {
            whole.len() == rows.len()
                && whole
                    .iter()
                    .zip(rows)
                    .all(|((key, fields), (held, row))| key == held && *fields == row.fields)
        }
        (Part::Missing(view), Some((held, _))) => view.same(held),
        _ => false,
    }
}

/// Each refusal a row makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Why {
    Identity(u32),
    Unspelled(u32),
    Unwitnessed { track: u32, folder: u32 },
}

#[derive(Default)]
struct Refusals {
    /// Each refusal's owner and text, by id.
    texts: Vec<(String, String)>,
    ids: HashMap<(u32, Why), u32>,
    /// How often an observation now makes each, where it makes any.
    rate: HashMap<u32, u64>,
}

impl Refusals {
    fn id(&mut self, owner: u32, why: Why, owners: &[String]) -> u32 {
        *self.ids.entry((owner, why)).or_insert_with(|| {
            let text = match why {
                Why::Identity(slot) => format!("doctrine slot {slot} country identity differs"),
                Why::Unspelled(token) => format!("token {token} spelling is not recorded"),
                Why::Unwitnessed { track, folder } => {
                    format!("track {track} has no witnessed folder {folder}")
                }
            };
            self.texts.push((owners[owner as usize].clone(), text));
            (self.texts.len() - 1) as u32
        })
    }

    /// A row made `now` in place of `before`.
    fn rerate(&mut self, before: &[u32], now: &[u32]) {
        if before == now {
            return;
        }
        for id in before {
            if let Some(count) = self.rate.get_mut(id) {
                *count -= 1;
                if *count == 0 {
                    self.rate.remove(id);
                }
            }
        }
        for &id in now {
            *self.rate.entry(id).or_default() += 1;
        }
    }
}

/// Each country name part, interned by its tag and slot.
#[derive(Default)]
struct Owners {
    names: Vec<String>,
    by_slot: HashMap<u32, Vec<(Box<[u8]>, u32)>>,
}

impl Owners {
    fn intern(&mut self, tag: &[u8], slot: u32) -> u32 {
        let held = self.by_slot.entry(slot).or_default();
        if let Some(&(_, id)) = held.iter().find(|(known, _)| **known == *tag) {
            return id;
        }
        let id = self.names.len() as u32;
        self.names.push(dashboard_keyed::countries::part(
            &String::from_utf8_lossy(tag),
            slot,
        ));
        held.push((tag.into(), id));
        id
    }
}

/// The rows an observation reads again.
#[derive(Default)]
struct Dirty {
    status: Vec<u32>,
    folders: Vec<FolderKey>,
    tracks: Vec<TrackKey>,
    /// Every folder and track: the dictionary became complete or stopped being.
    tokens: bool,
}

/// How an observation gives its values to the scan.
pub(super) enum Publish<'p, 'n> {
    /// Every value, after the rows are read.
    Whole(&'p mut Values),
    /// What changed since the last hour observed.
    Changes {
        names: &'p Names<'n>,
        delta: &'p mut Delta,
    },
}

impl Publish<'_, '_> {
    fn set(&mut self, entry: &mut Entry, owner: &str) {
        let Self::Changes { names, delta } = self else {
            return;
        };
        if entry.key == 0 {
            let name = spell(owner, entry.name);
            match names.key(&name) {
                Some(key) => entry.key = key,
                None => return delta.set(name, entry.value),
            }
        }
        delta.set_key(entry.key, entry.value);
    }

    fn remove(&mut self, entry: &Entry, owner: &str) {
        let Self::Changes { names, delta } = self else {
            return;
        };
        let key = match entry.key {
            0 => names.key(&spell(owner, entry.name)),
            key => Some(key),
        };
        if let Some(key) = key {
            delta.remove_key(key);
        }
    }
}

/// Gives `out` the values `now` of `owner`, publishing what changed.
fn renew(
    out: &mut Option<Out>,
    owner: Option<u32>,
    now: &[(Name, i128)],
    owners: &[String],
    publish: &mut Publish<'_, '_>,
) {
    if let (Some(held), Some(owner)) = (out.as_mut(), owner)
        && held.owner == owner
    {
        let spelled = &owners[owner as usize];
        for (at, &(name, value)) in now.iter().enumerate() {
            match held.entries.get_mut(at) {
                Some(entry) if entry.name == name => {
                    if entry.value != value {
                        entry.value = value;
                        publish.set(entry, spelled);
                    }
                }
                Some(entry) => {
                    publish.remove(entry, spelled);
                    *entry = Entry {
                        name,
                        key: 0,
                        value,
                    };
                    publish.set(entry, spelled);
                }
                None => {
                    let mut entry = Entry {
                        name,
                        key: 0,
                        value,
                    };
                    publish.set(&mut entry, spelled);
                    held.entries.push(entry);
                }
            }
        }
        for entry in held.entries.drain(now.len().min(held.entries.len())..) {
            publish.remove(&entry, spelled);
        }
        return;
    }
    if let Some(held) = out.take() {
        for entry in &held.entries {
            publish.remove(entry, &owners[held.owner as usize]);
        }
    }
    if let Some(owner) = owner {
        let spelled = &owners[owner as usize];
        let mut entries = Vec::with_capacity(now.len());
        for &(name, value) in now {
            let mut entry = Entry {
                name,
                key: 0,
                value,
            };
            publish.set(&mut entry, spelled);
            entries.push(entry);
        }
        *out = Some(Out { owner, entries });
    }
}

/// The observer's tables as its keeper's hours hold them.
#[derive(Default)]
pub(super) struct Mirror {
    /// It holds every table of the last hour kept: from a segment's first hour on.
    primed: bool,
    /// The scan holds what each row gave: from an hour observed whole until one is refused.
    published: bool,
    /// Why each table an observation needs complete is not, by `apply`'s order of them
    /// (status, folders, tracks, links, countries), with the refusal's text.
    missing: [Option<(Missing, String)>; 5],
    dictionary: Option<Shape>,
    status: BTreeMap<u32, Row<Status>>,
    folders: BTreeMap<FolderKey, Row<Folder>>,
    tracks: BTreeMap<TrackKey, Row<Track>>,
    links: BTreeMap<u32, Row<u32>>,
    countries: BTreeMap<u32, Row<Country>>,
    /// Each canonical slot's country: the last Countries holds in it.
    slotted: HashMap<u32, u32>,
    /// The spelling of each token the rows referenced, while the dictionary is complete.
    spellings: HashMap<u32, Option<Box<[u8]>>>,
    /// Each token's interned id under its spelling now.
    ids: HashMap<u32, u64>,
    owners: Owners,
    dirty: Dirty,
    /// What rows gone since the last hour observed gave it, by owner.
    retired: Vec<(u32, Entry)>,
    refusals: Refusals,
    /// An observation's values of one row.
    scratch: Vec<(Name, i128)>,
}

impl Mirror {
    pub fn published(&self) -> bool {
        self.published
    }

    /// Whether `kept` restates `table` as the last hour kept held it.
    pub fn restates(&self, kept: &Kept, table: TableId) -> bool {
        if !self.primed {
            return false;
        }
        match table {
            TableId::DoctrineCountries => restated(&kept.status, &self.status, &self.missing[0]),
            TableId::DoctrineFolders => restated(&kept.folders, &self.folders, &self.missing[1]),
            TableId::DoctrineTracks => restated(&kept.tracks, &self.tracks, &self.missing[2]),
            TableId::CountryLinks => restated(&kept.links, &self.links, &self.missing[3]),
            TableId::Countries => restated(&kept.countries, &self.countries, &self.missing[4]),
            TableId::LexicalTokens => {
                match (&kept.dictionary.shape, &self.dictionary) {
                    // Spellings are compared too, so no print collision loses one.
                    (Some(Shape::Complete(print)), Some(Shape::Complete(held))) => {
                        print == held
                            && kept.dictionary.spellings.iter().all(|(token, spelling)| {
                                self.spellings.get(token) == Some(spelling)
                            })
                    }
                    (Some(Shape::Missing(view)), Some(Shape::Missing(held))) => view.same(held),
                    _ => false,
                }
            }
            _ => false,
        }
    }

    fn complete(&self) -> bool {
        matches!(self.dictionary, Some(Shape::Complete(_)))
    }

    /// The spelling an observation interns `token` with.
    fn spelling(&self, token: u32) -> Option<&[u8]> {
        if !self.complete() {
            return None;
        }
        self.spellings.get(&token)?.as_deref()
    }

    /// The owner a status row's slot resolves to.
    fn owner(&mut self, slot: u32, country: u32) -> Owner {
        let Some(row) = self
            .slotted
            .get(&slot)
            .and_then(|id| self.countries.get_key_value(id))
        else {
            return Owner::None;
        };
        let (&id, row) = row;
        let owner = self.owners.intern(&row.fields.tag, slot);
        // Doctrine-array slots are not country references: the status names the slot's
        // country directly or by a link.
        if id == country || self.links.get(&country).map(|link| link.fields) == Some(slot) {
            Owner::Resolved(owner)
        } else {
            Owner::Refused(owner)
        }
    }

    fn resolved(&self, slot: u32) -> Option<u32> {
        match self.status.get(&slot)?.owner {
            Owner::Resolved(owner) => Some(owner),
            _ => None,
        }
    }

    /// Every folder and track of `slot` reads again.
    fn slot_dirty(&mut self, slot: u32) {
        self.dirty.folders.extend(
            self.folders
                .range((slot, 0)..=(slot, u32::MAX))
                .map(|(&key, _)| key),
        );
        self.dirty.tracks.extend(
            self.tracks
                .range((slot, 0, 0)..=(slot, u32::MAX, u32::MAX))
                .map(|(&key, _)| key),
        );
    }

    /// Resolves `slot`'s status row again, and reads again what that changed.
    fn resolve(&mut self, slot: u32) {
        let Some(country) = self.status.get(&slot).map(|row| row.fields.country) else {
            return;
        };
        let owner = self.owner(slot, country);
        if let Some(row) = self.status.get_mut(&slot)
            && row.owner != owner
        {
            row.owner = owner;
            self.dirty.status.push(slot);
            self.slot_dirty(slot);
        }
    }

    fn retire<F>(&mut self, row: Row<F>) {
        self.refusals.rerate(&row.refusals, &[]);
        if let Some(out) = row.out {
            self.retired
                .extend(out.entries.into_iter().map(|entry| (out.owner, entry)));
        }
    }

    /// Takes in what `kept` holds; an hour's changes need the tables before them.
    fn apply(&mut self, kept: &Kept) -> io::Result<()> {
        if !self.primed && !kept.whole() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "doctrine-mastery: an hour's changed rows follow no whole tables",
            ));
        }
        self.primed = true;
        self.take_dictionary(kept);
        let mut regrouped = false;
        apply_part(
            &mut self.countries,
            &mut self.missing[4],
            TableId::Countries,
            &kept.countries,
            |_, _| regrouped = true,
        );
        if regrouped {
            self.slotted.clear();
            for (&id, row) in &self.countries {
                self.slotted.insert(row.fields.slot, id);
            }
        }
        apply_part(
            &mut self.links,
            &mut self.missing[3],
            TableId::CountryLinks,
            &kept.links,
            |_, _| regrouped = true,
        );
        let mut resolve = Vec::new();
        let mut removed = Vec::new();
        apply_part(
            &mut self.status,
            &mut self.missing[0],
            TableId::DoctrineCountries,
            &kept.status,
            |slot, touch| match touch {
                Touch::Added | Touch::Changed => resolve.push(slot),
                Touch::Removed(row) => removed.push((slot, row)),
            },
        );
        for (slot, row) in removed {
            self.retire(row);
            self.dirty.status.push(slot);
            self.slot_dirty(slot);
        }
        if regrouped {
            let slots: Vec<u32> = self.status.keys().copied().collect();
            for slot in slots {
                self.resolve(slot);
            }
        } else {
            for slot in resolve {
                self.resolve(slot);
            }
        }
        let mut folders = Vec::new();
        let mut removed = Vec::new();
        apply_part(
            &mut self.folders,
            &mut self.missing[1],
            TableId::DoctrineFolders,
            &kept.folders,
            |key, touch| match touch {
                Touch::Added => folders.push((key, true)),
                Touch::Changed => folders.push((key, false)),
                Touch::Removed(row) => removed.push((key, row)),
            },
        );
        for ((slot, folder), row) in removed {
            self.retire(row);
            self.witnessed(slot, folder);
        }
        for ((slot, folder), witnessed) in folders {
            self.dirty.folders.push((slot, folder));
            if witnessed {
                self.witnessed(slot, folder);
            }
        }
        let mut removed = Vec::new();
        let dirty = &mut self.dirty.tracks;
        apply_part(
            &mut self.tracks,
            &mut self.missing[2],
            TableId::DoctrineTracks,
            &kept.tracks,
            |key, touch| match touch {
                Touch::Removed(row) => removed.push(row),
                Touch::Added | Touch::Changed => dirty.push(key),
            },
        );
        for row in removed {
            self.retire(row);
        }
        Ok(())
    }

    /// A folder appeared or ended: its tracks' witness changed.
    fn witnessed(&mut self, slot: u32, folder: u32) {
        self.dirty.tracks.extend(
            self.tracks
                .range((slot, folder, 0)..=(slot, folder, u32::MAX))
                .map(|(&key, _)| key),
        );
    }

    /// The dictionary's shape and spellings `kept` holds, and the rows they spell anew.
    fn take_dictionary(&mut self, kept: &Kept) {
        let dictionary = &kept.dictionary;
        if let Some(shape) = &dictionary.shape {
            let was = self.complete();
            self.dictionary = Some(shape.clone());
            if was != self.complete() {
                self.ids.clear();
                self.dirty.tokens = true;
            }
        }
        if !self.complete() {
            return;
        }
        let mut respelled = Vec::new();
        for (token, spelling) in &dictionary.spellings {
            match self.spellings.get_mut(token) {
                Some(held) if held == spelling => {}
                Some(held) => {
                    *held = spelling.clone();
                    self.ids.remove(token);
                    respelled.push(*token);
                }
                // No row read it before: the rows that do now are read anyway.
                None => {
                    self.spellings.insert(*token, spelling.clone());
                    self.ids.remove(token);
                }
            }
        }
        if respelled.is_empty() || self.dirty.tokens {
            return;
        }
        respelled.sort_unstable();
        let spelled = |token: &u32| respelled.binary_search(token).is_ok();
        self.dirty.folders.extend(
            self.folders
                .iter()
                .filter(|(_, row)| {
                    let folder = &row.fields;
                    spelled(&folder.folder)
                        || folder.grand.as_ref().is_some_and(spelled)
                        || folder.completed.iter().any(spelled)
                })
                .map(|(&key, _)| key),
        );
        self.dirty.tracks.extend(
            self.tracks
                .iter()
                .filter(|(_, row)| row.fields.sub.as_ref().is_some_and(spelled))
                .map(|(&key, _)| key),
        );
    }
}

impl DoctrineMasteryObserver {
    /// Observes the hour `kept` holds: Ok(false) where a table it needs is not complete.
    pub(super) fn observe_kept(
        &mut self,
        kept: &Kept,
        mut publish: Publish<'_, '_>,
    ) -> io::Result<bool> {
        self.mirror.apply(kept)?;
        if self.mirror.missing.iter().any(Option::is_some) {
            for (_, text) in self.mirror.missing.iter().flatten() {
                count(&mut self.refusals, "", text, 1);
            }
            self.mirror.published = false;
            return Ok(false);
        }
        let mut dirty = std::mem::take(&mut self.mirror.dirty);
        let mut scratch = std::mem::take(&mut self.mirror.scratch);
        dirty.status.sort_unstable();
        dirty.status.dedup();
        for &slot in &dirty.status {
            self.renew_status(slot, &mut scratch, &mut publish);
        }
        if dirty.tokens {
            dirty.folders = self.mirror.folders.keys().copied().collect();
            dirty.tracks = self.mirror.tracks.keys().copied().collect();
        } else {
            dirty.folders.sort_unstable();
            dirty.folders.dedup();
            dirty.tracks.sort_unstable();
            dirty.tracks.dedup();
        }
        for &key in &dirty.folders {
            self.renew_folder(key, &mut scratch, &mut publish);
        }
        for &key in &dirty.tracks {
            self.renew_track(key, &mut publish);
        }
        dirty.status.clear();
        dirty.folders.clear();
        dirty.tracks.clear();
        dirty.tokens = false;
        self.mirror.dirty = dirty;
        self.mirror.scratch = scratch;
        let mirror = &mut self.mirror;
        for (owner, entry) in mirror.retired.drain(..) {
            publish.remove(&entry, &mirror.owners.names[owner as usize]);
        }
        if let Publish::Whole(values) = &mut publish {
            let owners = &mirror.owners.names;
            let outs = (mirror.status.values().map(|row| &row.out))
                .chain(mirror.folders.values().map(|row| &row.out))
                .chain(mirror.tracks.values().map(|row| &row.out));
            for out in outs.flatten() {
                for entry in &out.entries {
                    match entry.key {
                        0 => {
                            values.push(spell(&owners[out.owner as usize], entry.name), entry.value)
                        }
                        key => values.push_key(key, entry.value),
                    }
                }
            }
        }
        for (&id, &times) in &mirror.refusals.rate {
            let (owner, why) = &mirror.refusals.texts[id as usize];
            count(&mut self.refusals, owner, why, times);
        }
        mirror.published = true;
        Ok(true)
    }

    fn renew_status(
        &mut self,
        slot: u32,
        scratch: &mut Vec<(Name, i128)>,
        publish: &mut Publish<'_, '_>,
    ) {
        let mirror = &mut self.mirror;
        let Some(row) = mirror.status.get_mut(&slot) else {
            return;
        };
        let (owner, refusals) = match row.owner {
            Owner::Resolved(owner) => (Some(owner), Vec::new()),
            Owner::Refused(owner) => (
                None,
                vec![
                    mirror
                        .refusals
                        .id(owner, Why::Identity(slot), &mirror.owners.names),
                ],
            ),
            Owner::None => (None, Vec::new()),
        };
        mirror.refusals.rerate(&row.refusals, &refusals);
        row.refusals = refusals;
        scratch.clear();
        if owner.is_some() {
            // The presence witness tells an empty inventory from an unassigned slot.
            scratch.push((Name::Present, 1));
        }
        renew(
            &mut row.out,
            owner,
            scratch.as_slice(),
            &mirror.owners.names,
            publish,
        );
    }

    /// The interned id of `token` as it is spelled now; a refusal of `owner`'s where it is not.
    fn token(&mut self, token: u32, owner: u32, refusals: &mut Vec<u32>) -> u64 {
        let mirror = &mut self.mirror;
        if mirror.spelling(token).is_none() {
            let why = Why::Unspelled(token);
            refusals.push(mirror.refusals.id(owner, why, &mirror.owners.names));
        }
        if let Some(&id) = mirror.ids.get(&token) {
            return id;
        }
        let literal = mirror.spelling(token).map(<[u8]>::to_vec);
        if self.index.len() != self.tokens.len() {
            self.index = self
                .tokens
                .iter()
                .enumerate()
                .map(|(at, token)| ((token.raw, token.literal.clone()), at as u64 + 1))
                .collect();
        }
        let key = (token, literal);
        let id = match self.index.get(&key) {
            Some(&id) => id,
            None => {
                let id = self.tokens.len() as u64 + 1;
                self.tokens.push(InternedToken {
                    raw: token,
                    literal: key.1.clone(),
                });
                self.index.insert(key, id);
                id
            }
        };
        self.mirror.ids.insert(token, id);
        id
    }

    fn renew_folder(
        &mut self,
        key: FolderKey,
        scratch: &mut Vec<(Name, i128)>,
        publish: &mut Publish<'_, '_>,
    ) {
        let (slot, ordinal) = key;
        let Some(row) = self.mirror.folders.get_mut(&key) else {
            return;
        };
        let completed = std::mem::take(&mut row.fields.completed);
        let (folder, grand) = (row.fields.folder, row.fields.grand);
        let owner = self.mirror.resolved(slot);
        let mut refusals = Vec::new();
        scratch.clear();
        if let Some(owner) = owner {
            let token = self.token(folder, owner, &mut refusals);
            let grand = grand
                .map(|raw| self.token(raw, owner, &mut refusals))
                .unwrap_or(0);
            let inventory = self.folders.entry(ordinal).or_default();
            inventory.tokens.insert(token);
            if grand != 0 {
                inventory.grands.insert(grand);
            }
            scratch.push((Name::Grand(ordinal), i128::from(grand)));
            for (at, &raw) in completed.iter().enumerate() {
                let token = self.token(raw, owner, &mut refusals);
                scratch.push((Name::Completed(ordinal, at as u32), i128::from(token)));
            }
        }
        let mirror = &mut self.mirror;
        let Some(row) = mirror.folders.get_mut(&key) else {
            return;
        };
        row.fields.completed = completed;
        mirror.refusals.rerate(&row.refusals, &refusals);
        row.refusals = refusals;
        renew(
            &mut row.out,
            owner,
            scratch.as_slice(),
            &mirror.owners.names,
            publish,
        );
    }

    fn renew_track(&mut self, key: TrackKey, publish: &mut Publish<'_, '_>) {
        let (slot, folder, ordinal) = key;
        let Some(row) = self.mirror.tracks.get(&key) else {
            return;
        };
        let track = row.fields;
        let owner = self.mirror.resolved(slot);
        let mut refusals = Vec::new();
        let mut now = [(Name::Present, 0); 6];
        if let Some(owner) = owner {
            let held_sub = row
                .out
                .as_ref()
                .filter(|out| out.owner == owner)
                .and_then(|out| out.entries.get(5))
                .map(|entry| entry.value);
            if !self.mirror.folders.contains_key(&(slot, folder)) {
                let why = Why::Unwitnessed {
                    track: ordinal,
                    folder,
                };
                let mirror = &mut self.mirror;
                refusals.push(mirror.refusals.id(owner, why, &mirror.owners.names));
            }
            let sub = track
                .sub
                .map(|raw| self.token(raw, owner, &mut refusals))
                .unwrap_or(0);
            // The inventory already holds what an unchanged selection would add.
            if held_sub != Some(i128::from(sub)) {
                let inventory = self.folders.entry(folder).or_default();
                inventory.tracks.insert(ordinal);
                if sub != 0 {
                    inventory.subs.entry(ordinal).or_default().insert(sub);
                }
            }
            for (at, value) in [
                i128::from(track.mastery),
                i128::from(track.bank),
                i128::from(track.daily),
                i128::from(track.effective),
                i128::from(track.rewards),
                i128::from(sub),
            ]
            .into_iter()
            .enumerate()
            {
                now[at] = (Name::Track(folder, ordinal, at as u8), value);
            }
        }
        let mirror = &mut self.mirror;
        let Some(row) = mirror.tracks.get_mut(&key) else {
            return;
        };
        mirror.refusals.rerate(&row.refusals, &refusals);
        row.refusals = refusals;
        let now = if owner.is_some() { &now[..] } else { &[] };
        renew(&mut row.out, owner, now, &mirror.owners.names, publish);
    }
}

/// Counts `times` refusals of `owner`'s, `why`.
fn count(
    refusals: &mut BTreeMap<String, BTreeMap<String, u64>>,
    owner: &str,
    why: &str,
    times: u64,
) {
    let whys = match refusals.get_mut(owner) {
        Some(whys) => whys,
        None => refusals.entry(owner.to_owned()).or_default(),
    };
    match whys.get_mut(why) {
        Some(count) => *count += times,
        None => {
            whys.insert(why.to_owned(), times);
        }
    }
}
