
use std::{
    fmt, io,
    ops::Index,
    sync::{Arc, Weak},
};

pub(crate) const SHIFT: u32 = 8;
pub const PAGE: usize = 1 << SHIFT;
const MASK: usize = PAGE - 1;

type Page<T> = Arc<Box<[T]>>;

#[derive(Clone, Debug)]
enum SlotPage<T> {
    Uniform(T),
    Dense(Page<T>),
}

impl<T: Copy> SlotPage<T> {
    fn ensure_dense(&mut self, cap: usize, count: usize, fill: T) -> io::Result<&mut [T]> {
        if let Self::Uniform(current) = *self {
            let mut items = engine_allocation::vec(cap)?;
            items.resize(count, current);
            items.resize(cap, fill);
            *self = Self::Dense(Arc::new(engine_allocation::boxed_slice(items)?));
        }
        let Self::Dense(arc) = self else {
            unreachable!()
        };
        unique(arc)
    }

    fn bytes(&self) -> usize {
        match self {
            Self::Uniform(_) => 0,
            Self::Dense(page) => page_bytes(page),
        }
    }

    fn owned_bytes(&self) -> usize {
        match self {
            Self::Uniform(_) => 0,
            Self::Dense(page) if Arc::strong_count(page) == 1 => page_bytes(page),
            Self::Dense(_) => 0,
        }
    }
}

enum IterPage<'a, T> {
    Uniform(std::iter::RepeatN<&'a T>),
    Dense(std::slice::Iter<'a, T>),
}

impl<'a, T> Iterator for IterPage<'a, T> {
    type Item = &'a T;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Uniform(iter) => iter.next(),
            Self::Dense(iter) => iter.next(),
        }
    }
}

fn page<T: Copy>(from: &[T], len: usize, fill: T) -> io::Result<Page<T>> {
    let mut items = engine_allocation::vec(len)?;
    items.extend_from_slice(from);
    items.resize(len, fill);
    Ok(Arc::new(engine_allocation::boxed_slice(items)?))
}

fn page_len(start: usize, len: usize) -> usize {
    let need = (len - start).min(PAGE);
    if need <= 16 {
        need.next_power_of_two()
    } else {
        need.next_multiple_of(16).min(PAGE)
    }
}

fn unique<T: Copy>(page: &mut Page<T>) -> io::Result<&mut [T]> {
    if Arc::get_mut(page).is_none() {
        let copied = engine_allocation::copy_vec(&page[..])?;
        *page = Arc::new(engine_allocation::boxed_slice(copied)?);
    }
    Ok(&mut **Arc::get_mut(page).expect("a page just copied is held once"))
}

fn page_bytes<T>(page: &Page<T>) -> usize {
    2 * size_of::<usize>() + size_of::<Box<[T]>>() + size_of_val(&***page)
}

#[derive(Clone, Debug)]
pub struct Paged<T> {
    pages: Vec<SlotPage<T>>,
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
        match &self.pages[at >> SHIFT] {
            SlotPage::Uniform(value) => value,
            SlotPage::Dense(page) => &page[at & MASK],
        }
    }
}

impl<T: Copy + PartialEq> Paged<T> {
    pub(crate) fn filled(len: usize, fill: T) -> io::Result<Self> {
        let mut paged = Self {
            pages: Vec::new(),
            len: 0,
            fill,
        };
        paged.resize(len)?;
        Ok(paged)
    }

    pub(crate) fn from_slice(values: &[T], fill: T) -> io::Result<Self> {
        let mut pages = engine_allocation::vec(values.len().div_ceil(PAGE))?;
        for (index, chunk) in values.chunks(PAGE).enumerate() {
            if let Some(&first) = chunk.first()
                && chunk.iter().all(|&value| value == first)
            {
                pages.push(SlotPage::Uniform(first));
            } else {
                pages.push(SlotPage::Dense(page(
                    chunk,
                    page_len(index << SHIFT, values.len()),
                    fill,
                )?));
            }
        }
        Ok(Self {
            pages,
            len: values.len(),
            fill,
        })
    }

