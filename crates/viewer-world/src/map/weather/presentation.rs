//! Viewer-local native snow/mud graphics refresh, independent of replay hours.

use super::{Refresh, SnowMud};
use map_display_data::Paged;
use std::{io, ops::Range, sync::Arc};
use viewer_foundation::map::presentation::Continuity;

#[derive(Default)]
pub struct Presentation {
    pub size: [u32; 2],
    /// The planes shown, unless [`Self::stale`]: then they are the target's bytes, written to the
    /// texture in rectangles or by a plane swapped in, and copied here only when a path reads them.
    pub texels: Vec<u8>,
    stale: bool,
    /// One rectangle's bytes, rows `columns * 4` apart ([`Self::take_rect`]).
    rect: Vec<u8>,
    /// One byte per cell, 255 where the hatch overlay draws: the cell is unknown in the pending
    /// hour, in the snapshot the texels show, or in the one they still fade from.
    pub unknown: Vec<u8>,
    /// Cells [`Self::unknown`] hatches.
    hatched: usize,
    /// Each [`Self::unknown`] page's hatched cells.
    page_hatched: Vec<u32>,
    /// The pending, shown and faded masks [`Self::unknown`] was last folded from, held so their
    /// pages' identities stay theirs.
    refreshed: [Option<Paged<u8>>; 3],
    unknown_changed: bool,
    shown_unknown: Paged<u8>,
    faded_unknown: Option<Paged<u8>>,
    shared_pending: Option<Arc<SnowMud>>,
    target: Paged<u8>,
    mismatched: Vec<usize>,
    /// Of [`Self::mismatched`], the pages held before the rectangles began: read again at the end.
    kept: usize,
    uploads: Vec<Range<usize>>,
    dirty: bool,
    refresh: Option<Refresh>,
    last_time: Option<f32>,
    last_refresh: f64,
    fade: f32,
}

/// Whether `now` and `was` hold the same page `page`, by allocation.
fn same_page(now: &Option<Paged<u8>>, was: &Option<Paged<u8>>, page: usize) -> bool {
    match (now, was) {
        (None, None) => true,
        (Some(now), Some(was)) => {
            let (pages, held) = (now.pages().len(), was.pages().len());
            (page >= pages && page >= held)
                || (page < pages && page < held && {
                    let (a, b) = (now.page(page), was.page(page));
                    a.len() == b.len() && std::ptr::eq(a.as_ptr(), b.as_ptr())
                })
        }
        _ => false,
    }
}

/// Whether a cell of `bytes` shows its planes unlike its bytes.
fn mismatching(bytes: &[u8]) -> bool {
    bytes.chunks_exact(4).any(|cell| cell[0] != cell[3] || cell[2] != cell[1])
}

fn dirty_pages(
    pending: &Paged<u8>,
    target: &Paged<u8>,
    mismatched: &[usize],
) -> io::Result<map_display_data::retained::Retained<Vec<usize>>> {
    let count = pending
        .changed_page_indices(target)
        .count()
        .checked_add(mismatched.len())
        .ok_or_else(|| io::Error::from(io::ErrorKind::OutOfMemory))?;
    let mut dirty = Vec::new();
    dirty
        .try_reserve_exact(count)
        .map_err(|_| io::Error::from(io::ErrorKind::OutOfMemory))?;
    dirty.extend(pending.changed_page_indices(target));
    dirty.extend_from_slice(mismatched);
    dirty.sort_unstable();
    dirty.dedup();
    Ok(map_display_data::retained::Retained::new(dirty))
}

fn reserve<T>(values: &mut Vec<T>, additional: usize) -> io::Result<()> {
    values
        .try_reserve(additional)
        .map_err(|_| io::Error::from(io::ErrorKind::OutOfMemory))
}

impl Presentation {
    fn reserve(&mut self, snow: &SnowMud) -> io::Result<()> {
        let texels = snow.texels.len().saturating_sub(self.texels.len());
        reserve(&mut self.texels, texels)?;
        let unknown = (snow.texels.len() / 4).saturating_sub(self.unknown.len());
        reserve(&mut self.unknown, unknown)?;
        let pages = snow.texels.pages().len();
        let mismatched = pages.saturating_sub(self.mismatched.len());
        reserve(&mut self.mismatched, mismatched)?;
        let uploads = pages.max(1).saturating_sub(self.uploads.len());
        reserve(&mut self.uploads, uploads)
    }

    fn upload(&mut self, mut range: Range<usize>) {
        let start = self.uploads.partition_point(|old| old.end < range.start);
        let mut end = start;
        while end < self.uploads.len() && self.uploads[end].start <= range.end {
            range.start = range.start.min(self.uploads[end].start);
            range.end = range.end.max(self.uploads[end].end);
            end += 1;
        }
        if start == end {
            self.uploads.insert(start, range);
        } else {
            self.uploads[start] = range;
            self.uploads.drain(start + 1..end);
        }
    }

    /// The planes made current where they are [`Self::stale`]: the target's bytes, which an
    /// immediate grid taken in rectangles or swapped in shows exactly.
    pub fn planes(&mut self) {
        if std::mem::take(&mut self.stale) {
            self.texels.clear();
            for page in self.target.pages() {
                self.texels.extend_from_slice(page);
            }
        }
    }

