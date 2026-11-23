
use map_display_data::{Refresh, SnowMud};
use map_display_data::Paged;
use std::{io, ops::Range, sync::Arc};
use viewer_runtime::presentation::Continuity;

#[derive(Default)]
pub struct Presentation {
    pub size: [u32; 2],
    texels: Vec<u8>,
    shown: Vec<bool>,
    rect: Vec<u8>,
    pub unknown: Vec<u8>,
    coverage: Option<super::HatchCoverage>,
    hatch_whole: bool,
    hatch_writes: Vec<Range<usize>>,
    refreshed: [Option<Paged<u8>>; 3],
    shown_unknown: Paged<u8>,
    faded_unknown: Option<Paged<u8>>,
    shared_pending: Option<Arc<SnowMud>>,
    target: Paged<u8>,
    mismatched: Vec<usize>,
    uploads: Vec<Range<usize>>,
    dirty: bool,
    refresh: Option<Refresh>,
    last_time: Option<f32>,
    last_refresh: f64,
    fade: f32,
}

const MERGE: usize = 32 * 1024;

static CHECKED_GRIDS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn shared_page(now: &Paged<u8>, was: &Paged<u8>, page: usize) -> bool {
    let (pages, held) = (now.pages().len(), was.pages().len());
    (page >= pages && page >= held)
        || (page < pages
            && page < held
            && {
                let (a, b) = (now.page(page), was.page(page));
                a.len() == b.len() && std::ptr::eq(a.as_ptr(), b.as_ptr())
            })
}

fn same_page(now: &Option<Paged<u8>>, was: &Option<Paged<u8>>, page: usize) -> bool {
    match (now, was) {
        (None, None) => true,
        (Some(now), Some(was)) => shared_page(now, was, page),
        _ => false,
    }
}

fn mismatching(bytes: &[u8]) -> bool {
    bytes.chunks_exact(4).fold(0u32, |differs, cell| {
        let cell = u32::from_ne_bytes([cell[0], cell[1], cell[2], cell[3]]);
        differs | (cell ^ cell.swap_bytes())
    }) != 0
}

