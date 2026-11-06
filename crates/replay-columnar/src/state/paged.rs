//! Slot-indexed storage in pages shared copy-on-write: a table cloned from another shares
//! every page until it writes one, and then copies that page alone. A page held weakly
//! elsewhere ([`Weak`]) is copied on its first write too, so a restart's decoded tables can be
//! found again by later walks without holding pages the walks have since replaced. Every
//! page's items are allocated fallibly: only its small fixed header is not.

use std::{
    fmt, io,
    ops::Index,
    sync::{Arc, Weak},
};

pub(crate) const SHIFT: u32 = 10;
/// Slots per page.
pub const PAGE: usize = 1 << SHIFT;
const MASK: usize = PAGE - 1;

/// A page: its items in a box allocated fallibly, behind a fixed-size shared header.
type Page<T> = Arc<Box<[T]>>;

/// A page of `len` items: those of `from`, then `fill`.
fn page<T: Copy>(from: &[T], len: usize, fill: T) -> io::Result<Page<T>> {
    let mut items = engine_allocation::vec(len)?;
    items.extend_from_slice(from);
    items.resize(len, fill);
    Ok(Arc::new(engine_allocation::boxed_slice(items)?))
}

/// The items the page holding slots `start..` takes for a column of `len` slots: a whole page,
/// or the last one at the next power of two its slots reach, so growing it is copied rarely.
fn page_len(start: usize, len: usize) -> usize {
    (len - start).min(PAGE).next_power_of_two().min(PAGE)
}

/// `page` held by nothing else, copied first when it is shared.
fn unique<T: Copy>(page: &mut Page<T>) -> io::Result<&mut [T]> {
    if Arc::get_mut(page).is_none() {
        let copied = engine_allocation::copy_vec(&page[..])?;
        *page = Arc::new(engine_allocation::boxed_slice(copied)?);
    }
    Ok(&mut Arc::get_mut(page).expect("a page just copied is held once")[..])
}

/// The bytes `page`'s allocations take, its shared header included.
fn page_bytes<T>(page: &Page<T>) -> usize {
    2 * size_of::<usize>() + size_of::<Box<[T]>>() + size_of_val(&***page)
}

/// One value per slot, `len` slots, in pages of [`PAGE`].
#[derive(Clone, Debug)]
pub struct Paged<T> {
    pages: Vec<Page<T>>,
    len: usize,
    fill: T,
}

impl<T: Copy + Default> Default for Paged<T> {
    fn default() -> Self {
        Self {
            pages: Vec::new(),
            len: 0,
            fill: T::default(),
        }
    }
}

impl<T: Copy + PartialEq> PartialEq for Paged<T> {
    fn eq(&self, other: &Self) -> bool {
        self.len == other.len && self.iter().eq(other.iter())
    }
}

impl<T: Copy + Eq> Eq for Paged<T> {}

impl<T: Copy> Index<usize> for Paged<T> {
    type Output = T;

    #[inline]
    fn index(&self, at: usize) -> &T {
        assert!(at < self.len, "slot {at} past {} slots", self.len);
        &self.pages[at >> SHIFT][at & MASK]
    }
}

impl<T: Copy> Paged<T> {
    /// `len` slots of `fill`, which slots it grows to hold too.
    pub(crate) fn filled(len: usize, fill: T) -> io::Result<Self> {
        let mut paged = Self {
            pages: Vec::new(),
            len: 0,
            fill,
        };
        paged.resize(len)?;
        Ok(paged)
    }

    /// `values`, one per slot, its growth `fill`.
    pub(crate) fn from_slice(values: &[T], fill: T) -> io::Result<Self> {
        let mut pages = engine_allocation::vec(values.len().div_ceil(PAGE))?;
        for (index, chunk) in values.chunks(PAGE).enumerate() {
            pages.push(page(chunk, page_len(index << SHIFT, values.len()), fill)?);
        }
        Ok(Self {
            pages,
            len: values.len(),
            fill,
        })
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline]
    pub fn get(&self, at: usize) -> Option<&T> {
        (at < self.len).then(|| &self.pages[at >> SHIFT][at & MASK])
    }