    pub fn set_shared(&mut self, snow: &Arc<SnowMud>, continuity: Continuity) -> io::Result<bool> {
        let size = [snow.width, snow.height];
        let reset = continuity == Continuity::Reset || self.size != size || self.texels.is_empty();
        if reset {
            self.stale = false;
        } else {
            self.planes();
        }
        let changed = reset
            || self.refresh != snow.refresh
            || !(self
                .shared_pending
                .as_ref()
                .is_some_and(|had| Arc::ptr_eq(had, snow))
                || self
                    .shared_pending
                    .as_ref()
                    .is_some_and(|had| had.texels == snow.texels && had.unknown == snow.unknown));
        if !reset && !changed {
            return Ok(false);
        }
        self.reserve(snow)?;
        let dirty = if !reset && snow.refresh.is_none() {
            Some(dirty_pages(&snow.texels, &self.target, &self.mismatched)?)
        } else {
            None
        };
        self.size = size;
        self.shared_pending = Some(Arc::clone(snow));
        self.refresh = snow.refresh;
        if reset || self.refresh.is_none() {
            self.mismatched.clear();
            if reset {
                self.texels.clear();
                for page in snow.texels.pages() {
                    self.texels.extend_from_slice(page);
                }
                self.uploads.clear();
                self.uploads.push(0..self.texels.len());
                for (page, bytes) in snow.texels.pages().enumerate() {
                    if mismatching(bytes) {
                        self.mismatched.push(page);
                    }
                }
            } else {
                for &page in dirty.as_ref().unwrap().iter() {
                    let start = page * map_display_data::PAGE_ITEMS;
                    let bytes = snow.texels.page(page);
                    let end = start + bytes.len();
                    if self.texels[start..end] != *bytes {
                        self.texels[start..end].copy_from_slice(bytes);
                        self.upload(start..end);
                    }
                    if mismatching(bytes) {
                        self.mismatched.push(page);
                    }
                }
            }
            self.target = snow.texels.clone();
            self.shown_unknown = snow.unknown.clone();
            self.faded_unknown = None;
            self.refresh_unknown();
            self.dirty = false;
            self.fade = 0.0;
            if reset {
                self.last_time = None;
                self.last_refresh = 0.0;
            }
            return Ok(true);
        }
        self.dirty = true;
        self.refresh_unknown();
        Ok(false)
    }

    /// Whether `snow` can be taken a rectangle at a time ([`Self::begin_parts`]) as [`Self::set_shared`]
    /// would take it whole: a reset always; a continuation only while it and the planes are drawn at
    /// once (no graphics refresh pending or due) and at one size. The planes shown are then their
    /// bytes, cells whose planes differ included, as every grid drawn at once leaves them.
    pub fn takes_parts(&self, snow: &SnowMud, continuity: Continuity) -> bool {
        continuity == Continuity::Reset
            || (self.size == [snow.width, snow.height]
                && !self.texels.is_empty()
                && self.refresh.is_none()
                && snow.refresh.is_none()
                && !self.dirty)
    }

    /// Starts taking `snow` in rectangles: a reset sizes the planes, whose every byte the
    /// rectangles then write, and drops a refresh still pending.
    pub fn begin_parts(&mut self, snow: &SnowMud, continuity: Continuity) -> io::Result<()> {
        if continuity == Continuity::Reset {
            self.reserve(snow)?;
            self.size = [snow.width, snow.height];
            self.texels.resize(snow.texels.len(), 0);
            self.mismatched.clear();
            self.dirty = false;
        }
        self.kept = self.mismatched.len();
        self.uploads.clear();
        Ok(())
    }