    pub(crate) fn matches_prefix(&self, prefix: &[T]) -> bool {
        if prefix.len() > self.len {
            return false;
        }
        let mut offset = 0;
        for (index, slot) in self.pages.iter().enumerate() {
            let take = (prefix.len() - offset).min((self.len - (index << SHIFT)).min(PAGE));
            if take == 0 {
                break;
            }
            let slice = &prefix[offset..offset + take];
            let matched = match slot {
                SlotPage::Uniform(value) => slice.iter().all(|item| item == value),
                SlotPage::Dense(page) => page[..take] == *slice,
            };
            if !matched {
                return false;
            }
            offset += take;
            if offset == prefix.len() {
                break;
            }
        }
        offset == prefix.len()
    }

    pub(crate) fn resize(&mut self, len: usize) -> io::Result<()> {
        if len <= self.len {
            return Ok(());
        }
        let start = self.pages.len().saturating_sub(1) << SHIFT;
        if let Some(last) = self.pages.last_mut() {
            let want = page_len(start, len);
            match last {
                SlotPage::Uniform(value) => {
                    let used = self.len - start;
                    if used < PAGE && *value != self.fill {
                        let mut items = engine_allocation::vec(want)?;
                        items.resize(used, *value);
                        items.resize(want, self.fill);
                        *last = SlotPage::Dense(Arc::new(engine_allocation::boxed_slice(items)?));
                    }
                }
                SlotPage::Dense(page_arc) => {
                    if want > page_arc.len() {
                        *page_arc = page(&page_arc[..], want, self.fill)?;
                    }
                }
            }
        }
        let count = len.div_ceil(PAGE);
        let missing = count - self.pages.len();
        engine_allocation::reserve(&mut self.pages, missing)?;
        while self.pages.len() < count {
            self.pages.push(SlotPage::Uniform(self.fill));
        }
        self.len = len;
        Ok(())
    }

    pub(crate) fn write_each(
        &mut self,
        slots: &[u32],
        past: impl Fn(u32) -> io::Error,
        mut value: impl FnMut(usize) -> io::Result<T>,
    ) -> io::Result<()> {
        let mut index = 0;
        while index < slots.len() {
            let first_slot = slots[index] as usize;
            if first_slot >= self.len {
                return Err(past(slots[index]));
            }
            let number = first_slot >> SHIFT;
            let mut end = index + 1;
            while let Some(&slot) = slots.get(end) {
                let at = slot as usize;
                if at >> SHIFT != number {
                    break;
                }
                if at >= self.len {
                    return Err(past(slot));
                }
                end += 1;
            }
            let base = number << SHIFT;
            let count = (self.len - base).min(PAGE);
            let cap = page_len(base, self.len);
            if end - index == count {
                let first = value(index)?;
                let mut diverged = None;
                for cursor in (index + 1)..end {
                    let next = value(cursor)?;
                    if next != first {
                        diverged = Some((cursor, next));
                        break;
                    }
                }
                if let Some((cursor, next)) = diverged {
                    let buf = self.pages[number].ensure_dense(cap, count, self.fill)?;
                    for &slot in &slots[index..cursor] {
                        buf[slot as usize & MASK] = first;
                    }
                    buf[slots[cursor] as usize & MASK] = next;
                    for item in (cursor + 1)..end {
                        buf[slots[item] as usize & MASK] = value(item)?;
                    }
                } else {
                    self.pages[number] = SlotPage::Uniform(first);
                }
            } else {
                match self.pages[number] {
                    SlotPage::Uniform(current) => {
                        let mut diverged = None;
                        for cursor in index..end {
                            let next = value(cursor)?;
                            if next != current {
                                diverged = Some((cursor, next));
                                break;
                            }
                        }
                        if let Some((cursor, next)) = diverged {
                            let buf = self.pages[number].ensure_dense(cap, count, self.fill)?;
                            buf[slots[cursor] as usize & MASK] = next;
                            for item in (cursor + 1)..end {
                                buf[slots[item] as usize & MASK] = value(item)?;
                            }
                        }
                    }
                    SlotPage::Dense(_) => {
                        let buf = self.pages[number].ensure_dense(cap, count, self.fill)?;
                        for item in index..end {
                            buf[slots[item] as usize & MASK] = value(item)?;
                        }
                    }
                }
            }
            index = end;
        }
        Ok(())
    }