    /// Every slot's value, in slot order.
    pub fn iter(&self) -> impl Iterator<Item = &T> + '_ {
        self.chunks().flatten()
    }

    /// Each page's slots, in order: slot `PAGE * index + offset` is the chunk's `offset`th.
    pub fn chunks(&self) -> impl Iterator<Item = &[T]> + '_ {
        self.pages.iter().enumerate().map(|(index, page)| {
            let start = index << SHIFT;
            &page[..(self.len - start).min(PAGE)]
        })
    }

    /// Holds `len` slots, the new ones `fill`; never fewer.
    pub(crate) fn resize(&mut self, len: usize) -> io::Result<()> {
        if len <= self.len {
            return Ok(());
        }
        let start = self.pages.len().saturating_sub(1) << SHIFT;
        if let Some(last) = self.pages.last_mut() {
            let want = page_len(start, len);
            if want > last.len() {
                *last = page(&last[..], want, self.fill)?;
            }
        }
        let count = len.div_ceil(PAGE);
        let missing = count - self.pages.len();
        engine_allocation::reserve(&mut self.pages, missing)?;
        while self.pages.len() < count {
            let start = self.pages.len() << SHIFT;
            self.pages.push(page(&[], page_len(start, len), self.fill)?);
        }
        self.len = len;
        Ok(())
    }

    /// Slot `at`, its page copied first when it is shared; None past the slots.
    pub(crate) fn get_mut(&mut self, at: usize) -> io::Result<Option<&mut T>> {
        if at >= self.len {
            return Ok(None);
        }
        Ok(Some(&mut unique(&mut self.pages[at >> SHIFT])?[at & MASK]))
    }

    /// Calls `write` with each of `slots` (ascending) and its value, each page copied once
    /// when it is shared; `past` refuses a slot past the slots.
    pub(crate) fn each_mut(
        &mut self,
        slots: &[u32],
        past: impl Fn(u32) -> io::Error,
        mut write: impl FnMut(usize, &mut T) -> io::Result<()>,
    ) -> io::Result<()> {
        let mut index = 0;
        while index < slots.len() {
            let at = slots[index] as usize;
            if at >= self.len {
                return Err(past(slots[index]));
            }
            let number = at >> SHIFT;
            let page = unique(&mut self.pages[number])?;
            while let Some(&slot) = slots.get(index) {
                let at = slot as usize;
                if at >> SHIFT != number {
                    break;
                }
                if at >= self.len {
                    return Err(past(slot));
                }
                write(index, &mut page[at & MASK])?;
                index += 1;
            }
        }
        Ok(())
    }

    /// A copy sharing every page.
    pub(crate) fn share(&self) -> io::Result<Self> {
        Ok(Self {
            pages: engine_allocation::collect_vec(self.pages.iter().cloned())?,
            len: self.len,
            fill: self.fill,
        })
    }

    /// The bytes its pages and their list take.
    pub fn held_bytes(&self) -> usize {
        self.pages.capacity() * size_of::<Page<T>>()
            + self.pages.iter().map(page_bytes).sum::<usize>()
    }

    /// [`Self::held_bytes`], but of the pages nothing else holds.
    pub fn owned_bytes(&self) -> usize {
        self.pages.capacity() * size_of::<Page<T>>()
            + self
                .pages
                .iter()
                .filter(|page| Arc::strong_count(page) == 1)
                .map(page_bytes)
                .sum::<usize>()
    }

    /// The bytes of the pages only it and `other` hold, each at the same place in both.
    pub fn shared_bytes(&self, other: &Self) -> usize {
        self.pages
            .iter()
            .zip(&other.pages)
            .filter(|(mine, theirs)| Arc::strong_count(mine) == 2 && Arc::ptr_eq(mine, theirs))
            .map(|(page, _)| page_bytes(page))
            .sum()
    }

    pub(crate) fn downgrade(&self) -> io::Result<WeakPaged<T>> {
        Ok(WeakPaged {
            pages: engine_allocation::collect_vec(self.pages.iter().map(Arc::downgrade))?,
            len: self.len,
            fill: self.fill,
        })
    }
}