    /// `snow`'s cells of `rows` and `columns`, rows `columns.len() * 4` bytes apart, for the
    /// texture; the planes they stand for go [`Self::stale`].
    pub fn take_rect(&mut self, snow: &SnowMud, rows: Range<u32>, columns: Range<u32>) -> io::Result<&[u8]> {
        let [width, height] = self.size;
        if rows.end > height || columns.end > width || snow.texels.len() != self.texels.len() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "snow rectangle outside the grid"));
        }
        let page = map_display_data::PAGE_ITEMS;
        self.rect.clear();
        for row in rows.clone() {
            let mut at = (row as usize * width as usize + columns.start as usize) * 4;
            let end = at + columns.len() * 4;
            while at < end {
                let bytes = &snow.texels.page(at / page)[at % page..];
                let take = bytes.len().min(end - at);
                let bytes = &bytes[..take];
                self.rect.extend_from_slice(bytes);
                // Without an early exit, so the check runs as wide as the copy.
                if bytes.chunks_exact(4).fold(false, |any, cell| any | (cell[0] != cell[3]) | (cell[2] != cell[1])) {
                    self.mismatched.push(at / page);
                }
                at += take;
            }
        }
        self.stale = true;
        Ok(&self.rect)
    }

    /// Ends taking `snow` in rectangles, every cell where it differs from the grid held written:
    /// the presentation is then as [`Self::set_shared`] leaves it after taking `snow` whole.
    pub fn finish_parts(&mut self, snow: &Arc<SnowMud>, continuity: Continuity) {
        self.stale = true;
        // A page that mismatched before stays listed while its bytes still do, rectangle or not.
        let (kept, pages) = (std::mem::take(&mut self.kept), snow.texels.pages().len());
        let mut still = 0;
        for at in 0..kept {
            let page = self.mismatched[at];
            if page < pages && mismatching(snow.texels.page(page)) {
                self.mismatched[still] = page;
                still += 1;
            }
        }
        self.mismatched.drain(still..kept);
        self.mismatched.sort_unstable();
        self.mismatched.dedup();
        self.shared_pending = Some(Arc::clone(snow));
        self.refresh = snow.refresh;
        self.target = snow.texels.clone();
        self.shown_unknown = snow.unknown.clone();
        self.faded_unknown = None;
        self.refresh_unknown();
        self.dirty = false;
        self.fade = 0.0;
        if continuity == Continuity::Reset {
            self.last_time = None;
            self.last_refresh = 0.0;
        }
    }

    /// Takes `snow` as written whole to a texture swapped in for the one drawn, its pages whose
    /// cells are shown unlike its bytes `mismatched`: the presentation is then as taking it in
    /// rectangles leaves it. Only when [`Self::takes_parts`] as a continuation.
    pub fn swap_in(&mut self, snow: &Arc<SnowMud>, mismatched: Vec<usize>) {
        (self.mismatched, self.kept) = (mismatched, 0);
        self.finish_parts(snow, Continuity::Continue);
    }

    /// Recomputes the drawn hatch flags from the pending, shown and faded-from masks.
    fn refresh_unknown(&mut self) {
        let count = self.size[0] as usize * self.size[1] as usize;
        let masks = [
            self.shared_pending
                .as_ref()
                .map(|snow| snow.unknown.clone()),
            Some(self.shown_unknown.clone()),
            self.faded_unknown.clone(),
        ];
        let resized = self.unknown.len() != count;
        let mut changed = resized;
        self.unknown.resize(count, 0);
        let pages = count.div_ceil(map_display_data::PAGE_ITEMS);
        if resized || self.page_hatched.len() != pages {
            self.page_hatched.clear();
            self.page_hatched.resize(pages, 0);
            self.refreshed = Default::default();
        }
        let mut flags = [0u8; map_display_data::PAGE_ITEMS];
        for (page, cells) in self
            .unknown
            .chunks_mut(map_display_data::PAGE_ITEMS)
            .enumerate()
        {
            // A page whose three source pages are the ones it was folded from keeps its flags.
            if !resized
                && masks
                    .iter()
                    .zip(&self.refreshed)
                    .all(|(now, was)| same_page(now, was, page))
            {
                continue;
            }
            let flags = &mut flags[..cells.len()];
            flags.fill(0);
            for mask in masks.iter().flatten() {
                if page < mask.pages().len() {
                    for (flag, &byte) in flags.iter_mut().zip(mask.page(page)) {
                        *flag |= byte;
                    }
                }
            }
            let mut hatched = 0;
            let mut differs = 0;
            for (cell, &flag) in cells.iter_mut().zip(flags.iter()) {
                let value = if flag != 0 { u8::MAX } else { 0 };
                hatched += u32::from(value & 1);
                differs |= *cell ^ value;
                *cell = value;
            }
            self.page_hatched[page] = hatched;
            changed |= differs != 0;
        }
        self.hatched = self.page_hatched.iter().map(|&count| count as usize).sum();
        self.refreshed = masks;
        self.unknown_changed |= changed;
    }

    /// Whether [`Self::unknown`] changed since the last call, so the hatched cells need it.
    pub fn take_unknown_upload(&mut self) -> bool {
        std::mem::take(&mut self.unknown_changed)
    }

    /// Cells the hatch overlay draws over; none skips the pass.
    pub fn hatched(&self) -> usize {
        self.hatched
    }

    #[cfg(test)]
    fn pending_texels(&self) -> &[u8] {
        self.shared_pending
            .as_ref()
            .map(|snow| snow.texels.as_slice())
            .unwrap_or_default()
    }

    pub fn advance(&mut self, now: f32, render_delta: f32) -> io::Result<bool> {
        let dt = if self.last_time.is_some() {
            render_delta
        } else {
            0.0
        };
        let last_refresh = if self.last_time.is_none() {
            f64::from(now)
        } else {
            self.last_refresh
        };
        let refreshed = self.dirty
            && self.refresh.is_some_and(|refresh| {
                f64::from(refresh.interval as f32 / 100_000.0) <= f64::from(now) - last_refresh
            });
        let dirty = if refreshed {
            Some(dirty_pages(
                &self.shared_pending.as_ref().expect("pending snow").texels,
                &self.target,
                &self.mismatched,
            )?)
        } else {
            None
        };
        self.last_refresh = last_refresh;
        self.last_time = Some(now);
        if refreshed {
            self.planes();
            self.last_refresh = f64::from(now);
            let pending = self
                .shared_pending
                .as_ref()
                .expect("pending snow")
                .texels
                .clone();
            self.mismatched.clear();
            for &page in dirty.as_ref().unwrap().iter() {
                let start = page * map_display_data::PAGE_ITEMS;
                let target = pending.page(page);
                let end = start + target.len();
                let mut mismatch = false;
                let mut changed = false;
                for (cell, target) in self.texels[start..end]
                    .chunks_exact_mut(4)
                    .zip(target.chunks_exact(4))
                {
                    let next = [cell[3], target[1], cell[1], target[3]];
                    // An immediate install copies raw bytes, so planes shown unlike them stay dirty.
                    mismatch |= next[0] != next[3] || next[2] != next[1] || next[..] != *target;
                    changed |= *cell != next;
                    cell.copy_from_slice(&next);
                }
                if changed {
                    self.upload(start..end);
                }
                if mismatch {
                    self.mismatched.push(page);
                }
            }
            self.target = pending;
            let shown = self
                .shared_pending
                .as_ref()
                .expect("pending snow")
                .unknown
                .clone();
            self.faded_unknown = Some(std::mem::replace(&mut self.shown_unknown, shown));
            self.dirty = false;
            self.fade = 0.0;
        }
        self.fade = (self.fade + dt).clamp(0.0, 1.0);
        let faded = self.fade >= 1.0 && self.faded_unknown.take().is_some();
        if refreshed || faded {
            self.refresh_unknown();
        }
        Ok(refreshed)
    }

    pub fn uploads(&self) -> &[Range<usize>] {
        &self.uploads
    }
    pub fn clear_uploads(&mut self) {
        self.uploads.clear();
    }

    #[cfg(test)]
    pub fn take_uploads(&mut self) -> Vec<Range<usize>> {
        let mut ranges = std::mem::take(&mut self.uploads);
        ranges.sort_unstable_by_key(|range| range.start);
        let mut written = 0;
        for read in 0..ranges.len() {
            let range = ranges[read].clone();
            if written > 0 && range.start <= ranges[written - 1].end {
                ranges[written - 1].end = ranges[written - 1].end.max(range.end);
            } else {
                ranges[written] = range;
                written += 1;
            }
        }
        ranges.truncate(written);
        ranges
    }

    pub fn fade(&self) -> f32 {
        self.fade
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use viewer_foundation::map::presentation::{
        Continuity::{Continue, Reset},
        continuity,
    };

    fn snapshot(snow: u8, mud: u8, identity: u64) -> Arc<SnowMud> {
        Arc::new(SnowMud {
            width: 1,
            height: 1,
            texels: vec![mud, snow, snow, mud].into(),
            unknown: vec![0].into(),
            province_snow: vec![].into(),
            post_factors: None,
            build: Default::default(),
            summary: Default::default(),
            refresh: Some(Refresh {
                interval: 25_000,
                identity,
            }),
        })
    }

    #[test]
    fn pending_upload_ranges_merge_only_touched_bytes_without_new_storage() {
        for count in [3, 1500, 5000] {
            let mut presentation = Presentation::default();
            let mut expected = std::collections::BTreeSet::new();
            for key in (0..count).rev() {
                let start = key * 12;
                for range in [start..start + 4, start + 4..start + 8, start..start + 4] {
                    expected.extend(range.clone());
                    presentation.uploads.push(range);
                }
            }
            let pointer = presentation.uploads.as_ptr();
            let capacity = presentation.uploads.capacity();
            let ranges = presentation.take_uploads();
            assert_eq!(ranges.as_ptr(), pointer);
            assert_eq!(ranges.capacity(), capacity);
            assert_eq!(ranges.len(), count);
            assert!(ranges.windows(2).all(|pair| pair[0].end < pair[1].start));
            assert_eq!(
                ranges
                    .into_iter()
                    .flatten()
                    .collect::<std::collections::BTreeSet<_>>(),
                expected
            );
            assert!(presentation.take_uploads().is_empty());
        }
    }

    #[test]
    fn the_hatch_mask_covers_pending_shown_and_fading_unknown_cells_only() {
        let snapshot = |unknown: [u8; 3], identity: u64| {
            Arc::new(SnowMud {
                width: 3,
                height: 1,
                texels: vec![0; 12].into(),
                unknown: unknown.to_vec().into(),
                province_snow: vec![].into(),
                post_factors: None,
                build: Default::default(),
                summary: Default::default(),
                refresh: Some(Refresh {
                    interval: 25_000,
                    identity,
                }),
            })
        };
        let mut player = Presentation::default();
        player.set_shared(&snapshot([0, 1, 0], 1), Reset).unwrap();
        assert_eq!(player.unknown, [0, 255, 0]);
        assert_eq!(player.hatched(), 1);
        assert!(player.take_unknown_upload());
        assert!(!player.take_unknown_upload(), "one upload per change");
        player.advance(0.0, 0.0).unwrap();
        // Equal bytes with another mask are a change; the texels still show the old cells.
        player
            .set_shared(&snapshot([1, 0, 0], 2), Continue)
            .unwrap();
        assert_eq!(player.unknown, [255, 255, 0]);
        assert!(player.take_unknown_upload());
        // The refresh shows the new hour while its planes still fade from the old one.
        assert!(player.advance(0.5, 0.5).unwrap());
        assert_eq!(player.unknown, [255, 255, 0]);
        assert!(!player.take_unknown_upload());
        player.advance(1.0, 0.5).unwrap();
        assert_eq!(player.fade(), 1.0);
        assert_eq!(player.unknown, [255, 0, 0]);
        assert!(player.take_unknown_upload());
        assert_eq!(player.hatched(), 1);
        // A whole hour clears the hatch once nothing shown or pending is unknown.
        player.set_shared(&snapshot([0, 0, 0], 3), Reset).unwrap();
        assert_eq!(player.unknown, [0, 0, 0]);
        assert_eq!(player.hatched(), 0);
    }

    /// A grid taken a rectangle at a time leaves the planes, the target and the hatch as taking it
    /// whole does, a reset as a continuation, copying only the rectangles' cells.
    #[test]
    fn a_grid_taken_in_rectangles_is_presented_as_one_taken_whole() {
        let (width, height) = (40u32, 30u32);
        let grid = |seed: u8, unknown: Option<usize>| {
            let texels: Vec<u8> = (0..width * height)
                .flat_map(|cell| {
                    let [snow, mud] = [(cell as u8).wrapping_mul(seed), seed];
                    [mud, snow, snow, mud]
                })
                .collect();
            let mut mask = vec![0; (width * height) as usize];
            if let Some(cell) = unknown {
                mask[cell] = 1;
            }
            Arc::new(SnowMud {
                width,
                height,
                texels: texels.into(),
                unknown: mask.into(),
                province_snow: vec![].into(),
                post_factors: None,
                build: Default::default(),
                summary: Default::default(),
                refresh: None,
            })
        };
        let (first, second) = (grid(3, None), grid(5, Some(7)));
        let mut whole = Presentation::default();
        whole.set_shared(&first, Reset).unwrap();
        let mut parts = Presentation::default();
        assert!(parts.takes_parts(&first, Reset));
        parts.begin_parts(&first, Reset).unwrap();
        for band in (0..height).step_by(8) {
            parts.take_rect(&first, band..(band + 8).min(height), 0..width).unwrap();
        }
        parts.finish_parts(&first, Reset);
        parts.planes();
        assert_eq!(parts.texels, whole.texels, "a reset in rectangles writes every plane");
        assert_eq!((parts.unknown.clone(), parts.hatched()), (whole.unknown.clone(), whole.hatched()));

        whole.set_shared(&second, Continue).unwrap();
        assert!(parts.takes_parts(&second, Continue));
        parts.begin_parts(&second, Continue).unwrap();
        assert_eq!(parts.take_rect(&second, 0..height, 0..width).unwrap(), second.texels.to_vec());
        parts.finish_parts(&second, Continue);
        parts.planes();
        assert_eq!(parts.texels, whole.texels, "a continuation in rectangles equals one taken whole");
        assert_eq!((parts.unknown.clone(), parts.hatched()), (whole.unknown.clone(), whole.hatched()));
        assert!(parts.take_unknown_upload() && whole.take_unknown_upload(), "the hatch is written again once, in both");
        assert!(parts.target == whole.target && parts.mismatched == whole.mismatched);

        // One cell changed: the rectangle holding it alone is copied, and nothing else moves.
        let third = Arc::new(SnowMud { texels: second.texels.edited((0..4).map(|channel| ((12 * width as usize + 9) * 4 + channel, 77))), ..SnowMud::clone(&second) });
        let before = parts.texels.clone();
        parts.begin_parts(&third, Continue).unwrap();
        let rect: Vec<u8> = (8..16).flat_map(|row| third.texels.to_vec()[(row * width as usize + 9) * 4..(row * width as usize + 10) * 4].to_vec()).collect();
        assert_eq!(parts.take_rect(&third, 8..16, 9..10).unwrap(), rect, "the rectangle's bytes alone");
        assert_eq!(parts.texels, before, "a rectangle copies nothing into the planes");
        parts.finish_parts(&third, Continue);
        parts.planes();
        let moved: Vec<usize> = (0..before.len()).filter(|&at| before[at] != parts.texels[at]).collect();
        assert_eq!(moved, ((12 * width as usize + 9) * 4..(12 * width as usize + 9) * 4 + 4).collect::<Vec<_>>());
        whole.set_shared(&third, Continue).unwrap();
        assert_eq!(parts.texels, whole.texels);

        // A cell whose planes differ (on the second page) is taken in rectangles, and so are the
        // grids after it: its page stays listed while its bytes differ, and goes once they agree.
        let mut last = third;
        for (row, column, cell) in [(27, 3, [1, 2, 3, 4]), (2, 5, [6, 6, 6, 6]), (27, 3, [9, 8, 8, 9])] {
            let next = Arc::new(SnowMud { texels: last.texels.edited((0..4).map(|channel| ((row * width as usize + column) * 4 + channel, cell[channel]))), ..SnowMud::clone(&last) });
            whole.set_shared(&next, Continue).unwrap();
            assert!(parts.takes_parts(&next, Continue), "a grid after one whose planes differ is taken in rectangles");
            parts.begin_parts(&next, Continue).unwrap();
            let band = row as u32 / 8 * 8;
            parts.take_rect(&next, band..(band + 8).min(height), column as u32..column as u32 + 1).unwrap();
            parts.finish_parts(&next, Continue);
            parts.planes();
            assert_eq!(parts.texels, whole.texels, "cell {row}/{column}");
            assert_eq!(parts.mismatched, whole.mismatched, "cell {row}/{column}: the pages whose planes differ");
            last = next;
        }
        assert!(parts.mismatched.is_empty());
        let third = last;

        // A refresh pending, or a grid that refreshes, is taken whole.
        let refreshed = Arc::new(SnowMud { refresh: Some(Refresh { interval: 25_000, identity: 1 }), ..SnowMud::clone(&third) });
        assert!(!parts.takes_parts(&refreshed, Continue) && parts.takes_parts(&refreshed, Reset));
    }

    /// A grid written whole to the texture swapped in leaves the presentation as taking it in
    /// rectangles does, with no plane copied; a refreshing grid after it reads the planes it shows.
    #[test]
    fn a_grid_swapped_in_is_presented_as_one_taken_in_rectangles() {
        let grid = |seed: u8, unknown: Option<usize>| {
            let texels: Vec<u8> = (0..24u32).flat_map(|cell| [seed, cell as u8, cell as u8, seed]).collect();
            let mut mask = vec![0; 24];
            if let Some(cell) = unknown {
                mask[cell] = 1;
            }
            Arc::new(SnowMud { width: 6, height: 4, texels: texels.into(), unknown: mask.into(), province_snow: vec![].into(), post_factors: None, build: Default::default(), summary: Default::default(), refresh: None })
        };
        let (first, second) = (grid(3, None), grid(5, Some(2)));
        let (mut parts, mut swapped) = (Presentation::default(), Presentation::default());
        parts.set_shared(&first, Reset).unwrap();
        swapped.set_shared(&first, Reset).unwrap();
        assert!(swapped.takes_parts(&second, Continue));
        parts.begin_parts(&second, Continue).unwrap();
        parts.take_rect(&second, 0..4, 0..6).unwrap();
        parts.finish_parts(&second, Continue);
        let held = swapped.texels.clone();
        swapped.swap_in(&second, Vec::new());
        assert_eq!(swapped.texels, held, "a swap copies no plane");
        for player in [&mut parts, &mut swapped] {
            player.planes();
        }
        assert_eq!((&swapped.texels, &swapped.unknown, swapped.hatched()), (&parts.texels, &parts.unknown, parts.hatched()));
        assert!(swapped.target == parts.target && swapped.mismatched == parts.mismatched && swapped.fade == parts.fade);
        assert_eq!(swapped.take_unknown_upload(), parts.take_unknown_upload());
        // The cross-fade then starts from the planes the swap left shown.
        let fading = Arc::new(SnowMud { refresh: Some(Refresh { interval: 25_000, identity: 1 }), ..SnowMud::clone(&grid(9, None)) });
        let (mut again, mut whole) = (Presentation::default(), Presentation::default());
        for player in [&mut again, &mut whole] {
            player.set_shared(&first, Reset).unwrap();
        }
        again.swap_in(&second, Vec::new());
        whole.set_shared(&second, Continue).unwrap();
        for player in [&mut again, &mut whole] {
            player.set_shared(&fading, Continue).unwrap();
            player.advance(0.0, 0.0).unwrap();
            assert!(player.advance(1.0, 0.5).unwrap());
        }
        assert_eq!(again.texels, whole.texels, "the fade's planes are those the swap showed, as one taken whole shows them");
    }

    #[test]
    fn snow_mud_refresh_uses_graphics_clock_and_native_interrupted_planes() {
        let mut player = Presentation::default();
        player.set_shared(&snapshot(10, 20, 1), Reset).unwrap();
        assert!(!player.advance(0.0, 0.0).unwrap());
        player.advance(0.5, 0.5).unwrap();
        player.advance(1.0, 0.5).unwrap();
        assert_eq!(player.fade(), 1.0);
        player.set_shared(&snapshot(90, 100, 2), Continue).unwrap();
        assert_eq!(
            player.texels,
            [20, 10, 10, 20],
            "hour application only queues work"
        );
        assert!(player.advance(1.125, 0.125).unwrap());
        assert_eq!(player.texels, [20, 90, 10, 100]);
        assert_eq!(
            player.fade(),
            0.125,
            "reset then add this frame's graphics delta"
        );
        player.set_shared(&snapshot(110, 120, 3), Continue).unwrap();
        assert!(
            !player.advance(1.25, 0.125).unwrap(),
            "dirty work waits for the native interval"
        );
        assert_eq!(player.fade(), 0.25);
        player.set_shared(&snapshot(130, 140, 4), Continue).unwrap();
        assert!(
            player.advance(1.375, 0.125).unwrap(),
            "lower-inclusive interval boundary"
        );
        assert_eq!(
            player.texels,
            [100, 130, 90, 140],
            "copy raw current; no blended baseline"
        );
        assert_eq!(player.fade(), 0.125);
        // Freeze the replay entirely: frames alone finish the one-second fade.
        player.advance(1.625, 0.25).unwrap();
        assert_eq!(player.fade(), 0.375);
        player.advance(2.5, 0.5).unwrap();
        assert_eq!(player.fade(), 0.875, "admitted delta caps a stalled frame");
        assert!(!player.set_shared(&snapshot(130, 140, 4), Continue).unwrap());
        assert!(!player.advance(2.625, 0.125).unwrap());
        assert_eq!(player.fade(), 1.0, "unchanged hour does not restart");
        // A full rebuild can publish equal bytes; the native identity catches it.
        player.set_shared(&snapshot(130, 140, 5), Continue).unwrap();
        assert!(player.advance(2.75, 0.125).unwrap());
        assert_eq!(player.fade(), 0.125);
    }

    #[test]
    fn snow_mud_refresh_resets_on_seek_branch_and_perspective() {
        let previous = Some((1, 20, Some("AAA")));
        for (next, sought) in [
            ((1, 20, Some("AAA")), true),
            ((1, 19, Some("AAA")), false),
            ((2, 21, Some("AAA")), false),
            ((1, 21, Some("BBB")), false),
        ] {
            let mut player = Presentation::default();
            player.set_shared(&snapshot(10, 20, 1), Reset).unwrap();
            player.advance(10.0, 0.0).unwrap();
            player.set_shared(&snapshot(90, 100, 2), Continue).unwrap(); // Queued old-hour work.
            let mode = continuity(previous, next, sought, &[]);
            assert_eq!(mode, Reset);
            assert!(player.set_shared(&snapshot(30, 40, 3), mode).unwrap());
            assert_eq!(player.texels, [40, 30, 30, 40]);
            assert_eq!(player.fade(), 0.0);
            assert!(
                !player.advance(300.0, 0.5).unwrap(),
                "old pending refresh was cancelled"
            );
            assert_eq!(player.fade(), 0.0, "reset has its own clock epoch");
            player.advance(300.5, 0.5).unwrap();
            assert_eq!(player.fade(), 0.5);
        }
        assert_eq!(
            continuity(previous, (1, 200, Some("AAA")), false, &[]),
            Continue
        );
        assert_eq!(
            continuity(previous, (1, 20, Some("AAA")), false, &[]),
            Continue
        );
    }

    #[test]
    fn shared_snow_keeps_fixed_planes_across_refreshes_resets_and_skips() {
        let mut player = Presentation::default();
        for cycle in 0..24 {
            let base = cycle as f32 * 4.0;
            let first = snapshot(10, 20, 1);
            assert!(player.set_shared(&first, Reset).unwrap());
            assert!(Arc::ptr_eq(player.shared_pending.as_ref().unwrap(), &first));
            assert_eq!(player.texels, [20, 10, 10, 20]);
            assert!(!player.advance(base, 0.5).unwrap());
            assert_eq!(player.fade().to_bits(), 0.0f32.to_bits());
            assert!(!player.set_shared(&snapshot(10, 20, 1), Continue).unwrap());
            let next = snapshot(90, 100, 2);
            assert!(!player.set_shared(&next, Continue).unwrap());
            assert!(Arc::ptr_eq(player.shared_pending.as_ref().unwrap(), &next));
            assert!(!player.advance(base + 0.125, 0.125).unwrap());
            assert_eq!(player.texels, [20, 10, 10, 20]);
            assert_eq!(player.fade(), 0.125);
            // Replacing pending skipped hours never changes the shown planes.
            assert!(!player.set_shared(&snapshot(110, 120, 3), Continue).unwrap());
            assert!(!player.set_shared(&snapshot(130, 140, 4), Continue).unwrap());
            assert!(player.advance(base + 0.25, 0.125).unwrap());
            assert_eq!(player.texels, [20, 130, 10, 140]);
            assert_eq!(player.fade(), 0.125);
            assert!(!player.advance(base + 0.375, 0.125).unwrap());
            assert_eq!(player.fade(), 0.25);
            // Equal bytes with a new refresh identity still copy actual current
            // planes, not the partially blended result.
            assert!(!player.set_shared(&snapshot(130, 140, 5), Continue).unwrap());
            assert!(player.advance(base + 0.5, 0.125).unwrap());
            assert_eq!(player.texels, [140, 130, 130, 140]);
            assert_eq!(player.fade(), 0.125);
            let mut immediate = snapshot(200, 201, 6);
            Arc::get_mut(&mut immediate).unwrap().refresh = None;
            assert!(player.set_shared(&immediate, Continue).unwrap());
            assert_eq!(player.texels, [201, 200, 200, 201]);
            assert!(!player.advance(base + 0.625, 0.125).unwrap());
            assert_eq!(player.fade(), 0.125);
            let mut resized = snapshot(30, 40, 7);
            let grid = Arc::get_mut(&mut resized).unwrap();
            grid.width = 2;
            grid.texels.extend_from_within(..);
            assert!(player.set_shared(&resized, Continue).unwrap());
            assert_eq!(player.size, [2, 1]);
            assert_eq!(player.texels, [40, 30, 30, 40, 40, 30, 30, 40]);
            assert_eq!(player.pending_texels(), resized.texels);
            assert!(!player.advance(base + 0.75, 0.5).unwrap());
            assert_eq!(player.fade().to_bits(), 0.0f32.to_bits());
        }
    }

    // Frozen texel-first install reference, independent of the production refresh-first path.
    fn set_reference(
        player: &mut Presentation,
        snow: &Arc<SnowMud>,
        continuity: Continuity,
    ) -> bool {
        let size = [snow.width, snow.height];
        let reset =
            continuity == Continuity::Reset || player.size != size || player.texels.is_empty();
        let same = player
            .shared_pending
            .as_ref()
            .is_some_and(|had| Arc::ptr_eq(had, snow))
            || player.pending_texels() == snow.texels;
        let changed = !same || player.refresh != snow.refresh;
        if !reset && !changed {
            return false;
        }
        player.size = size;
        player.shared_pending = Some(Arc::clone(snow));
        player.refresh = snow.refresh;
        if reset || player.refresh.is_none() {
            player.texels = snow.texels.to_vec();
            player.dirty = false;
            player.fade = 0.0;
            if reset {
                player.last_time = None;
                player.last_refresh = 0.0;
            }
            return true;
        }
        player.dirty = true;
        false
    }

    fn advance_reference(player: &mut Presentation, now: f32, delta: f32) -> bool {
        let dt = if player.last_time.is_some() {
            delta
        } else {
            0.0
        };
        if player.last_time.is_none() {
            player.last_refresh = f64::from(now);
        }
        player.last_time = Some(now);
        let refreshed = player.dirty
            && player.refresh.is_some_and(|refresh| {
                f64::from(refresh.interval as f32 / 100_000.0)
                    <= f64::from(now) - player.last_refresh
            });
        if refreshed {
            player.last_refresh = f64::from(now);
            let pending = player.pending_texels().to_vec();
            for (cell, target) in player
                .texels
                .chunks_exact_mut(4)
                .zip(pending.chunks_exact(4))
            {
                cell[0] = cell[3];
                cell[2] = cell[1];
                cell[1] = target[1];
                cell[3] = target[3];
            }
            player.dirty = false;
            player.fade = 0.0;
        }
        player.fade = (player.fade + dt).clamp(0.0, 1.0);
        refreshed
    }

    #[test]
    fn shared_snapshots_and_refresh_first_keep_every_hour_and_frame_equal() {
        let snapshot = |hour: u32, branch: u32| {
            let width = 1 + branch + map_display_data::PAGE_ITEMS as u32 / 4;
            let bytes = (hour / 3 % 4) as u8 * 30;
            let texels: Vec<u8> = (0..width * 4)
                .map(|index| bytes.wrapping_add(index as u8))
                .collect();
            SnowMud {
                width,
                height: 1,
                texels: texels.into(),
                unknown: vec![0; width as usize].into(),
                province_snow: vec![].into(),
                post_factors: None,
                build: std::time::Duration::from_micros(u64::from(hour)),
                summary: Default::default(),
                refresh: (hour % 11 != 10).then(|| Refresh {
                    interval: 25_000,
                    identity: u64::from(hour / 5),
                }),
            }
        };
        let mut arms: [Presentation; 2] = Default::default();
        let mut interned: Vec<Arc<SnowMud>> = Vec::new();
        let mut time = 0.0f32;
        let mut shared = 0;
        for hour in 0..600u32 {
            let branch = hour / 97 % 2;
            let continuity = if hour % 97 == 0 || hour % 41 == 17 {
                Continuity::Reset
            } else {
                Continuity::Continue
            };
            let fresh = Arc::new(snapshot(hour, branch));
            let held = match interned.iter().find(|held| ***held == *fresh) {
                Some(held) => {
                    shared += 1;
                    Arc::clone(held)
                }
                None => {
                    interned.insert(0, Arc::new(snapshot(hour, branch)));
                    interned.truncate(8);
                    Arc::clone(&interned[0])
                }
            };
            let installed = [
                set_reference(&mut arms[0], &fresh, continuity),
                arms[1].set_shared(&held, continuity).unwrap(),
            ];
            assert_eq!(installed[0], installed[1], "hour {hour}");
            for frame in 0..1 + hour % 3 {
                time += [0.125, 0.0, 0.5][(hour + frame) as usize % 3];
                let delta = [0.125, 0.0, 0.25][(hour + frame) as usize % 3];
                let refreshed = [
                    advance_reference(&mut arms[0], time, delta),
                    arms[1].advance(time, delta).unwrap(),
                ];
                assert_eq!(refreshed[0], refreshed[1], "hour {hour} frame {frame}");
                let [old, new] = &arms;
                assert_eq!(old.size, new.size);
                assert_eq!(old.texels, new.texels);
                assert_eq!(old.pending_texels(), new.pending_texels());
                assert_eq!(old.dirty, new.dirty);
                assert_eq!(old.refresh, new.refresh);
                assert_eq!(
                    old.last_time.map(f32::to_bits),
                    new.last_time.map(f32::to_bits)
                );
                assert_eq!(old.last_refresh.to_bits(), new.last_refresh.to_bits());
                assert_eq!(old.fade.to_bits(), new.fade.to_bits(), "hour {hour}");
            }
        }
        assert!(shared > 100, "{shared} shared snapshots");
    }

    fn steady_render_delta() -> super::super::RenderDelta {
        let mut delta = super::super::RenderDelta::default();
        delta.advance(0.0);
        for frame in 1..=256 {
            delta.advance(frame as f32 * 0.125);
        }
        delta
    }

    #[test]
    fn snow_mud_refresh_stall_uses_admitted_delta_and_renders_halfway_snow() {
        let mut delta = steady_render_delta();
        let mut player = Presentation::default();
        player.set_shared(&snapshot(10, 20, 1), Reset).unwrap();
        player.advance(32.0, delta.advance(32.0)).unwrap();
        player.set_shared(&snapshot(90, 100, 2), Continue).unwrap();
        let admitted = delta.advance(34.0);
        assert_eq!(admitted, 0.5);
        assert!(
            player.advance(34.0, admitted).unwrap(),
            "raw two-second clock admits refresh"
        );
        assert_eq!(player.texels, [20, 90, 10, 100]);
        assert_eq!(
            player.fade(),
            0.5,
            "refresh resets then adds admitted f32 delta"
        );
        let snow = f32::from(player.texels[2]) * (1.0 - player.fade())
            + f32::from(player.texels[1]) * player.fade();
        assert_eq!(
            snow, 50.0,
            "old unbounded elapsed input rendered snow 90 immediately"
        );
    }

    #[test]
    fn snow_mud_refresh_irregular_frames_keep_refresh_clock_separate_from_fade() {
        let mut delta = steady_render_delta();
        let mut player = Presentation::default();
        player.set_shared(&snapshot(10, 20, 1), Reset).unwrap();
        player.advance(32.0, delta.advance(32.0)).unwrap();
        player.set_shared(&snapshot(90, 100, 2), Continue).unwrap();
        assert!(player.advance(32.25, delta.advance(32.25)).unwrap());
        assert_eq!(
            player.fade(),
            0.1875,
            "half trimmed mean plus half current frame"
        );
        player.set_shared(&snapshot(110, 120, 3), Continue).unwrap();
        assert!(!player.advance(32.375, delta.advance(32.375)).unwrap());
        assert_eq!(player.fade(), 0.3125);
        assert!(
            player.advance(32.5, delta.advance(32.5)).unwrap(),
            "raw clock reaches 0.25 interval"
        );
        assert_eq!(
            player.texels,
            [100, 110, 90, 120],
            "interrupted fade copies raw current planes"
        );
        assert_eq!(player.fade(), 0.125);
        assert!(!player.advance(32.5, delta.advance(32.5)).unwrap());
        assert_eq!(
            player.fade(),
            0.125,
            "frozen presentation retry preserves fade"
        );
    }
}
