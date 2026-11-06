//! The rows a segment appends, held in chunks of a page's bytes: past its first chunk an append
//! never moves what is held, so a segment's columns grow without copying, and its heap seals a
//! chunk to a page.

use crate::pages::PAGE_BYTES;
use std::{
    io,
    mem::size_of,
    ops::{Index, IndexMut},
};

pub(crate) struct Chunked<T> {
    /// Every chunk but the last holds [`Self::PER`] values.
    chunks: Vec<Vec<T>>,
    len: usize,
}

impl<T> Default for Chunked<T> {
    fn default() -> Self {
        Self {
            chunks: Vec::new(),
            len: 0,
        }
    }
}

impl<T> Chunked<T> {
    /// Values a chunk holds: the most of a page's bytes that is a power of two, so an index splits
    /// by a shift.
    const SHIFT: u32 = {
        let size = if size_of::<T>() == 0 { 1 } else { size_of::<T>() };
        let per = PAGE_BYTES / size;
        if per == 0 { 0 } else { per.ilog2() }
    };
    const PER: usize = 1 << Self::SHIFT;
    const MASK: usize = Self::PER - 1;

    /// The bytes its chunks hold.
    pub(crate) fn held(&self) -> usize {
        self.chunks
            .iter()
            .map(|chunk| chunk.capacity() * size_of::<T>())
            .sum()
    }

    pub(crate) fn len(&self) -> usize {
        self.len
    }

    /// The values it has room for.
    fn capacity(&self) -> usize {
        match self.chunks.len() {
            0 => 0,
            1 => self.chunks[0].capacity().min(Self::PER),
            chunks => chunks << Self::SHIFT,
        }
    }

    /// Makes room for `more` further values: the first chunk grows by doubling, as a slab does,
    /// until it holds a whole chunk; then a whole chunk at a time.
    pub(crate) fn reserve(&mut self, more: usize) -> io::Result<()> {
        let need = engine_allocation::add(self.len, more)?;
        let held = self.capacity();
        if need <= held {
            return Ok(());
        }
        if self.chunks.len() <= 1 && held < Self::PER {
            let target = need.max(held.max(4).saturating_mul(2)).min(Self::PER);
            if self.chunks.is_empty() {
                engine_allocation::reserve(&mut self.chunks, 1)?;
                self.chunks.push(Vec::new());
            }
            let first = &mut self.chunks[0];
            let more = target - first.len();
            engine_allocation::reserve_exact(first, more)?;
        }
        while self.capacity() < need {
            engine_allocation::reserve(&mut self.chunks, 1)?;
            self.chunks.push(engine_allocation::vec(Self::PER)?);
        }
        Ok(())
    }

    /// Appends into room [`Self::reserve`] made.
    pub(crate) fn put(&mut self, value: T) {
        self.chunks[self.len >> Self::SHIFT].push(value);
        self.len += 1;
    }

    pub(crate) fn push(&mut self, value: T) -> io::Result<()> {
        self.reserve(1)?;
        self.put(value);
        Ok(())
    }

    pub(crate) fn get(&self, at: usize) -> Option<&T> {
        (at < self.len).then(|| &self.chunks[at >> Self::SHIFT][at & Self::MASK])
    }

    pub(crate) fn first(&self) -> Option<&T> {
        self.get(0)
    }

    /// Keeps its first `len` values; the room after them stays held.
    pub(crate) fn truncate(&mut self, len: usize) {
        if len >= self.len {
            return;
        }
        let at = len >> Self::SHIFT;
        self.chunks[at].truncate(len & Self::MASK);
        for chunk in &mut self.chunks[at + 1..] {
            chunk.clear();
        }
        self.len = len;
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &T> + Clone {
        self.chunks.iter().flat_map(|chunk| chunk.iter())
    }

    pub(crate) fn iter_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.chunks.iter_mut().flat_map(|chunk| chunk.iter_mut())
    }
}

impl<T: Copy> Chunked<T> {
    /// Appends `values`, which may cross chunks.
    pub(crate) fn extend(&mut self, mut values: &[T]) -> io::Result<()> {
        // Most appends fit the chunk they start in.
        let offset = self.len & Self::MASK;
        if let Some(chunk) = self.chunks.get_mut(self.len >> Self::SHIFT)
            && offset + values.len() <= Self::PER
            && values.len() <= chunk.capacity() - chunk.len()
        {
            chunk.extend_from_slice(values);
            self.len += values.len();
            return Ok(());
        }
        self.reserve(values.len())?;
        while !values.is_empty() {
            let room = Self::PER - (self.len & Self::MASK);
            let (now, rest) = values.split_at(room.min(values.len()));
            self.chunks[self.len >> Self::SHIFT].extend_from_slice(now);
            self.len += now.len();
            values = rest;
        }
        Ok(())
    }

    /// The values `start..end` as the chunks hold them, in order.
    pub(crate) fn pieces(&self, start: usize, end: usize) -> impl Iterator<Item = &[T]> {
        let end = end.min(self.len);
        let mut at = start.min(end);
        std::iter::from_fn(move || {
            if at >= end {
                return None;
            }
            let offset = at & Self::MASK;
            let chunk = &self.chunks[at >> Self::SHIFT];
            let take = (Self::PER - offset).min(end - at);
            at += take;
            Some(&chunk[offset..offset + take])
        })
    }

    /// Every value as the chunks hold them, each piece a whole chunk but the last.
    pub(crate) fn whole(&self) -> impl Iterator<Item = &[T]> {
        self.pieces(0, self.len)
    }
}

impl<T> Index<usize> for Chunked<T> {
    type Output = T;

    fn index(&self, at: usize) -> &T {
        assert!(at < self.len, "series row index out of range");
        &self.chunks[at >> Self::SHIFT][at & Self::MASK]
    }
}

impl<T> IndexMut<usize> for Chunked<T> {
    fn index_mut(&mut self, at: usize) -> &mut T {
        assert!(at < self.len, "series row index out of range");
        &mut self.chunks[at >> Self::SHIFT][at & Self::MASK]
    }
}