/// A [`Paged`] whose pages are held weakly.
#[derive(Debug)]
pub(crate) struct WeakPaged<T> {
    pages: Vec<Weak<Box<[T]>>>,
    len: usize,
    fill: T,
}

impl<T: Copy> WeakPaged<T> {
    /// Its column again, when every page is still held somewhere.
    pub(crate) fn upgrade(&self) -> io::Result<Option<Paged<T>>> {
        let mut pages = engine_allocation::vec(self.pages.len())?;
        for page in &self.pages {
            let Some(page) = page.upgrade() else {
                return Ok(None);
            };
            pages.push(page);
        }
        Ok(Some(Paged {
            pages,
            len: self.len,
            fill: self.fill,
        }))
    }

    /// Whether its first page is still held: a cheap guess at whether any is. An empty
    /// column holds nothing to share.
    pub(crate) fn alive(&self) -> bool {
        self.pages
            .first()
            .is_some_and(|page| page.strong_count() > 0)
    }
}

/// One bit per slot, in [`Paged`] words: bits past its words are clear.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PagedBits {
    words: Paged<u64>,
}

static BITS: [bool; 2] = [false, true];

impl PagedBits {
    #[inline]
    pub fn get(&self, bit: u32) -> bool {
        self.words
            .get(bit as usize / 64)
            .is_some_and(|word| word >> (bit % 64) & 1 == 1)
    }

    /// The set bits, ascending.
    pub fn ones(&self) -> impl Iterator<Item = u32> + '_ {
        self.words.iter().enumerate().flat_map(|(index, &word)| {
            let mut rest = word;
            std::iter::from_fn(move || {
                (rest != 0).then(|| {
                    let bit = rest.trailing_zeros();
                    rest &= rest - 1;
                    index as u32 * 64 + bit
                })
            })
        })
    }

    pub fn count(&self) -> usize {
        self.words
            .iter()
            .map(|word| word.count_ones() as usize)
            .sum()
    }

    /// Holds bits `..bits` without growing.
    pub(crate) fn resize(&mut self, bits: usize) -> io::Result<()> {
        self.words.resize(bits.div_ceil(64))
    }

    pub(crate) fn put(&mut self, bit: u32, value: bool) -> io::Result<()> {
        let word = bit as usize / 64;
        if value {
            self.words.resize(word + 1)?;
        }
        if let Some(held) = self.words.get_mut(word)? {
            let mask = 1 << (bit % 64);
            *held = if value { *held | mask } else { *held & !mask };
        }
        Ok(())
    }

    /// Sets each of `bits` (ascending) to `value(index)`, each page copied once.
    pub(crate) fn put_each(
        &mut self,
        bits: &[u32],
        mut value: impl FnMut(usize) -> io::Result<bool>,
    ) -> io::Result<()> {
        if let Some(&last) = bits.last() {
            self.words.resize(last as usize / 64 + 1)?;
        }
        // Slots are ascending, so their words are too, each repeated per bit.
        let mut index = 0;
        while index < bits.len() {
            let word = bits[index] as usize / 64;
            let held = self
                .words
                .get_mut(word)?
                .expect("the words were grown to the last bit");
            while let Some(&bit) = bits.get(index) {
                if bit as usize / 64 != word {
                    break;
                }
                let mask = 1 << (bit % 64);
                *held = if value(index)? {
                    *held | mask
                } else {
                    *held & !mask
                };
                index += 1;
            }
        }
        Ok(())
    }

    pub(crate) fn share(&self) -> io::Result<Self> {
        Ok(Self {
            words: self.words.share()?,
        })
    }

    pub fn held_bytes(&self) -> usize {
        self.words.held_bytes()
    }

    pub fn owned_bytes(&self) -> usize {
        self.words.owned_bytes()
    }

    pub fn shared_bytes(&self, other: &Self) -> usize {
        self.words.shared_bytes(&other.words)
    }

    pub(crate) fn downgrade(&self) -> io::Result<WeakPaged<u64>> {
        self.words.downgrade()
    }

    pub(crate) fn upgrade(weak: &WeakPaged<u64>) -> io::Result<Option<Self>> {
        Ok(weak.upgrade()?.map(|words| Self { words }))
    }
}