    pub(crate) fn write_each_optional(
        &mut self,
        slots: &[u32],
        past: impl Fn(u32) -> io::Error,
        mut value: impl FnMut(usize) -> io::Result<Option<T>>,
    ) -> io::Result<()> {
        let mut index = 0;
        while index < slots.len() {
            let first_slot = slots[index] as usize;
            if first_slot >= self.len {
                return Err(past(slots[index]));
            }
            let number = first_slot >> SHIFT;
            let mut end = index + 1;
            while let Some(&slot) = slots.get(end) {
                let at = slot as usize;
                if at >> SHIFT != number {
                    break;
                }
                if at >= self.len {
                    return Err(past(slot));
                }
                end += 1;
            }
            let base = number << SHIFT;
            let count = (self.len - base).min(PAGE);
            let cap = page_len(base, self.len);
            match self.pages[number] {
                SlotPage::Uniform(current) => {
                    let mut diverged = None;
                    for cursor in index..end {
                        if let Some(next) = value(cursor)?
                            && next != current
                        {
                            diverged = Some((cursor, next));
                            break;
                        }
                    }
                    if let Some((cursor, next)) = diverged {
                        let buf = self.pages[number].ensure_dense(cap, count, self.fill)?;
                        buf[slots[cursor] as usize & MASK] = next;
                        for item in (cursor + 1)..end {
                            if let Some(next) = value(item)? {
                                buf[slots[item] as usize & MASK] = next;
                            }
                        }
                    }
                }
                SlotPage::Dense(_) => {
                    let mut first_write = None;
                    for cursor in index..end {
                        if let Some(next) = value(cursor)? {
                            first_write = Some((cursor, next));
                            break;
                        }
                    }
                    if let Some((cursor, next)) = first_write {
                        let buf = self.pages[number].ensure_dense(cap, count, self.fill)?;
                        buf[slots[cursor] as usize & MASK] = next;
                        for item in (cursor + 1)..end {
                            if let Some(next) = value(item)? {
                                buf[slots[item] as usize & MASK] = next;
                            }
                        }
                    }
                }
            }
            index = end;
        }
        Ok(())
    }
}

impl<T: Copy> Paged<T> {
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[inline]
    pub fn get(&self, at: usize) -> Option<&T> {
        (at < self.len).then(|| match &self.pages[at >> SHIFT] {
            SlotPage::Uniform(value) => value,
            SlotPage::Dense(page) => &page[at & MASK],
        })
    }

    pub fn iter(&self) -> impl Iterator<Item = &T> + '_ {
        self.pages.iter().enumerate().flat_map(|(index, slot)| {
            let count = (self.len - (index << SHIFT)).min(PAGE);
            match slot {
                SlotPage::Uniform(value) => IterPage::Uniform(std::iter::repeat_n(value, count)),
                SlotPage::Dense(page) => IterPage::Dense(page[..count].iter()),
            }
        })
    }

    pub(crate) fn get_mut(&mut self, at: usize) -> io::Result<Option<&mut T>> {
        if at >= self.len {
            return Ok(None);
        }
        let number = at >> SHIFT;
        let base = number << SHIFT;
        let count = (self.len - base).min(PAGE);
        let cap = page_len(base, self.len);
        Ok(Some(
            &mut self.pages[number].ensure_dense(cap, count, self.fill)?[at & MASK],
        ))
    }

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
            let base = number << SHIFT;
            let count = (self.len - base).min(PAGE);
            let cap = page_len(base, self.len);
            let page = self.pages[number].ensure_dense(cap, count, self.fill)?;
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

    pub(crate) fn share(&self) -> io::Result<Self> {
        Ok(Self {
            pages: engine_allocation::collect_vec(self.pages.iter().cloned())?,
            len: self.len,
            fill: self.fill,
        })
    }

    pub fn held_bytes(&self) -> usize {
        self.pages.capacity() * size_of::<SlotPage<T>>()
            + self.pages.iter().map(SlotPage::bytes).sum::<usize>()
    }

    pub fn owned_bytes(&self) -> usize {
        self.pages.capacity() * size_of::<SlotPage<T>>()
            + self.pages.iter().map(SlotPage::owned_bytes).sum::<usize>()
    }

    pub fn shared_bytes(&self, other: &Self) -> usize {
        self.pages
            .iter()
            .zip(&other.pages)
            .filter_map(|(mine, theirs)| match (mine, theirs) {
                (SlotPage::Dense(mine), SlotPage::Dense(theirs))
                    if Arc::strong_count(mine) == 2 && Arc::ptr_eq(mine, theirs) =>
                {
                    Some(page_bytes(mine))
                }
                _ => None,
            })
            .sum()
    }

    pub fn common_bytes(&self, other: &Self) -> usize {
        self.pages
            .iter()
            .zip(&other.pages)
            .filter_map(|(mine, theirs)| match (mine, theirs) {
                (SlotPage::Dense(mine), SlotPage::Dense(theirs)) if Arc::ptr_eq(mine, theirs) => {
                    Some(page_bytes(mine))
                }
                _ => None,
            })
            .sum()
    }

    pub(crate) fn downgrade(&self) -> io::Result<WeakPaged<T>> {
        Ok(WeakPaged {
            pages: engine_allocation::collect_vec(self.pages.iter().map(|slot| match slot {
                SlotPage::Uniform(value) => WeakSlotPage::Uniform(*value),
                SlotPage::Dense(page) => WeakSlotPage::Dense(Arc::downgrade(page)),
            }))?,
            len: self.len,
            fill: self.fill,
        })
    }
}

