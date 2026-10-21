//! Viewer-local native snow/mud graphics refresh, independent of replay hours.

use super::{Refresh, SnowMud};
use std::sync::Arc;
use viewer_foundation::map::presentation::Continuity;

#[derive(Default)]
pub struct Presentation {
    pub size: [u32; 2],
    pub texels: Vec<u8>,
    shared_pending: Option<Arc<SnowMud>>,
    dirty: bool,
    refresh: Option<Refresh>,
    last_time: Option<f32>,
    last_refresh: f64,
    fade: f32,
}

impl Presentation {
    /// Keeps the producer's immutable grid until the graphics clock admits it.
    /// Only the pending ownership changes; refreshes still copy the actual shown planes.
    pub fn set_shared(&mut self, snow: &Arc<SnowMud>, continuity: Continuity) -> bool {
        let size = [snow.width, snow.height];
        let reset = continuity == Continuity::Reset || self.size != size || self.texels.is_empty();
        let same = self
            .shared_pending
            .as_ref()
            .is_some_and(|had| Arc::ptr_eq(had, snow))
            || self.pending_texels() == snow.texels;
        let changed = !same || self.refresh != snow.refresh;
        if !reset && !changed {
            return false;
        }
        self.size = size;
        self.shared_pending = Some(Arc::clone(snow));
        self.refresh = snow.refresh;
        if reset || self.refresh.is_none() {
            self.texels.clone_from(&snow.texels);
            self.dirty = false;
            self.fade = 0.0;
            if reset {
                self.last_time = None;
                self.last_refresh = 0.0;
            }
            return true;
        }
        self.dirty = true;
        false
    }

    fn pending_texels(&self) -> &[u8] {
        self.shared_pending
            .as_ref()
            .map(|snow| snow.texels.as_slice())
            .unwrap_or_default()
    }

