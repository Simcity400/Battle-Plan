//! Viewer-local native snow/mud graphics refresh, independent of replay hours.

use super::{Refresh, SnowMud};
use map_display_data::{Paged, admission::Accounting, credit::display_products};
use std::{io, ops::Range, sync::Arc};
use viewer_foundation::map::presentation::Continuity;

#[derive(Default)]
pub struct Presentation {
    pub size: [u32; 2],
    pub texels: Vec<u8>,
    shared_pending: Option<Arc<SnowMud>>,
    target: Paged<u8>,
    mismatched: Vec<usize>,
    uploads: Vec<Range<usize>>,
    allocation: Option<Accounting>,
    dirty: bool,
    refresh: Option<Refresh>,
    last_time: Option<f32>,
    last_refresh: f64,
    fade: f32,
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
    map_display_data::retained::Retained::try_build_in(
        display_products(),
        count
            .checked_mul(size_of::<usize>())
            .ok_or_else(|| io::Error::from(io::ErrorKind::OutOfMemory))?,
        || {
            let mut dirty = Vec::new();
            dirty
                .try_reserve_exact(count)
                .map_err(|_| io::Error::from(io::ErrorKind::OutOfMemory))?;
            dirty.extend(pending.changed_page_indices(target));
            dirty.extend_from_slice(mismatched);
            dirty.sort_unstable();
            dirty.dedup();
            Ok(dirty)
        },
    )
}

impl Presentation {
    fn reserve(&mut self, snow: &SnowMud) -> io::Result<()> {
        if self.allocation.is_none() {
            self.allocation = Some(Accounting::new(
                display_products(),
                size_of::<Accounting>(),
            )?);
        }
        let allocation = self.allocation.as_mut().unwrap();
        let additional = snow.texels.len().saturating_sub(self.texels.len());
        allocation.reserve(&mut self.texels, additional)?;
        let pages = snow.texels.pages().len();
        let additional = pages.saturating_sub(self.mismatched.len());
        allocation.reserve(&mut self.mismatched, additional)?;
        let additional = pages.max(1).saturating_sub(self.uploads.len());
        allocation.reserve(&mut self.uploads, additional)?;
        Ok(())
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
                    .is_some_and(|had| had.texels == snow.texels));
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
                self.texels.extend(snow.texels.iter().copied());
                self.uploads.clear();
                self.uploads.push(0..self.texels.len());
                for (page, bytes) in snow.texels.pages().enumerate() {
                    if bytes
                        .chunks_exact(4)
                        .any(|cell| cell[0] != cell[3] || cell[2] != cell[1])
                    {
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
                    if bytes
                        .chunks_exact(4)
                        .any(|cell| cell[0] != cell[3] || cell[2] != cell[1])
                    {
                        self.mismatched.push(page);
                    }
                }
            }
            self.target = snow.texels.clone();
            self.dirty = false;
            self.fade = 0.0;
            if reset {
                self.last_time = None;
                self.last_refresh = 0.0;
            }
            return Ok(true);
        }
        self.dirty = true;
        Ok(false)
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
            self.dirty = false;
            self.fade = 0.0;
        }
        self.fade = (self.fade + dt).clamp(0.0, 1.0);
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