#[derive(Debug)]
enum WeakSlotPage<T> {
    Uniform(T),
    Dense(Weak<Box<[T]>>),
}

#[derive(Debug)]
pub(crate) struct WeakPaged<T> {
    pages: Vec<WeakSlotPage<T>>,
    len: usize,
    fill: T,
}

impl<T: Copy> WeakPaged<T> {
    pub(crate) fn upgrade(&self) -> io::Result<Option<Paged<T>>> {
        let mut pages = engine_allocation::vec(self.pages.len())?;
        for slot in &self.pages {
            match slot {
                WeakSlotPage::Uniform(value) => pages.push(SlotPage::Uniform(*value)),
                WeakSlotPage::Dense(page) => {
                    let Some(page) = page.upgrade() else {
                        return Ok(None);
                    };
                    pages.push(SlotPage::Dense(page));
                }
            }
        }
        Ok(Some(Paged {
            pages,
            len: self.len,
            fill: self.fill,
        }))
    }

    pub(crate) fn alive(&self) -> bool {
        self.pages
            .iter()
            .any(|slot| matches!(slot, WeakSlotPage::Dense(page) if page.strong_count() > 0))
    }
}

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

    pub(crate) fn resize(&mut self, bits: usize) -> io::Result<()> {
        self.words.resize(bits.div_ceil(64))
    }

    pub(crate) fn put(&mut self, bit: u32, value: bool) -> io::Result<()> {
        if self.get(bit) == value {
            return Ok(());
        }
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

    pub(crate) fn put_each(
        &mut self,
        bits: &[u32],
        mut value: impl FnMut(usize) -> io::Result<bool>,
    ) -> io::Result<()> {
        let mut index = 0;
        while index < bits.len() {
            let word = bits[index] as usize / 64;
            let mut set = 0u64;
            let mut clear = 0u64;
            while let Some(&bit) = bits.get(index) {
                if bit as usize / 64 != word {
                    break;
                }
                let mask = 1u64 << (bit % 64);
                if value(index)? {
                    set |= mask;
                } else {
                    clear |= mask;
                }
                index += 1;
            }
            let old = self.words.get(word).copied().unwrap_or(0);
            let updated = (old | set) & !clear;
            if updated != old {
                if word >= self.words.len() {
                    self.words.resize(word + 1)?;
                }
                *self
                    .words
                    .get_mut(word)?
                    .expect("the words were grown to the updated word") = updated;
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

    pub fn common_bytes(&self, other: &Self) -> usize {
        self.words.common_bytes(&other.words)
    }

    pub(crate) fn downgrade(&self) -> io::Result<WeakPaged<u64>> {
        self.words.downgrade()
    }

    pub(crate) fn upgrade(weak: &WeakPaged<u64>) -> io::Result<Option<Self>> {
        Ok(weak.upgrade()?.map(|words| Self { words }))
    }
}

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

    pub fn common_bytes(&self, other: &Self) -> usize {
        self.bits.common_bytes(&other.bits)
    }

    pub(crate) fn downgrade(&self) -> io::Result<(WeakPaged<u64>, usize)> {
        Ok((self.bits.downgrade()?, self.len))
    }

    pub(crate) fn upgrade(weak: &(WeakPaged<u64>, usize)) -> io::Result<Option<Self>> {
        Ok(PagedBits::upgrade(&weak.0)?.map(|bits| Self { bits, len: weak.1 }))
    }
}

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

#[inline]
fn moved(at: u32, shift: isize) -> usize {
    (at as isize + shift) as usize
}

trait Rewrite<T>: FnMut(usize, &[T], &mut Vec<T>) -> io::Result<()> {}

impl<T, F: FnMut(usize, &[T], &mut Vec<T>) -> io::Result<()>> Rewrite<T> for F {}

fn nothing<T>(_: usize, _: &[T], _: &mut Vec<T>) -> io::Result<()> {
    Ok(())
}

#[derive(Clone, Copy, Debug)]
struct Edit {
    offset: u32,
    start: u32,
    end: u32,
    new: u32,
}

struct Edits<T> {
    cells: Vec<Edit>,
    items: Vec<T>,
}

const KEPT_EDIT_BYTES: usize = 0;

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

    fn done(&mut self) {
        if self.items.capacity() * size_of::<T>() > KEPT_EDIT_BYTES {
            self.items = Vec::new();
        }
        if self.cells.capacity() * size_of::<Edit>() > 1024 {
            self.cells = Vec::new();
        }
    }
}

impl<T: Copy> ListPage<T> {
    fn from_edits(slots: usize, edits: &mut Edits<T>) -> io::Result<Self> {
        let mut ends = engine_allocation::filled(slots, 0u32)?;
        let mut filled = 0usize;
        let mut current = 0u32;
        for edit in &edits.cells {
            let offset = edit.offset as usize;
            ends[filled..offset].fill(current);
            ends[offset] = edit.new;
            current = edit.new;
            filled = offset + 1;
        }
        ends[filled..].fill(current);
        let mut items = std::mem::take(&mut edits.items);
        if items.capacity() > items.len().saturating_add(items.len() / 8).max(16)
            && let Ok(compact) = engine_allocation::copy_vec(&items)
        {
            items = compact;
        }
        Ok(Self { ends, items })
    }

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

    fn grown(&self, slots: usize) -> io::Result<Self> {
        let mut ends = engine_allocation::vec(slots)?;
        ends.extend_from_slice(&self.ends);
        ends.resize(slots, self.start(self.ends.len()));
        Ok(Self {
            ends,
            items: engine_allocation::copy_vec(&self.items)?,
        })
    }

    fn grow(&mut self, slots: usize) -> io::Result<()> {
        let end = self.start(self.ends.len());
        let missing = slots - self.ends.len();
        engine_allocation::reserve_exact(&mut self.ends, missing)?;
        self.ends.resize(slots, end);
        Ok(())
    }

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
        if items.capacity() > items.len().saturating_add(items.len() / 8).max(16) {
            items = engine_allocation::copy_vec(&items)?;
        }
        Ok(Self { ends, items })
    }

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
        if self.items.capacity() > self.items.len().saturating_add(self.items.len() / 8).max(16)
            && let Ok(compact) = engine_allocation::copy_vec(&self.items)
        {
            self.items = compact;
        }
        Ok(())
    }

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
            engine_allocation::reserve_exact(&mut self.items, len - held)?;
            self.items.resize(len, T::default());
        }
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
            if self.items.capacity() > len.saturating_add(len / 8).max(16) {
                self.items = engine_allocation::copy_vec(&self.items)?;
            }
        }
        Ok(())
    }
}

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