/// A bool per slot, `len` slots, packed as [`PagedBits`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Bools {
    bits: PagedBits,
    len: usize,
}

impl Index<usize> for Bools {
    type Output = bool;

    #[inline]
    fn index(&self, at: usize) -> &bool {
        self.get(at).expect("a slot past the column")
    }
}

impl Bools {
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Slot `at`'s value, None past the slots.
    #[inline]
    pub fn get(&self, at: usize) -> Option<&'static bool> {
        (at < self.len).then(|| &BITS[usize::from(self.bits.get(at as u32))])
    }

    pub fn iter(&self) -> impl Iterator<Item = bool> + '_ {
        (0..self.len).map(|at| self.bits.get(at as u32))
    }

    pub(crate) fn resize(&mut self, len: usize) -> io::Result<()> {
        if len > self.len {
            self.bits.resize(len)?;
            self.len = len;
        }
        Ok(())
    }

    pub(crate) fn put(&mut self, at: u32, value: bool) -> io::Result<()> {
        self.bits.put(at, value)
    }

    pub(crate) fn put_each(
        &mut self,
        slots: &[u32],
        past: impl Fn(u32) -> io::Error,
        value: impl FnMut(usize) -> io::Result<bool>,
    ) -> io::Result<()> {
        if let Some(&slot) = slots.iter().find(|&&slot| slot as usize >= self.len) {
            return Err(past(slot));
        }
        self.bits.put_each(slots, value)
    }

    pub(crate) fn share(&self) -> io::Result<Self> {
        Ok(Self {
            bits: self.bits.share()?,
            len: self.len,
        })
    }

    pub fn held_bytes(&self) -> usize {
        self.bits.held_bytes()
    }

    pub fn owned_bytes(&self) -> usize {
        self.bits.owned_bytes()
    }

    pub fn shared_bytes(&self, other: &Self) -> usize {
        self.bits.shared_bytes(&other.bits)
    }

    pub(crate) fn downgrade(&self) -> io::Result<(WeakPaged<u64>, usize)> {
        Ok((self.bits.downgrade()?, self.len))
    }

    pub(crate) fn upgrade(weak: &(WeakPaged<u64>, usize)) -> io::Result<Option<Self>> {
        Ok(PagedBits::upgrade(&weak.0)?.map(|bits| Self { bits, len: weak.1 }))
    }
}

/// One page of a list column: each slot's cell end in the page's items, which the last
/// slot's end ends.
#[derive(Debug)]
struct ListPage<T> {
    ends: Vec<u32>,
    items: Vec<T>,
}

fn items_end(items: usize) -> io::Result<u32> {
    u32::try_from(items).map_err(|_| crate::invalid("A list page passes 4 GiB of items"))
}

fn put<T: Copy>(items: &mut Vec<T>, kept: &[T]) -> io::Result<()> {
    engine_allocation::reserve(items, kept.len())?;
    items.extend_from_slice(kept);
    Ok(())
}

/// Item position `at` moved by `shift` items.
#[inline]
fn moved(at: u32, shift: isize) -> usize {
    (at as isize + shift) as usize
}

/// What a rewrite's `cell` is: given a slot's index among the rewritten slots and its held
/// items, it puts the slot's new items at the end of the vector it is given.
trait Rewrite<T>: FnMut(usize, &[T], &mut Vec<T>) -> io::Result<()> {}

impl<T, F: FnMut(usize, &[T], &mut Vec<T>) -> io::Result<()>> Rewrite<T> for F {}

