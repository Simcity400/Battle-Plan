//! Doctrine mastery's keeper on a native recording's segment: at each hour the observer
//! observes, the rows the hour changed of each table it reads, in the fields it reads, or every
//! row where the hour restated the table; of the dictionary, its print and the spellings of the
//! tokens those rows reference. The world's views are never kept: a table that is not complete
//! keeps a copy of its view where the hour lent another.

use super::observer::DoctrineMasteryObserver as Observer;
use dashboard_base::prepared::prepass::HeldBytes;
use dashboard_keyed::{
    Delta, Values,
    block::Operands,
    delta::Names,
    native,
    segment::{Touch, Touched},
};
use replay::columnar_frames::view_heap_bytes;
use replay_data::{
    format::FormatVersion,
    rows::{
        CountryLinkRow, CountryRow, DoctrineCountryRow, DoctrineFolderRow, DoctrineTrackRow,
        RowCodec, TableId, TableSchema, tables,
    },
};
use replay_frame::frame::{DecodedFrame, SharedView, ViewIdentity};
use std::{collections::HashSet, io};

/// A doctrine slot's status: its country, and its observed expiry occurrences.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Status {
    pub country: u32,
    pub expiry: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Folder {
    pub folder: u32,
    pub grand: Option<u32>,
    pub completed: Box<[u32]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Track {
    pub sub: Option<u32>,
    pub rewards: i32,
    pub mastery: i64,
    pub bank: i64,
    pub daily: i64,
    pub effective: i64,
}

/// A country's canonical slot and tag.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Country {
    pub slot: u32,
    pub tag: Box<[u8]>,
}

pub(super) type FolderKey = (u32, u32);
pub(super) type TrackKey = (u32, u32, u32);

/// A table that is not complete: a copy of its view, and why the observer cannot read it.
#[derive(Clone, Debug)]
pub(super) struct Missing {
    pub view: SharedView,
    pub why: String,
}

impl Missing {
    /// Whether two hours lent the same view, as an hour restating a table compares it.
    pub fn same(&self, held: &Self) -> bool {
        !self.view.is_not_selected() && *self.view == *held.view
    }
}

/// What an hour gave of one table, keyed as its rows are.
pub(super) enum Part<K, F> {
    /// Nothing the observer reads changed.
    Same,
    /// The table complete, restated: every row.
    Whole(Vec<(K, F)>),
    /// The table complete, as it was the hour before: each row written, or None where deleted,
    /// ascending.
    Changed(Vec<(K, Option<F>)>),
    /// Not complete: restated, or lent another view.
    Missing(Box<Missing>),
}

impl<K, F> Part<K, F> {
    fn whole(&self) -> bool {
        matches!(self, Self::Whole(_) | Self::Missing(_))
    }

    /// Each row it gives.
    pub fn rows(&self) -> impl Iterator<Item = &F> {
        let (whole, changed) = match self {
            Self::Whole(rows) => (rows.as_slice(), &[][..]),
            Self::Changed(rows) => (&[][..], rows.as_slice()),
            _ => (&[][..], &[][..]),
        };
        whole
            .iter()
            .map(|(_, fields)| fields)
            .chain(changed.iter().filter_map(|(_, fields)| fields.as_ref()))
    }

    fn bytes(&self, heap: impl Fn(&F) -> usize) -> usize {
        match self {
            Self::Same => 0,
            Self::Whole(rows) => {
                rows.capacity() * size_of::<(K, F)>()
                    + rows.iter().map(|(_, fields)| heap(fields)).sum::<usize>()
            }
            Self::Changed(rows) => {
                rows.capacity() * size_of::<(K, Option<F>)>()
                    + rows
                        .iter()
                        .filter_map(|(_, fields)| fields.as_ref())
                        .map(&heap)
                        .sum::<usize>()
            }
            Self::Missing(missing) => missing.bytes(),
        }
    }
}

impl Missing {
    fn bytes(&self) -> usize {
        size_of::<Self>() + self.why.capacity() + view_heap_bytes(&self.view)
    }
}

/// The dictionary's shape: its rows' print where it is complete.
#[derive(Clone, Debug)]
pub(super) enum Shape {
    Complete(u128),
    Missing(Box<Missing>),
}