#[derive(Debug)]
pub struct Lists<T> {
    pages: Vec<Option<Arc<ListPage<T>>>>,
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
        (at < self.len).then(|| match &self.pages[at >> SHIFT] {
            Some(page) => page.cell(at & MASK),
            None => &[],
        })
    }

    pub fn iter(&self) -> impl Iterator<Item = &[T]> + '_ {
        (0..self.len).map(|at| match &self.pages[at >> SHIFT] {
            Some(page) => page.cell(at & MASK),
            None => &[],
        })
    }

    pub(crate) fn resize(&mut self, len: usize) -> io::Result<()> {
        if len <= self.len {
            return Ok(());
        }
        let start = self.pages.len().saturating_sub(1) << SHIFT;
        if let Some(Some(last)) = self.pages.last_mut() {
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
            self.pages.push(None);
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
        self.pages.capacity() * size_of::<Option<Arc<ListPage<T>>>>()
            + self.edits.bytes()
            + self
                .pages
                .iter()
                .filter_map(|page| page.as_ref().map(|page| page.bytes()))
                .sum::<usize>()
    }

    pub fn owned_bytes(&self) -> usize {
        self.pages.capacity() * size_of::<Option<Arc<ListPage<T>>>>()
            + self.edits.bytes()
            + self
                .pages
                .iter()
                .filter_map(|page| {
                    page.as_ref()
                        .filter(|page| Arc::strong_count(page) == 1)
                        .map(|page| page.bytes())
                })
                .sum::<usize>()
    }

    pub fn shared_bytes(&self, other: &Self) -> usize {
        self.pages
            .iter()
            .zip(&other.pages)
            .filter_map(|(mine, theirs)| match (mine, theirs) {
                (Some(mine), Some(theirs))
                    if Arc::strong_count(mine) == 2 && Arc::ptr_eq(mine, theirs) =>
                {
                    Some(mine.bytes())
                }
                _ => None,
            })
            .sum()
    }

    pub fn common_bytes(&self, other: &Self) -> usize {
        self.pages
            .iter()
            .zip(&other.pages)
            .filter_map(|(mine, theirs)| match (mine, theirs) {
                (Some(mine), Some(theirs)) if Arc::ptr_eq(mine, theirs) => Some(mine.bytes()),
                _ => None,
            })
            .sum()
    }

    pub(crate) fn downgrade(&self) -> io::Result<WeakLists<T>> {
        Ok(WeakLists {
            pages: engine_allocation::collect_vec(
                self.pages.iter().map(|page| page.as_ref().map(Arc::downgrade)),
            )?,
            len: self.len,
        })
    }
}