/// A rewrite emptying its cells.
fn nothing<T>(_: usize, _: &[T], _: &mut Vec<T>) -> io::Result<()> {
    Ok(())
}

/// One cell a rewrite puts in a page held once: its slot's offset in the page, where its
/// held items start and end there, and where its new ones end in [`Edits::items`].
#[derive(Clone, Copy, Debug)]
struct Edit {
    offset: u32,
    start: u32,
    end: u32,
    new: u32,
}

/// The new cells of a page edited in place, kept from one rewrite to the next.
struct Edits<T> {
    cells: Vec<Edit>,
    items: Vec<T>,
}

/// The most bytes of new items [`Edits`] keeps once a rewrite is done.
const KEPT_EDIT_BYTES: usize = 64 << 10;

impl<T> Default for Edits<T> {
    fn default() -> Self {
        Self {
            cells: Vec::new(),
            items: Vec::new(),
        }
    }
}

impl<T> fmt::Debug for Edits<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Edits").finish_non_exhaustive()
    }
}

impl<T: Copy> Edits<T> {
    fn bytes(&self) -> usize {
        self.cells.capacity() * size_of::<Edit>() + self.items.capacity() * size_of::<T>()
    }

    /// The new cells of `slots`, slots of `page` from slot `base`, the first `first` among
    /// the rewritten slots.
    fn gather(
        &mut self,
        page: &ListPage<T>,
        (base, first): (usize, usize),
        slots: &[u32],
        cell: &mut impl Rewrite<T>,
    ) -> io::Result<()> {
        self.cells.clear();
        self.items.clear();
        engine_allocation::reserve(&mut self.cells, slots.len())?;
        for (index, &slot) in (first..).zip(slots) {
            let offset = slot as usize - base;
            let (start, end) = (page.start(offset), page.ends[offset]);
            cell(
                index,
                &page.items[start as usize..end as usize],
                &mut self.items,
            )?;
            self.cells.push(Edit {
                offset: offset as u32,
                start,
                end,
                new: items_end(self.items.len())?,
            });
        }
        Ok(())
    }

    /// Lets go of a large rewrite's items.
    fn done(&mut self) {
        if self.items.capacity() * size_of::<T>() > KEPT_EDIT_BYTES {
            self.items = Vec::new();
        }
    }
}

impl<T: Copy> ListPage<T> {
    fn empty(slots: usize) -> io::Result<Self> {
        Ok(Self {
            ends: engine_allocation::filled(slots, 0)?,
            items: Vec::new(),
        })
    }

    /// Where slot `at`'s items start: where the slot before's end.
    #[inline]
    fn start(&self, at: usize) -> u32 {
        at.checked_sub(1).map_or(0, |before| self.ends[before])
    }

    #[inline]
    fn cell(&self, at: usize) -> &[T] {
        &self.items[self.start(at) as usize..self.ends[at] as usize]
    }

    fn bytes(&self) -> usize {
        2 * size_of::<usize>()
            + size_of::<Self>()
            + self.ends.capacity() * size_of::<u32>()
            + self.items.capacity() * size_of::<T>()
    }

    /// A copy holding `slots` slots, the new ones empty.
    fn grown(&self, slots: usize) -> io::Result<Self> {
        let mut ends = engine_allocation::vec(slots)?;
        ends.extend_from_slice(&self.ends);
        ends.resize(slots, self.start(self.ends.len()));
        Ok(Self {
            ends,
            items: engine_allocation::copy_vec(&self.items)?,
        })
    }

    /// Holds `slots` slots, the new ones empty.
    fn grow(&mut self, slots: usize) -> io::Result<()> {
        let end = self.start(self.ends.len());
        let missing = slots - self.ends.len();
        engine_allocation::reserve_exact(&mut self.ends, missing)?;
        self.ends.resize(slots, end);
        Ok(())
    }