fn dirty_pages(mut pages: Vec<usize>, mismatched: &[usize]) -> io::Result<map_display_data::retained::Retained<Vec<usize>>> {
    pages
        .try_reserve(mismatched.len())
        .map_err(|_| io::Error::from(io::ErrorKind::OutOfMemory))?;
    pages.extend_from_slice(mismatched);
    pages.sort_unstable();
    pages.dedup();
    Ok(map_display_data::retained::Retained::new(pages))
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
        let shown = snow.texels.pages().len().saturating_sub(self.shown.len());
        reserve(&mut self.shown, shown)?;
        let unknown = (snow.texels.len() / 4).saturating_sub(self.unknown.len());
        reserve(&mut self.unknown, unknown)?;
        let pages = snow.texels.pages().len();
        let mismatched = pages.saturating_sub(self.mismatched.len());
        reserve(&mut self.mismatched, mismatched)?;
        let uploads = pages.max(1).saturating_sub(self.uploads.len());
        reserve(&mut self.uploads, uploads)
    }

    fn upload(&mut self, mut range: Range<usize>) {
        let start = self
            .uploads
            .partition_point(|old| old.end.saturating_add(MERGE) < range.start);
        let mut end = start;
        while end < self.uploads.len()
            && self.uploads[end].start <= range.end.saturating_add(MERGE)
        {
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

    pub fn planes_shown(&mut self) -> &[u8] {
        self.planes();
        &self.texels
    }

    pub fn owed_planes(&mut self) -> (&[Range<usize>], &[u8]) {
        self.planes_owed();
        (&self.uploads, &self.texels)
    }

    fn planes(&mut self) {
        for page in 0..self.shown.len() {
            self.plane_page(page);
        }
    }

    fn plane_page(&mut self, page: usize) -> Range<usize> {
        let start = page * map_display_data::PAGE_ITEMS;
        if std::mem::replace(&mut self.shown[page], true) {
            return start..(start + map_display_data::PAGE_ITEMS).min(self.texels.len());
        }
        let bytes = self.target.page(page);
        let end = start + bytes.len();
        self.texels[start..end].copy_from_slice(bytes);
        start..end
    }

    fn shown_page(&self, page: usize) -> &[u8] {
        let start = page * map_display_data::PAGE_ITEMS;
        if self.shown[page] {
            return &self.texels[start..(start + map_display_data::PAGE_ITEMS).min(self.texels.len())];
        }
        self.target.page(page)
    }

    fn planes_owed(&mut self) {
        let owed: Vec<Range<usize>> = self.uploads.clone();
        for range in owed {
            for page in range.start / map_display_data::PAGE_ITEMS..range.end.div_ceil(map_display_data::PAGE_ITEMS).min(self.shown.len()) {
                self.plane_page(page);
            }
        }
    }

    fn planes_are_the_target(&mut self) {
        self.shown.clear();
        self.shown.resize(self.target.pages().len(), false);
    }

    pub fn set_shared(&mut self, snow: &Arc<SnowMud>, continuity: Continuity) -> io::Result<bool> {
        let size = [snow.width, snow.height];
        let reset = continuity == Continuity::Reset || self.size != size || self.texels.is_empty();
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
            let rebuilt = (0..snow.texels.pages().len()).filter(|&page| !shared_page(&snow.texels, &self.target, page)).collect();
            Some(dirty_pages(rebuilt, &self.mismatched)?)
        } else {
            None
        };
        self.size = size;
        self.shared_pending = Some(Arc::clone(snow));
        self.refresh = snow.refresh;
        if reset || self.refresh.is_none() {
            if reset {
                self.texels.resize(snow.texels.len(), 0);
                self.uploads.clear();
                self.uploads.push(0..snow.texels.len());
            } else {
                for &page in dirty.as_ref().unwrap().iter() {
                    let start = page * map_display_data::PAGE_ITEMS;
                    self.upload(start..start + snow.texels.page(page).len());
                }
                if viewer_runtime::install_check::due(&CHECKED_GRIDS) {
                    let owed = |page: usize| {
                        let start = page * map_display_data::PAGE_ITEMS;
                        self.uploads.iter().any(|range| range.start <= start && start < range.end)
                    };
                    if let Some(page) = (0..snow.texels.pages().len()).find(|&page| !owed(page) && self.shown_page(page) != snow.texels.page(page)) {
                        viewer_runtime::install_check::found(
                            "snow grid",
                            format_args!("page {page} of {} shows planes unlike the grid and is not written", snow.texels.pages().len()),
                        );
                    }
                }
            }
            self.installed_at_once(&snow.texels);
            self.target = snow.texels.clone();
            self.planes_are_the_target();
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

    pub fn takes_parts(&self, snow: &SnowMud, continuity: Continuity) -> bool {
        continuity == Continuity::Reset
            || (self.size == [snow.width, snow.height]
                && !self.texels.is_empty()
                && self.refresh.is_none()
                && snow.refresh.is_none()
                && !self.dirty)
    }

    pub fn begin_parts(&mut self, snow: &SnowMud, continuity: Continuity) -> io::Result<()> {
        if continuity == Continuity::Reset {
            self.reserve(snow)?;
            self.size = [snow.width, snow.height];
            self.texels.resize(snow.texels.len(), 0);
            self.dirty = false;
        }
        self.uploads.clear();
        Ok(())
    }

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
                self.rect.extend_from_slice(&bytes[..take]);
                at += take;
            }
        }
        Ok(&self.rect)
    }

    pub fn finish_parts(&mut self, snow: &Arc<SnowMud>, continuity: Continuity) {
        self.installed_at_once(&snow.texels);
        self.shared_pending = Some(Arc::clone(snow));
        self.refresh = snow.refresh;
        self.target = snow.texels.clone();
        self.planes_are_the_target();
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

    fn refresh_unknown(&mut self) {
        let count = self.size[0] as usize * self.size[1] as usize;
        let masks = [
            self.shared_pending
                .as_ref()
                .map(|snow| snow.unknown.clone()),
            Some(self.shown_unknown.clone()),
            self.faded_unknown.clone(),
        ];
        if self.unknown.len() != count {
            self.unknown.clear();
            self.unknown.resize(count, 0);
            self.refreshed = Default::default();
            self.coverage = None;
            self.hatch_writes.clear();
        }
        let mut folded = [0u8; map_display_data::PAGE_ITEMS];
        for page in 0..count.div_ceil(map_display_data::PAGE_ITEMS) {
            if masks
                .iter()
                .zip(&self.refreshed)
                .all(|(now, was)| same_page(now, was, page))
            {
                continue;
            }
            let first = page * map_display_data::PAGE_ITEMS;
            let folded = &mut folded[..(count - first).min(map_display_data::PAGE_ITEMS)];
            folded.fill(0);
            for mask in masks.iter().flatten() {
                if page < mask.pages().len() {
                    for (flag, &byte) in folded.iter_mut().zip(mask.page(page)) {
                        *flag |= byte;
                    }
                }
            }
            if self.unknown[first..first + folded.len()] == *folded {
                continue;
            }
            for at in 0..folded.len() {
                let (held, now) = (self.unknown[first + at], folded[at]);
                if held == now {
                    continue;
                }
                if let Some(coverage) = self
                    .coverage
                    .as_mut()
                    .filter(|_| (held != 0) != (now != 0))
                {
                    coverage.patch((first + at) as u32, now != 0);
                }
                self.unknown[first + at] = now;
                self.owe_mask(first + at);
            }
        }
        self.refreshed = masks;
        self.settle_coverage();
    }

    fn owe_mask(&mut self, cell: usize) {
        match self.hatch_writes.last_mut() {
            Some(last) if cell + MERGE >= last.start && cell <= last.end.saturating_add(MERGE) => {
                last.start = last.start.min(cell);
                last.end = last.end.max(cell + 1);
            }
            _ => self.hatch_writes.push(cell..cell + 1),
        }
    }

    fn installed_at_once(&mut self, texels: &Paged<u8>) {
        self.mismatched.clear();
        if viewer_runtime::install_check::due(&CHECKED_GRIDS) {
            if let Some(page) = (0..texels.pages().len()).find(|&page| mismatching(texels.page(page))) {
                viewer_runtime::install_check::found(
                    "snow grid",
                    format_args!("page {page} of {} shows planes unlike its bytes, so an install at once owes it", texels.pages().len()),
                );
            }
        }
    }

    fn settle_coverage(&mut self) {
        if self
            .coverage
            .as_ref()
            .is_some_and(|coverage| coverage.counts(self.size))
        {
            return;
        }
        self.coverage = Some(super::HatchCoverage::new(&self.unknown, self.size));
        self.hatch_writes.clear();
        self.hatch_whole = true;
    }

    pub fn coverage(&self) -> Option<&super::HatchCoverage> {
        self.coverage.as_ref()
    }

    pub fn hatch_whole(&self) -> bool {
        self.hatch_whole
    }

    pub fn hatch_writes(&self) -> &[Range<usize>] {
        &self.hatch_writes
    }

    pub fn clear_hatch(&mut self) {
        self.hatch_whole = false;
        self.hatch_writes.clear();
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
            let pending = &self.shared_pending.as_ref().expect("pending snow").texels;
            Some(dirty_pages(pending.changed_pages(&self.target), &self.mismatched)?)
        } else {
            None
        };
        self.last_refresh = last_refresh;
        self.last_time = Some(now);
        if refreshed {
            self.last_refresh = f64::from(now);
            let pending = self
                .shared_pending
                .as_ref()
                .expect("pending snow")
                .texels
                .clone();
            self.mismatched.clear();
            for &page in dirty.as_ref().unwrap().iter() {
                let Range { start, end } = self.plane_page(page);
                let target = pending.page(page);
                let mut mismatch = false;
                let mut changed = false;
                for (cell, target) in self.texels[start..end]
                    .chunks_exact_mut(4)
                    .zip(target.chunks_exact(4))
                {
                    let next = [cell[3], target[1], cell[1], target[3]];
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
    use viewer_runtime::presentation::{Continuity::{Continue, Reset}, continuity};

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
    fn pending_upload_ranges_go_in_one_write_where_a_write_costs_more_than_the_gap() {
        for count in [3, 1500, 5000] {
            let apart = MERGE + 16;
            let mut presentation = Presentation::default();
            for key in (0..count).rev() {
                let start = key * apart;
                for range in [start..start + 4, start + 4..start + 8, start..start + 4] {
                    presentation.upload(range);
                }
            }
            assert_eq!(presentation.uploads.len(), count, "one write a group");
            assert!(
                presentation
                    .uploads
                    .windows(2)
                    .all(|pair| pair[0].end < pair[1].start)
            );
            assert_eq!(
                presentation
                    .uploads
                    .iter()
                    .map(|range| range.len())
                    .sum::<usize>(),
                count * 8,
                "the bytes pushed, with nothing between the groups"
            );
            let mut near = Presentation::default();
            for key in (0..count).rev() {
                near.upload(key * 12..key * 12 + 4);
            }
            assert_eq!(near.uploads, [0..(count - 1) * 12 + 4]);
            let pointer = presentation.uploads.as_ptr();
            let capacity = presentation.uploads.capacity();
            let ranges = presentation.take_uploads();
            assert_eq!(ranges.as_ptr(), pointer);
            assert_eq!(ranges.capacity(), capacity);
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
        assert_eq!(player.unknown, [0, 1, 0]);
        assert!(player.coverage().unwrap().covered());
        assert!(player.hatch_whole(), "the first mask is owed whole");
        player.clear_hatch();
        player.advance(0.0, 0.0).unwrap();
        player
            .set_shared(&snapshot([1, 0, 0], 2), Continue)
            .unwrap();
        assert_eq!(player.unknown, [1, 1, 0]);
        assert_eq!(player.hatch_writes(), [0..1], "the cell that turned, not the mask");
        player.clear_hatch();
        assert!(player.advance(0.5, 0.5).unwrap());
        assert_eq!(player.unknown, [1, 1, 0]);
        assert!(player.hatch_writes().is_empty() && !player.hatch_whole());
        player.advance(1.0, 0.5).unwrap();
        assert_eq!(player.fade(), 1.0);
        assert_eq!(player.unknown, [1, 0, 0]);
        assert_eq!(player.hatch_writes(), [1..2], "the cell that stopped fading");
        assert!(player.coverage().unwrap().covered());
        player.clear_hatch();
        player.set_shared(&snapshot([0, 0, 0], 3), Reset).unwrap();
        assert_eq!(player.unknown, [0, 0, 0]);
        assert!(!player.coverage().unwrap().covered());
        assert_eq!(player.hatch_writes(), [0..1]);
    }

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
        assert_eq!(parts.planes_shown().to_vec(), whole.planes_shown().to_vec(), "a reset in rectangles writes every plane");
        assert_eq!((parts.unknown.clone(), parts.coverage().cloned()), (whole.unknown.clone(), whole.coverage().cloned()));
        (parts.clear_hatch(), whole.clear_hatch());

        whole.set_shared(&second, Continue).unwrap();
        assert!(parts.takes_parts(&second, Continue));
        parts.begin_parts(&second, Continue).unwrap();
        assert_eq!(parts.take_rect(&second, 0..height, 0..width).unwrap(), second.texels.to_vec());
        parts.finish_parts(&second, Continue);
        parts.planes();
        assert_eq!(parts.planes_shown().to_vec(), whole.planes_shown().to_vec(), "a continuation in rectangles equals one taken whole");
        assert_eq!((parts.unknown.clone(), parts.coverage().cloned()), (whole.unknown.clone(), whole.coverage().cloned()));
        assert_eq!((parts.hatch_writes(), whole.hatch_writes()), ([7..8].as_slice(), [7..8].as_slice()), "the cell that turned is owed, in both");
        (parts.clear_hatch(), whole.clear_hatch());
        assert!(parts.target == whole.target && parts.mismatched == whole.mismatched);

        let third = Arc::new(SnowMud { texels: second.texels.edited((0..4).map(|channel| ((12 * width as usize + 9) * 4 + channel, 77))), ..SnowMud::clone(&second) });
        let before = parts.planes_shown().to_vec();
        parts.begin_parts(&third, Continue).unwrap();
        let rect: Vec<u8> = (8..16).flat_map(|row| third.texels.to_vec()[(row * width as usize + 9) * 4..(row * width as usize + 10) * 4].to_vec()).collect();
        assert_eq!(parts.take_rect(&third, 8..16, 9..10).unwrap(), rect, "the rectangle's bytes alone");
        parts.finish_parts(&third, Continue);
        let after = parts.planes_shown().to_vec();
        let moved: Vec<usize> = (0..before.len()).filter(|&at| before[at] != after[at]).collect();
        assert_eq!(moved, ((12 * width as usize + 9) * 4..(12 * width as usize + 9) * 4 + 4).collect::<Vec<_>>());
        whole.set_shared(&third, Continue).unwrap();
        assert_eq!(parts.planes_shown().to_vec(), whole.planes_shown().to_vec());

        let mut last = third;
        for (row, column, cell) in [(27, 3, [4, 3, 3, 4]), (2, 5, [6, 6, 6, 6]), (27, 3, [9, 8, 8, 9])] {
            let next = Arc::new(SnowMud { texels: last.texels.edited((0..4).map(|channel| ((row * width as usize + column) * 4 + channel, cell[channel]))), ..SnowMud::clone(&last) });
            whole.set_shared(&next, Continue).unwrap();
            assert!(parts.takes_parts(&next, Continue), "a grid after one whose planes differ is taken in rectangles");
            parts.begin_parts(&next, Continue).unwrap();
            let band = row as u32 / 8 * 8;
            parts.take_rect(&next, band..(band + 8).min(height), column as u32..column as u32 + 1).unwrap();
            parts.finish_parts(&next, Continue);
            parts.planes();
            assert_eq!(parts.planes_shown().to_vec(), whole.planes_shown().to_vec(), "cell {row}/{column}");
            assert!(parts.mismatched.is_empty() && whole.mismatched.is_empty(), "cell {row}/{column}: an install at once leaves no page whose planes differ");
            last = next;
        }
        assert!(parts.mismatched.is_empty());
        let third = last;

        let refreshed = Arc::new(SnowMud { refresh: Some(Refresh { interval: 25_000, identity: 1 }), ..SnowMud::clone(&third) });
        assert!(!parts.takes_parts(&refreshed, Continue) && parts.takes_parts(&refreshed, Reset));
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
            player.planes_shown(),
            [20, 10, 10, 20],
            "hour application only queues work"
        );
        assert!(player.advance(1.125, 0.125).unwrap());
        assert_eq!(player.planes_shown(), [20, 90, 10, 100]);
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
            player.planes_shown(),
            [100, 130, 90, 140],
            "copy raw current; no blended baseline"
        );
        assert_eq!(player.fade(), 0.125);
        player.advance(1.625, 0.25).unwrap();
        assert_eq!(player.fade(), 0.375);
        player.advance(2.5, 0.5).unwrap();
        assert_eq!(player.fade(), 0.875, "admitted delta caps a stalled frame");
        assert!(!player.set_shared(&snapshot(130, 140, 4), Continue).unwrap());
        assert!(!player.advance(2.625, 0.125).unwrap());
        assert_eq!(player.fade(), 1.0, "unchanged hour does not restart");
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
            player.set_shared(&snapshot(90, 100, 2), Continue).unwrap();
            let mode = continuity(previous, next, sought, &[]);
            assert_eq!(mode, Reset);
            assert!(player.set_shared(&snapshot(30, 40, 3), mode).unwrap());
            assert_eq!(player.planes_shown(), [40, 30, 30, 40]);
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
            assert_eq!(player.planes_shown(), [20, 10, 10, 20]);
            assert!(!player.advance(base, 0.5).unwrap());
            assert_eq!(player.fade().to_bits(), 0.0f32.to_bits());
            assert!(!player.set_shared(&snapshot(10, 20, 1), Continue).unwrap());
            let next = snapshot(90, 100, 2);
            assert!(!player.set_shared(&next, Continue).unwrap());
            assert!(Arc::ptr_eq(player.shared_pending.as_ref().unwrap(), &next));
            assert!(!player.advance(base + 0.125, 0.125).unwrap());
            assert_eq!(player.planes_shown(), [20, 10, 10, 20]);
            assert_eq!(player.fade(), 0.125);
            assert!(!player.set_shared(&snapshot(110, 120, 3), Continue).unwrap());
            assert!(!player.set_shared(&snapshot(130, 140, 4), Continue).unwrap());
            assert!(player.advance(base + 0.25, 0.125).unwrap());
            assert_eq!(player.planes_shown(), [20, 130, 10, 140]);
            assert_eq!(player.fade(), 0.125);
            assert!(!player.advance(base + 0.375, 0.125).unwrap());
            assert_eq!(player.fade(), 0.25);
            assert!(!player.set_shared(&snapshot(130, 140, 5), Continue).unwrap());
            assert!(player.advance(base + 0.5, 0.125).unwrap());
            assert_eq!(player.planes_shown(), [140, 130, 130, 140]);
            assert_eq!(player.fade(), 0.125);
            let mut immediate = snapshot(200, 201, 6);
            Arc::get_mut(&mut immediate).unwrap().refresh = None;
            assert!(player.set_shared(&immediate, Continue).unwrap());
            assert_eq!(player.planes_shown(), [201, 200, 200, 201]);
            assert!(!player.advance(base + 0.625, 0.125).unwrap());
            assert_eq!(player.fade(), 0.125);
            let mut resized = snapshot(30, 40, 7);
            let grid = Arc::get_mut(&mut resized).unwrap();
            grid.width = 2;
            grid.texels.extend_from_within(..);
            assert!(player.set_shared(&resized, Continue).unwrap());
            assert_eq!(player.size, [2, 1]);
            assert_eq!(player.planes_shown(), [40, 30, 30, 40, 40, 30, 30, 40]);
            assert_eq!(player.pending_texels(), resized.texels);
            assert!(!player.advance(base + 0.75, 0.5).unwrap());
            assert_eq!(player.fade().to_bits(), 0.0f32.to_bits());
        }
    }

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
            player.shown.clear();
            player.shown.resize(snow.texels.pages().len(), true);
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
            let texels: Vec<u8> = (0..width)
                .flat_map(|cell| {
                    let snow = bytes.wrapping_add(cell as u8);
                    let mud = snow.wrapping_mul(3);
                    [mud, snow, snow, mud]
                })
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
                let [old, new] = &mut arms;
                assert_eq!(old.size, new.size);
                let (reference, made) = (old.planes_shown().to_vec(), new.planes_shown().to_vec());
                let differing: Vec<usize> = (0..reference.len().min(made.len())).filter(|&at| reference[at] != made[at]).collect();
                assert!(
                    reference.len() == made.len() && differing.is_empty(),
                    "hour {hour} frame {frame}: {} planes against {}, differing at {:?}",
                    reference.len(),
                    made.len(),
                    &differing[..differing.len().min(8)]
                );
                let [old, new] = &arms;
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
        assert_eq!(player.planes_shown(), [20, 90, 10, 100]);
        assert_eq!(
            player.fade(),
            0.5,
            "refresh resets then adds admitted f32 delta"
        );
        let snow = f32::from(player.planes_shown()[2]) * (1.0 - player.fade())
            + f32::from(player.planes_shown()[1]) * player.fade();
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
            player.planes_shown(),
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