/// What an hour gave of the dictionary.
pub(super) struct Dictionary {
    /// Its shape where the hour restated or changed it.
    pub shape: Option<Shape>,
    /// Where it is complete: the spelling of each token the segment's rows first referenced at
    /// the hour, each it changed of those referenced before, or of every one referenced where
    /// it restated it; None where it holds none.
    pub spellings: Vec<(u32, Option<Box<[u8]>>)>,
}

/// One hour the observer observes, as the keeper kept it.
pub(super) struct Kept {
    pub status: Part<u32, Status>,
    pub folders: Part<FolderKey, Folder>,
    pub tracks: Part<TrackKey, Track>,
    pub links: Part<u32, u32>,
    pub countries: Part<u32, Country>,
    pub dictionary: Dictionary,
}

impl Kept {
    /// Whether it restates every table: a segment's first hour.
    pub fn whole(&self) -> bool {
        self.status.whole()
            && self.folders.whole()
            && self.tracks.whole()
            && self.links.whole()
            && self.countries.whole()
            && self.dictionary.shape.is_some()
    }

    fn bytes(&self) -> usize {
        size_of::<Self>()
            + self.status.bytes(|_| 0)
            + self
                .folders
                .bytes(|folder| folder.completed.len() * size_of::<u32>())
            + self.tracks.bytes(|_| 0)
            + self.links.bytes(|_| 0)
            + self.countries.bytes(|country| country.tag.len())
            + match &self.dictionary.shape {
                Some(Shape::Missing(missing)) => missing.bytes(),
                _ => 0,
            }
            + self.dictionary.spellings.capacity() * size_of::<(u32, Option<Box<[u8]>>)>()
            + self
                .dictionary
                .spellings
                .iter()
                .filter_map(|(_, spelling)| spelling.as_ref().map(|spelling| spelling.len()))
                .sum::<usize>()
    }
}

/// A table the observer reads, by its rows' keys and the fields it reads of them.
trait Table: TableSchema {
    type Key: Copy;
    type Fields;
    fn key(key: u128) -> Self::Key;
    fn fields(row: &Self::Row) -> Self::Fields;
}

impl Table for tables::DoctrineCountries {
    type Key = u32;
    type Fields = Status;
    fn key(key: u128) -> u32 {
        key as u32
    }
    fn fields(row: &DoctrineCountryRow) -> Status {
        Status {
            country: row.country,
            expiry: row.expiry_events,
        }
    }
}

impl Table for tables::DoctrineFolders {
    type Key = FolderKey;
    type Fields = Folder;
    fn key(key: u128) -> FolderKey {
        ((key >> 32) as u32, key as u32)
    }
    fn fields(row: &DoctrineFolderRow) -> Folder {
        Folder {
            folder: row.folder,
            grand: row.grand_doctrine,
            completed: row.completed_subdoctrines.as_slice().into(),
        }
    }
}

impl Table for tables::DoctrineTracks {
    type Key = TrackKey;
    type Fields = Track;
    fn key(key: u128) -> TrackKey {
        ((key >> 64) as u32, (key >> 32) as u32, key as u32)
    }
    fn fields(row: &DoctrineTrackRow) -> Track {
        Track {
            sub: row.sub_doctrine,
            rewards: row.rewards,
            mastery: row.mastery,
            bank: row.mastery_bank,
            daily: row.daily_mastery,
            effective: row.effective_daily_mastery,
        }
    }
}

impl Table for tables::CountryLinks {
    type Key = u32;
    type Fields = u32;
    fn key(key: u128) -> u32 {
        key as u32
    }
    fn fields(row: &CountryLinkRow) -> u32 {
        row.canonical_slot
    }
}

impl Table for tables::Countries {
    type Key = u32;
    type Fields = Country;
    fn key(key: u128) -> u32 {
        key as u32
    }
    fn fields(row: &CountryRow) -> Country {
        Country {
            slot: row.canonical_slot,
            tag: row.tag.as_slice().into(),
        }
    }
}

/// The dictionary's table index among the keeper's lent views.
const DICTIONARY: usize = 5;

/// A segment's keeper, given the hours the observer observes in order from the segment's first.
#[derive(Default)]
pub(super) struct Keeper {
    /// The view each table's last missing part copied, while the table is not complete.
    copied: [Option<ViewIdentity>; 6],
    /// Each dictionary row's print, by token, while the dictionary is complete.
    prints: Vec<(u32, u128)>,
    /// Their sum.
    print: u128,
    /// The tokens the segment's rows referenced.
    referenced: HashSet<u32>,
}