    /// A copy with the cells of `slots` (its slots from slot `base`) rewritten.
    fn rebuilt(
        &self,
        (base, first): (usize, usize),
        slots: &[u32],
        cell: &mut impl Rewrite<T>,
    ) -> io::Result<Self> {
        let mut items = engine_allocation::vec(self.items.len())?;
        let mut ends = engine_allocation::copy_vec(&self.ends)?;
        let (mut kept, mut shifted, mut shift) = (0, slots[0] as usize - base, 0);
        for (index, &slot) in (first..).zip(slots) {
            let offset = slot as usize - base;
            let (start, end) = (self.start(offset) as usize, self.ends[offset] as usize);
            put(&mut items, &self.items[kept..start])?;
            cell(index, &self.items[start..end], &mut items)?;
            items_end(items.len())?;
            for moving in &mut ends[shifted..offset] {
                *moving = moved(*moving, shift) as u32;
            }
            (kept, shifted, shift) = (end, offset, items.len() as isize - end as isize);
        }
        put(&mut items, &self.items[kept..])?;
        items_end(items.len())?;
        for moving in &mut ends[shifted..] {
            *moving = moved(*moving, shift) as u32;
        }
        Ok(Self { ends, items })
    }

    /// Rewrites the cells of `slots` where every cell from the first of them on is empty:
    /// their new items follow the page's.
    fn append(
        &mut self,
        (base, first): (usize, usize),
        slots: &[u32],
        cell: &mut impl Rewrite<T>,
    ) -> io::Result<()> {
        let held = self.items.len();
        let from = slots[0] as usize - base;
        let mut filled = from;
        for (index, &slot) in (first..).zip(slots) {
            let offset = slot as usize - base;
            let end = self.start(filled);
            self.ends[filled..offset].fill(end);
            let written =
                cell(index, &[], &mut self.items).and_then(|()| items_end(self.items.len()));
            match written {
                Ok(end) => self.ends[offset] = end,
                Err(error) => {
                    self.items.truncate(held);
                    self.ends[from..].fill(held as u32);
                    return Err(error);
                }
            }
            filled = offset + 1;
        }
        let end = self.start(filled);
        self.ends[filled..].fill(end);
        Ok(())
    }

    /// Puts the cells `edits` gathered from it in place of the cells they replace, moving the
    /// items between them only by what the cells before them grew or shrank.
    fn splice(&mut self, edits: &Edits<T>) -> io::Result<()>
    where
        T: Default,
    {
        let cells = &edits.cells[..];
        let held = self.items.len();
        let delta = |at: usize| {
            let before = at.checked_sub(1).map_or(0, |before| cells[before].new);
            let edit = cells[at];
            (edit.new - before) as isize - (edit.end - edit.start) as isize
        };
        let kept = |at: usize| {
            cells[at].end as usize..cells.get(at + 1).map_or(held, |next| next.start as usize)
        };
        let total: isize = (0..cells.len()).map(delta).sum();
        let len = (held as isize + total) as usize;
        items_end(len)?;
        if len > held {
            engine_allocation::resize(&mut self.items, len, T::default())?;
        }
        // Items moving back go first, front to back, then those moving on, back to front:
        // neither overwrites items yet to move.
        let mut shift = 0;
        for at in 0..cells.len() {
            shift += delta(at);
            if shift < 0 {
                self.items
                    .copy_within(kept(at), moved(cells[at].end, shift));
            }
        }
        for at in (0..cells.len()).rev() {
            if shift > 0 {
                self.items
                    .copy_within(kept(at), moved(cells[at].end, shift));
            }
            shift -= delta(at);
        }
        let mut new = 0;
        for (at, edit) in cells.iter().enumerate() {
            let start = moved(edit.start, shift);
            let cell = &edits.items[new as usize..edit.new as usize];
            self.items[start..start + cell.len()].copy_from_slice(cell);
            (shift, new) = (shift + delta(at), edit.new);
        }
        shift = 0;
        for (at, edit) in cells.iter().enumerate() {
            shift += delta(at);
            let to = cells
                .get(at + 1)
                .map_or(self.ends.len(), |next| next.offset as usize);
            for moving in &mut self.ends[edit.offset as usize..to] {
                *moving = moved(*moving, shift) as u32;
            }
        }
        if len < held {
            self.items.truncate(len);
            if self.items.capacity() / 4 > len {
                self.items = engine_allocation::copy_vec(&self.items)?;
            }
        }
        Ok(())
    }
}