    /// Refresh only with dirty work and sufficient elapsed graphics time, narrowing the fixed-five
    /// interval to f32. Copy current to previous before updates. Equality identifies refreshes even
    /// when averaged cells agree. Each successful refresh resets the fade, then independently admitted
    /// graphics deltas advance it over one second, including interrupted fades. `now` controls
    /// eligibility; `render_delta` is independent of refresh timestamps.
    pub fn advance(&mut self, now: f32, render_delta: f32) -> bool {
        // Preserve the viewer's first-frame/reset epoch: both planes were just
        // bound to one snapshot, so the new phase starts at zero on this frame.
        let dt = if self.last_time.is_some() {
            render_delta
        } else {
            0.0
        };
        if self.last_time.is_none() {
            self.last_refresh = f64::from(now);
        }
        self.last_time = Some(now);
        let refreshed = self.dirty
            && self.refresh.is_some_and(|refresh| {
                f64::from(refresh.interval as f32 / 100_000.0) <= f64::from(now) - self.last_refresh
            });
        if refreshed {
            self.last_refresh = f64::from(now);
            let pending = self
                .shared_pending
                .as_ref()
                .map(|snow| snow.texels.as_slice())
                .unwrap_or_default();
            for (cell, target) in self.texels.chunks_exact_mut(4).zip(pending.chunks_exact(4)) {
                // Logical RGBA: previous mud, current snow,
                // previous snow, current mud. Copy raw current, not its fade.
                cell[0] = cell[3];
                cell[2] = cell[1];
                cell[1] = target[1];
                cell[3] = target[3];
            }
            self.dirty = false;
            self.fade = 0.0;
        }
        self.fade = (self.fade + dt).clamp(0.0, 1.0);
        refreshed
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
            texels: vec![mud, snow, snow, mud],
            province_snow: vec![],
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
    fn snow_mud_refresh_uses_graphics_clock_and_native_interrupted_planes() {
        let mut player = Presentation::default();
        player.set_shared(&snapshot(10, 20, 1), Reset);
        assert!(!player.advance(0.0, 0.0));
        player.advance(0.5, 0.5);
        player.advance(1.0, 0.5);
        assert_eq!(player.fade(), 1.0);
        player.set_shared(&snapshot(90, 100, 2), Continue);
        assert_eq!(
            player.texels,
            [20, 10, 10, 20],
            "hour application only queues work"
        );
        assert!(player.advance(1.125, 0.125));
        assert_eq!(player.texels, [20, 90, 10, 100]);
        assert_eq!(
            player.fade(),
            0.125,
            "reset then add this frame's graphics delta"
        );
        player.set_shared(&snapshot(110, 120, 3), Continue);
        assert!(
            !player.advance(1.25, 0.125),
            "dirty work waits for the native interval"
        );
        assert_eq!(player.fade(), 0.25);
        player.set_shared(&snapshot(130, 140, 4), Continue);
        assert!(
            player.advance(1.375, 0.125),
            "lower-inclusive interval boundary"
        );
        assert_eq!(
            player.texels,
            [100, 130, 90, 140],
            "copy raw current; no blended baseline"
        );
        assert_eq!(player.fade(), 0.125);
        // Freeze the replay entirely: frames alone finish the one-second fade.
        player.advance(1.625, 0.25);
        assert_eq!(player.fade(), 0.375);
        player.advance(2.5, 0.5);
        assert_eq!(player.fade(), 0.875, "admitted delta caps a stalled frame");
        assert!(!player.set_shared(&snapshot(130, 140, 4), Continue));
        assert!(!player.advance(2.625, 0.125));
        assert_eq!(player.fade(), 1.0, "unchanged hour does not restart");
        // A full rebuild can publish equal bytes; the native identity catches it.
        player.set_shared(&snapshot(130, 140, 5), Continue);
        assert!(player.advance(2.75, 0.125));
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
            player.set_shared(&snapshot(10, 20, 1), Reset);
            player.advance(10.0, 0.0);
            player.set_shared(&snapshot(90, 100, 2), Continue); // Queued old-hour work.
            let mode = continuity(previous, next, sought, &[]);
            assert_eq!(mode, Reset);
            assert!(player.set_shared(&snapshot(30, 40, 3), mode));
            assert_eq!(player.texels, [40, 30, 30, 40]);
            assert_eq!(player.fade(), 0.0);
            assert!(
                !player.advance(300.0, 0.5),
                "old pending refresh was cancelled"
            );
            assert_eq!(player.fade(), 0.0, "reset has its own clock epoch");
            player.advance(300.5, 0.5);
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
            assert!(player.set_shared(&first, Reset));
            assert!(Arc::ptr_eq(player.shared_pending.as_ref().unwrap(), &first));
            assert_eq!(player.texels, [20, 10, 10, 20]);
            assert!(!player.advance(base, 0.5));
            assert_eq!(player.fade().to_bits(), 0.0f32.to_bits());
            assert!(!player.set_shared(&snapshot(10, 20, 1), Continue));
            let next = snapshot(90, 100, 2);
            assert!(!player.set_shared(&next, Continue));
            assert!(Arc::ptr_eq(player.shared_pending.as_ref().unwrap(), &next));
            assert!(!player.advance(base + 0.125, 0.125));
            assert_eq!(player.texels, [20, 10, 10, 20]);
            assert_eq!(player.fade(), 0.125);
            // Replacing pending skipped hours never changes the shown planes.
            assert!(!player.set_shared(&snapshot(110, 120, 3), Continue));
            assert!(!player.set_shared(&snapshot(130, 140, 4), Continue));
            assert!(player.advance(base + 0.25, 0.125));
            assert_eq!(player.texels, [20, 130, 10, 140]);
            assert_eq!(player.fade(), 0.125);
            assert!(!player.advance(base + 0.375, 0.125));
            assert_eq!(player.fade(), 0.25);
            // Equal bytes with a new refresh identity still copy actual current
            // planes, not the partially blended result.
            assert!(!player.set_shared(&snapshot(130, 140, 5), Continue));
            assert!(player.advance(base + 0.5, 0.125));
            assert_eq!(player.texels, [140, 130, 130, 140]);
            assert_eq!(player.fade(), 0.125);
            let mut immediate = snapshot(200, 201, 6);
            Arc::get_mut(&mut immediate).unwrap().refresh = None;
            assert!(player.set_shared(&immediate, Continue));
            assert_eq!(player.texels, [201, 200, 200, 201]);
            assert!(!player.advance(base + 0.625, 0.125));
            assert_eq!(player.fade(), 0.125);
            let mut resized = snapshot(30, 40, 7);
            let grid = Arc::get_mut(&mut resized).unwrap();
            grid.width = 2;
            grid.texels.extend_from_within(..);
            assert!(player.set_shared(&resized, Continue));
            assert_eq!(player.size, [2, 1]);
            assert_eq!(player.texels, [40, 30, 30, 40, 40, 30, 30, 40]);
            assert_eq!(player.pending_texels(), resized.texels);
            assert!(!player.advance(base + 0.75, 0.5));
            assert_eq!(player.fade().to_bits(), 0.0f32.to_bits());
        }
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
        player.set_shared(&snapshot(10, 20, 1), Reset);
        player.advance(32.0, delta.advance(32.0));
        player.set_shared(&snapshot(90, 100, 2), Continue);
        let admitted = delta.advance(34.0);
        assert_eq!(admitted, 0.5);
        assert!(
            player.advance(34.0, admitted),
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
        player.set_shared(&snapshot(10, 20, 1), Reset);
        player.advance(32.0, delta.advance(32.0));
        player.set_shared(&snapshot(90, 100, 2), Continue);
        assert!(player.advance(32.25, delta.advance(32.25)));
        assert_eq!(
            player.fade(),
            0.1875,
            "half trimmed mean plus half current frame"
        );
        player.set_shared(&snapshot(110, 120, 3), Continue);
        assert!(!player.advance(32.375, delta.advance(32.375)));
        assert_eq!(player.fade(), 0.3125);
        assert!(
            player.advance(32.5, delta.advance(32.5)),
            "raw clock reaches 0.25 interval"
        );
        assert_eq!(
            player.texels,
            [100, 110, 90, 120],
            "interrupted fade copies raw current planes"
        );
        assert_eq!(player.fade(), 0.125);
        assert!(!player.advance(32.5, delta.advance(32.5)));
        assert_eq!(
            player.fade(),
            0.125,
            "frozen presentation retry preserves fade"
        );
    }
}