/// What an hour gave of `T`, its view's last missing copy `copied`.
fn part<T: Table>(
    frame: &DecodedFrame,
    touch: &Touch,
    copied: &mut Option<ViewIdentity>,
) -> Part<T::Key, T::Fields> {
    let view = &frame.tables[T::ID.index()];
    let Ok(rows) = frame.complete::<T>() else {
        if !touch.restated && copied.as_ref().is_some_and(|known| known.is(view)) {
            return Part::Same;
        }
        *copied = Some(view.identity());
        return Part::Missing(Box::new(Missing {
            view: SharedView::from((**view).clone()),
            why: dashboard_base::products::table_missing_reason(frame, T::ID).unwrap_or_default(),
        }));
    };
    *copied = None;
    if touch.restated {
        return Part::Whole(
            rows.iter()
                .map(|row| (T::key(row.key()), T::fields(row)))
                .collect(),
        );
    }
    if touch.written.is_empty() && touch.deleted.is_empty() {
        return Part::Same;
    }
    // Written and deleted keys are each ascending and never shared.
    let mut changed = Vec::with_capacity(touch.written.len() + touch.deleted.len());
    let (mut written, mut deleted) = (0, 0);
    loop {
        let write = touch.written.get(written).map(|&(key, _)| key);
        let delete = touch.deleted.get(deleted).copied();
        match (write, delete) {
            (Some(key), delete) if delete.is_none_or(|delete| key < delete) => {
                written += 1;
                let row = rows
                    .binary_search_by_key(&key, RowCodec::key)
                    .ok()
                    .map(|at| T::fields(&rows[at]));
                changed.push((T::key(key), row));
            }
            (_, Some(key)) => {
                deleted += 1;
                changed.push((T::key(key), None));
            }
            _ => break,
        }
    }
    Part::Changed(changed)
}

/// A dictionary row's print: two 64-bit hashes of its token and literal, which a complete
/// dictionary's rows sum to.
fn print(token: u32, literal: &[u8]) -> u128 {
    let mut high = mix(0x243f_6a88_85a3_08d3 ^ u64::from(token));
    let mut low = mix(0x1319_8a2e_0370_7344 ^ (u64::from(token) << 32) ^ literal.len() as u64);
    for chunk in literal.chunks(8) {
        let mut word = [0; 8];
        word[..chunk.len()].copy_from_slice(chunk);
        let word = u64::from_le_bytes(word);
        high = mix(high ^ word);
        low = mix(low.wrapping_add(word).rotate_left(29) ^ 0xa409_3822_299f_31d0);
    }
    (u128::from(high) << 64) | u128::from(low)
}