/// Rewrites the cells of `slots`, ascending slots of `page` from slot `at.0`, the first
/// `at.1` among the rewritten slots: in place when nothing else holds the page, else in a
/// copy of it.
fn edit_page<T: Copy + Default>(
    page: &mut Arc<ListPage<T>>,
    edits: &mut Edits<T>,
    at: (usize, usize),
    slots: &[u32],
    cell: &mut impl Rewrite<T>,
) -> io::Result<()> {
    match Arc::get_mut(page) {
        None => *page = Arc::new(page.rebuilt(at, slots, cell)?),
        Some(held) if held.start(slots[0] as usize - at.0) as usize == held.items.len() => {
            held.append(at, slots, cell)?
        }
        Some(held) => {
            edits.gather(held, at, slots, cell)?;
            held.splice(edits)?;
            edits.done();
        }
    }
    Ok(())
}

/// A list of items per slot, `len` slots: each page one buffer of items and each slot's end
/// in it, shared copy-on-write and edited in place once held alone.
#[derive(Debug)]
pub struct Lists<T> {
    pages: Vec<Arc<ListPage<T>>>,
    len: usize,
    edits: Edits<T>,
}

impl<T> Default for Lists<T> {
    fn default() -> Self {
        Self {
            pages: Vec::new(),
            len: 0,
            edits: Edits::default(),
        }
    }
}

impl<T> Clone for Lists<T> {
    fn clone(&self) -> Self {
        Self {
            pages: self.pages.clone(),
            len: self.len,
            edits: Edits::default(),
        }
    }
}

impl<T: Copy + PartialEq> PartialEq for Lists<T> {
    fn eq(&self, other: &Self) -> bool {
        self.len == other.len && (0..self.len).all(|at| self.get(at) == other.get(at))
    }
}

impl<T: Copy + Eq> Eq for Lists<T> {}

impl<T: Copy> Index<usize> for Lists<T> {
    type Output = [T];

    #[inline]
    fn index(&self, at: usize) -> &[T] {
        self.get(at).expect("a slot past the column")
    }
}

impl<T: Copy> Lists<T> {
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline]
    pub fn get(&self, at: usize) -> Option<&[T]> {
        (at < self.len).then(|| self.pages[at >> SHIFT].cell(at & MASK))
    }

    pub fn iter(&self) -> impl Iterator<Item = &[T]> + '_ {
        (0..self.len).map(|at| self.pages[at >> SHIFT].cell(at & MASK))
    }

    /// Holds `len` slots, the new ones empty.
    pub(crate) fn resize(&mut self, len: usize) -> io::Result<()> {
        if len <= self.len {
            return Ok(());
        }
        let start = self.pages.len().saturating_sub(1) << SHIFT;
        if let Some(last) = self.pages.last_mut() {
            let want = page_len(start, len);
            if want > last.ends.len() {
                match Arc::get_mut(last) {
                    Some(page) => page.grow(want)?,
                    None => *last = Arc::new(last.grown(want)?),
                }
            }
        }
        let count = len.div_ceil(PAGE);
        let missing = count - self.pages.len();
        engine_allocation::reserve(&mut self.pages, missing)?;
        while self.pages.len() < count {
            let start = self.pages.len() << SHIFT;
            self.pages
                .push(Arc::new(ListPage::empty(page_len(start, len))?));
        }
        self.len = len;
        Ok(())
    }

    pub(crate) fn share(&self) -> io::Result<Self> {
        Ok(Self {
            pages: engine_allocation::collect_vec(self.pages.iter().cloned())?,
            len: self.len,
            edits: Edits::default(),
        })
    }

    pub fn held_bytes(&self) -> usize {
        self.pages.capacity() * size_of::<usize>()
            + self.edits.bytes()
            + self.pages.iter().map(|page| page.bytes()).sum::<usize>()
    }

    pub fn owned_bytes(&self) -> usize {
        self.pages.capacity() * size_of::<usize>()
            + self.edits.bytes()
            + self
                .pages
                .iter()
                .filter(|page| Arc::strong_count(page) == 1)
                .map(|page| page.bytes())
                .sum::<usize>()
    }

    /// The bytes of the pages only it and `other` hold, each at the same place in both.
    pub fn shared_bytes(&self, other: &Self) -> usize {
        self.pages
            .iter()
            .zip(&other.pages)
            .filter(|(mine, theirs)| Arc::strong_count(mine) == 2 && Arc::ptr_eq(mine, theirs))
            .map(|(page, _)| page.bytes())
            .sum()
    }

    pub(crate) fn downgrade(&self) -> io::Result<WeakLists<T>> {
        Ok(WeakLists {
            pages: engine_allocation::collect_vec(self.pages.iter().map(Arc::downgrade))?,
            len: self.len,
        })
    }
}