impl<T: Copy + Default> Lists<T> {
    pub(crate) fn rewrite(
        &mut self,
        slots: &[u32],
        past: impl Fn(u32) -> io::Error,
        mut cell: impl FnMut(usize, &[T], &mut Vec<T>) -> io::Result<()>,
    ) -> io::Result<()> {
        self.each_page(slots, past, true, |_, _| true, &mut cell)
    }

    pub(crate) fn clear(
        &mut self,
        slots: &[u32],
        past: impl Fn(u32) -> io::Error,
    ) -> io::Result<()> {
        self.each_page(
            slots,
            past,
            false,
            |page, (base, run)| {
                run.iter()
                    .any(|&slot| !page.cell(slot as usize - base).is_empty())
            },
            &mut nothing::<T>,
        )
    }

    fn each_page(
        &mut self,
        slots: &[u32],
        past: impl Fn(u32) -> io::Error,
        write_empty: bool,
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
            match &mut self.pages[number] {
                Some(page) => {
                    if wanted(page, (base, run)) {
                        edit_page(page, &mut self.edits, (base, index), run, cell)?;
                        if page.items.is_empty() {
                            self.pages[number] = None;
                        }
                    }
                }
                None if write_empty => {
                    self.edits.cells.clear();
                    self.edits.items.clear();
                    engine_allocation::reserve(&mut self.edits.cells, run.len())?;
                    for (cursor, &slot) in (index..).zip(run) {
                        let offset = (slot as usize - base) as u32;
                        cell(cursor, &[], &mut self.edits.items)?;
                        self.edits.cells.push(Edit {
                            offset,
                            start: 0,
                            end: 0,
                            new: items_end(self.edits.items.len())?,
                        });
                    }
                    if !self.edits.items.is_empty() {
                        let cap = page_len(base, self.len);
                        let built = ListPage::from_edits(cap, &mut self.edits)?;
                        self.pages[number] = Some(Arc::new(built));
                    }
                    self.edits.done();
                }
                None => {}
            }
            index = end;
        }
        Ok(())
    }
}

#[derive(Debug)]
pub(crate) struct WeakLists<T> {
    pages: Vec<Option<Weak<ListPage<T>>>>,
    len: usize,
}

impl<T: Copy> WeakLists<T> {
    pub(crate) fn upgrade(&self) -> io::Result<Option<Lists<T>>> {
        let mut pages = engine_allocation::vec(self.pages.len())?;
        for page in &self.pages {
            match page {
                Some(page) => {
                    let Some(page) = page.upgrade() else {
                        return Ok(None);
                    };
                    pages.push(Some(page));
                }
                None => pages.push(None),
            }
        }
        Ok(Some(Lists {
            pages,
            len: self.len,
            edits: Edits::default(),
        }))
    }
}