fn mix(mut value: u64) -> u64 {
    value ^= value >> 30;
    value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

/// The spelling the dictionary's `rows` give `token`, as an observation interns it.
fn spelling(rows: &[replay_data::rows::LexicalTokenRow], token: u32) -> Option<Box<[u8]>> {
    rows.binary_search_by_key(&token, |row| row.token)
        .ok()
        .map(|at| installed_content_core::parser::c_string(&rows[at].literal).into())
}

impl Keeper {
    fn kept(&mut self, frame: &DecodedFrame, touched: &Touched) -> Kept {
        let [status, folders, tracks, links, countries, _] = &mut self.copied;
        let mut kept = Kept {
            status: part::<tables::DoctrineCountries>(
                frame,
                touched.of(TableId::DoctrineCountries),
                status,
            ),
            folders: part::<tables::DoctrineFolders>(
                frame,
                touched.of(TableId::DoctrineFolders),
                folders,
            ),
            tracks: part::<tables::DoctrineTracks>(
                frame,
                touched.of(TableId::DoctrineTracks),
                tracks,
            ),
            links: part::<tables::CountryLinks>(frame, touched.of(TableId::CountryLinks), links),
            countries: part::<tables::Countries>(frame, touched.of(TableId::Countries), countries),
            dictionary: Dictionary {
                shape: None,
                spellings: Vec::new(),
            },
        };
        self.dictionary(frame, touched.of(TableId::LexicalTokens), &mut kept);
        kept
    }

    /// The dictionary's shape and the spellings `kept`'s rows need.
    fn dictionary(&mut self, frame: &DecodedFrame, touch: &Touch, kept: &mut Kept) {
        let view = &frame.tables[TableId::LexicalTokens.index()];
        let rows = frame.complete::<tables::LexicalTokens>().ok();
        let dictionary = &mut kept.dictionary;
        match rows {
            Some(rows) if touch.restated || self.copied[DICTIONARY].is_some() => {
                self.copied[DICTIONARY] = None;
                self.prints.clear();
                self.prints.reserve(rows.len());
                self.print = 0;
                for row in rows {
                    let print = print(row.token, &row.literal);
                    self.prints.push((row.token, print));
                    self.print = self.print.wrapping_add(print);
                }
                dictionary.shape = Some(Shape::Complete(self.print));
                dictionary.spellings.extend(
                    self.referenced
                        .iter()
                        .map(|&token| (token, spelling(rows, token))),
                );
            }
            Some(rows) if touch.any() => {
                for (key, now) in touch
                    .written
                    .iter()
                    .map(|&(key, _)| (key, true))
                    .chain(touch.deleted.iter().map(|&key| (key, false)))
                {
                    let token = key as u32;
                    let row = now
                        .then(|| rows.binary_search_by_key(&token, |row| row.token).ok())
                        .flatten()
                        .map(|at| print(token, &rows[at].literal));
                    match (
                        self.prints.binary_search_by_key(&token, |&(held, _)| held),
                        row,
                    ) {
                        (Ok(at), Some(print)) => {
                            self.print = self
                                .print
                                .wrapping_sub(self.prints[at].1)
                                .wrapping_add(print);
                            self.prints[at].1 = print;
                        }
                        (Err(at), Some(print)) => {
                            self.print = self.print.wrapping_add(print);
                            self.prints.insert(at, (token, print));
                        }
                        (Ok(at), None) => {
                            self.print = self.print.wrapping_sub(self.prints.remove(at).1);
                        }
                        (Err(_), None) => {}
                    }
                    if self.referenced.contains(&token) {
                        dictionary.spellings.push((token, spelling(rows, token)));
                    }
                }
                dictionary.shape = Some(Shape::Complete(self.print));
            }
            Some(_) => {}
            None => {
                self.prints = Vec::new();
                self.print = 0;
                let copied = &mut self.copied[DICTIONARY];
                if touch.restated || !copied.as_ref().is_some_and(|known| known.is(view)) {
                    *copied = Some(view.identity());
                    dictionary.shape = Some(Shape::Missing(Box::new(Missing {
                        view: SharedView::from((**view).clone()),
                        why: String::new(),
                    })));
                }
            }
        }
        // The tokens the hour's rows reference, spelled once a segment and again as they change.
        let tokens = kept
            .folders
            .rows()
            .flat_map(|folder| {
                std::iter::once(folder.folder)
                    .chain(folder.grand)
                    .chain(folder.completed.iter().copied())
            })
            .chain(kept.tracks.rows().filter_map(|track| track.sub));
        for token in tokens {
            if self.referenced.insert(token)
                && let Some(rows) = rows
            {
                kept.dictionary
                    .spellings
                    .push((token, spelling(rows, token)));
            }
        }
    }
}

impl native::Keeper<Observer> for Keeper {
    fn keep(
        &mut self,
        _: FormatVersion,
        frame: &DecodedFrame,
        touched: &Touched,
    ) -> io::Result<(Box<dyn Operands<Observer>>, usize)> {
        let kept = self.kept(frame, touched);
        let bytes = kept.bytes();
        Ok((Box::new(kept), bytes))
    }
}

impl Operands<Observer> for Kept {
    fn observe(&self, observer: &mut Observer, values: &mut Values) -> io::Result<bool> {
        observer.observe_kept(self, super::mirror::Publish::Whole(values))
    }

    fn observe_changes(
        &self,
        observer: &mut Observer,
        names: &Names<'_>,
        delta: &mut Delta,
    ) -> io::Result<Option<bool>> {
        if !observer.mirror.published() {
            return Ok(None);
        }
        observer
            .observe_kept(self, super::mirror::Publish::Changes { names, delta })
            .map(Some)
    }

    fn restates(&self, observer: &Observer, table: TableId) -> bool {
        observer.mirror.restates(self, table)
    }

    fn held(&self, bytes: &mut HeldBytes) {
        bytes.add(self.bytes());
    }
}