impl<T: Copy + Default> Lists<T> {
    /// Writes the cells of `slots` (strictly ascending): `cell` puts slot `slots[index]`'s
    /// new items, given its held ones, at the end of the vector it is given.
    pub(crate) fn rewrite(
        &mut self,
        slots: &[u32],
        past: impl Fn(u32) -> io::Error,
        mut cell: impl FnMut(usize, &[T], &mut Vec<T>) -> io::Result<()>,
    ) -> io::Result<()> {
        self.each_page(slots, past, |_, _| true, &mut cell)
    }

    /// Empties the cells of `slots` (strictly ascending); a page whose cells there are empty
    /// already stays as it is.
    pub(crate) fn clear(
        &mut self,
        slots: &[u32],
        past: impl Fn(u32) -> io::Error,
    ) -> io::Result<()> {
        self.each_page(
            slots,
            past,
            |page, (base, run)| {
                run.iter()
                    .any(|&slot| !page.cell(slot as usize - base).is_empty())
            },
            &mut nothing::<T>,
        )
    }

    /// Rewrites the cells of `slots` a page at a time, each page `wanted` takes.
    fn each_page(
        &mut self,
        slots: &[u32],
        past: impl Fn(u32) -> io::Error,
        wanted: impl Fn(&ListPage<T>, (usize, &[u32])) -> bool,
        cell: &mut impl Rewrite<T>,
    ) -> io::Result<()> {
        let mut index = 0;
        while index < slots.len() {
            let number = slots[index] as usize >> SHIFT;
            let mut end = index;
            while let Some(&slot) = slots.get(end) {
                if slot as usize >> SHIFT != number {
                    break;
                }
                if slot as usize >= self.len {
                    return Err(past(slot));
                }
                if end > 0 && slot <= slots[end - 1] {
                    return Err(crate::invalid("Slots of a run are not strictly ascending"));
                }
                end += 1;
            }
            let (base, run) = (number << SHIFT, &slots[index..end]);
            if wanted(&self.pages[number], (base, run)) {
                edit_page(
                    &mut self.pages[number],
                    &mut self.edits,
                    (base, index),
                    run,
                    cell,
                )?;
            }
            index = end;
        }
        Ok(())
    }
}

/// A [`Lists`] whose pages are held weakly.
#[derive(Debug)]
pub(crate) struct WeakLists<T> {
    pages: Vec<Weak<ListPage<T>>>,
    len: usize,
}

impl<T: Copy> WeakLists<T> {
    pub(crate) fn upgrade(&self) -> io::Result<Option<Lists<T>>> {
        let mut pages = engine_allocation::vec(self.pages.len())?;
        for page in &self.pages {
            let Some(page) = page.upgrade() else {
                return Ok(None);
            };
            pages.push(page);
        }
        Ok(Some(Lists {
            pages,
            len: self.len,
            edits: Edits::default(),
        }))
    }
}
