//! The on-screen TIME SCRUBBER state machine (Stage 1 deliverable 5).
//!
//! PURE logic — no `Timeline`, no GPU, no window. It owns only:
//!   * the geometry of the draggable track (a pixel rect on screen),
//!   * the cursor position into the series (an index `0..len`),
//!   * play/pause + a fixed-step auto-advance ACCUMULATOR.
//!
//! It converts raw input (a drag x, a play toggle, an elapsed `dt`) into a
//! [`ScrubIntent`] — "seek to index N" / "step forward" / "nothing" — which the app
//! applies to the real [`scrub::Timeline`]. Keeping the math here (a) makes the
//! whole transport headlessly testable without a GPU/window, and (b) keeps the
//! Timeline the single source of truth for which hours exist (the scrubber never
//! fabricates an hour — it only ever produces an INDEX the Timeline clamps).
//!
//! The Timeline already clamps seeks and pauses at the ends; this layer mirrors the
//! clamp so the on-screen thumb position is correct even before the seek resolves.

/// What the scrubber wants the app to do this update. The app maps a `Seek`/`Step`
/// onto `Timeline::seek_to` / `Timeline::step`; `None` means no transport change
/// (e.g. playing but not enough time elapsed for the next step).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrubIntent {
    /// Jump the cursor to this series index (a drag/click on the track).
    Seek(usize),
    /// Advance one snapshot forward (an auto-advance tick fired).
    StepForward,
    /// No transport change this update.
    None,
}

/// The scrubber widget's pixel geometry on screen (the draggable track).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl TrackRect {
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
}

/// The scrubber state machine.
#[derive(Debug, Clone)]
pub struct Scrubber {
    /// Number of snapshots in the series (the index space is `0..len`).
    len: usize,
    /// Current cursor index into the series (kept in sync with the Timeline).
    pos: usize,
    /// Whether auto-advance is on.
    playing: bool,
    /// Seconds between auto-advance steps (transport speed).
    step_secs: f32,
    /// Accumulated play time since the last step.
    accum: f32,
    /// The on-screen track geometry (updated on resize/layout).
    track: TrackRect,
    /// True while the user is actively dragging the thumb (mouse down on the track).
    dragging: bool,
}

impl Scrubber {
    /// Build a scrubber for a `len`-entry series. `step_secs` is the auto-advance
    /// interval (e.g. 0.5s/step). The track geometry starts zero-sized; the app
    /// calls [`set_track`](Self::set_track) on layout/resize.
    pub fn new(len: usize, step_secs: f32) -> Scrubber {
        Scrubber {
            len,
            pos: 0,
            playing: false,
            step_secs: step_secs.max(0.0001),
            accum: 0.0,
            track: TrackRect {
                x: 0.0,
                y: 0.0,
                w: 0.0,
                h: 0.0,
            },
            dragging: false,
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub fn pos(&self) -> usize {
        self.pos
    }
    pub fn is_playing(&self) -> bool {
        self.playing
    }
    pub fn is_dragging(&self) -> bool {
        self.dragging
    }
    pub fn track(&self) -> TrackRect {
        self.track
    }

    /// Update the on-screen track rect (app layout / window resize).
    pub fn set_track(&mut self, track: TrackRect) {
        self.track = track;
    }

    /// Keep the scrubber's cursor in sync with the Timeline after a non-scrubber
    /// move (e.g. a province pick doesn't move time, but an external seek might).
    pub fn sync_pos(&mut self, pos: usize) {
        self.pos = clamp_index(pos, self.len);
    }

    /// Toggle play/pause. Starting to play resets the accumulator so the first step
    /// lands a full interval later (no immediate jump).
    pub fn toggle_play(&mut self) {
        self.playing = !self.playing;
        self.accum = 0.0;
    }
    pub fn play(&mut self) {
        if !self.playing {
            self.playing = true;
            self.accum = 0.0;
        }
    }
    pub fn pause(&mut self) {
        self.playing = false;
    }

    /// Convert an absolute cursor x (pixels) to a series index over the track.
    /// Clamped to `[0, len-1]`. The thumb spans the track; x maps linearly. Returns
    /// `None` for an empty series (no index space).
    pub fn index_at_x(&self, px: f32) -> Option<usize> {
        if self.len == 0 || self.track.w <= 0.0 {
            return None;
        }
        let t = ((px - self.track.x) / self.track.w).clamp(0.0, 1.0);
        // map t in [0,1] across len discrete stops: round to the nearest index.
        let idx = (t * (self.len.saturating_sub(1)) as f32).round() as usize;
        Some(clamp_index(idx, self.len))
    }

    /// The thumb's centre x for the current cursor (for drawing). `None` if empty.
    pub fn thumb_x(&self) -> Option<f32> {
        if self.len == 0 || self.track.w <= 0.0 {
            return None;
        }
        let denom = self.len.saturating_sub(1).max(1) as f32;
        let t = self.pos as f32 / denom;
        Some(self.track.x + t * self.track.w)
    }

    /// Begin a drag if `(px,py)` is on the track. Returns the seek intent for the
    /// initial click position (HOI4-style: click-anywhere-on-track jumps there).
    pub fn press(&mut self, px: f32, py: f32) -> ScrubIntent {
        if self.track.contains(px, py) {
            self.dragging = true;
            // pressing the scrubber pauses auto-advance (the user took manual control).
            self.playing = false;
            if let Some(idx) = self.index_at_x(px) {
                self.pos = idx;
                return ScrubIntent::Seek(idx);
            }
        }
        ScrubIntent::None
    }

    /// A mouse-move while dragging → a seek to the index under the cursor. Returns
    /// `None` when not dragging or the index didn't change (avoids redundant rebinds).
    pub fn drag_to(&mut self, px: f32) -> ScrubIntent {
        if !self.dragging {
            return ScrubIntent::None;
        }
        match self.index_at_x(px) {
            Some(idx) if idx != self.pos => {
                self.pos = idx;
                ScrubIntent::Seek(idx)
            }
            _ => ScrubIntent::None,
        }
    }

    /// End a drag (mouse up). No intent — the position is already where the last
    /// `drag_to` left it.
    pub fn release(&mut self) {
        self.dragging = false;
    }

    /// Advance the auto-play accumulator by `dt` seconds. Emits at most ONE
    /// `StepForward` per call when a full `step_secs` has elapsed; auto-pauses at the
    /// end of the series (mirroring `Timeline::tick`). The app applies the step to the
    /// Timeline and calls [`sync_pos`](Self::sync_pos) with the result.
    pub fn tick(&mut self, dt: f32) -> ScrubIntent {
        if !self.playing || self.len == 0 {
            return ScrubIntent::None;
        }
        // already at the end → pause, no step (no wrap).
        if self.pos + 1 >= self.len {
            self.playing = false;
            return ScrubIntent::None;
        }
        self.accum += dt.max(0.0);
        if self.accum >= self.step_secs {
            self.accum -= self.step_secs;
            self.pos += 1;
            // pausing exactly at the end is handled on the NEXT tick.
            return ScrubIntent::StepForward;
        }
        ScrubIntent::None
    }
}

/// Clamp an index into `0..len` (or 0 for an empty series).
fn clamp_index(idx: usize, len: usize) -> usize {
    if len == 0 {
        0
    } else {
        idx.min(len - 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track() -> TrackRect {
        TrackRect {
            x: 100.0,
            y: 900.0,
            w: 400.0,
            h: 20.0,
        }
    }

    #[test]
    fn index_at_x_maps_track_ends_and_clamps() {
        let mut s = Scrubber::new(5, 0.5);
        s.set_track(track());
        // left edge -> 0, right edge -> last, beyond -> clamped.
        assert_eq!(s.index_at_x(100.0), Some(0));
        assert_eq!(s.index_at_x(500.0), Some(4));
        assert_eq!(s.index_at_x(50.0), Some(0)); // left of track clamps to 0
        assert_eq!(s.index_at_x(9999.0), Some(4)); // right of track clamps to last
                                                   // mid maps to the nearest stop (t=0.5 over 5 stops -> idx 2).
        assert_eq!(s.index_at_x(300.0), Some(2));
    }

    #[test]
    fn empty_series_has_no_index_or_thumb() {
        let mut s = Scrubber::new(0, 0.5);
        s.set_track(track());
        assert!(s.is_empty());
        assert_eq!(s.index_at_x(300.0), None);
        assert_eq!(s.thumb_x(), None);
        assert_eq!(s.tick(10.0), ScrubIntent::None);
    }

    #[test]
    fn press_on_track_seeks_and_pauses_play() {
        let mut s = Scrubber::new(10, 0.5);
        s.set_track(track());
        s.play();
        assert!(s.is_playing());
        // press near the right end (just inside the exclusive right edge) -> seek to
        // the last index, and play stops.
        let intent = s.press(499.0, 905.0);
        assert_eq!(intent, ScrubIntent::Seek(9));
        assert_eq!(s.pos(), 9);
        assert!(
            !s.is_playing(),
            "pressing the scrubber takes manual control"
        );
        assert!(s.is_dragging());
    }

    #[test]
    fn press_off_track_is_noop() {
        let mut s = Scrubber::new(10, 0.5);
        s.set_track(track());
        // y above the track -> not on it.
        assert_eq!(s.press(300.0, 10.0), ScrubIntent::None);
        assert!(!s.is_dragging());
    }

    #[test]
    fn drag_emits_only_on_index_change() {
        let mut s = Scrubber::new(5, 0.5);
        s.set_track(track());
        s.press(100.0, 905.0); // start at idx 0, dragging
        assert!(s.is_dragging());
        // tiny move that stays on idx 0 -> no intent.
        assert_eq!(s.drag_to(105.0), ScrubIntent::None);
        // move to the far end -> seek to 4.
        assert_eq!(s.drag_to(500.0), ScrubIntent::Seek(4));
        assert_eq!(s.pos(), 4);
        // staying at 4 -> no repeat intent.
        assert_eq!(s.drag_to(498.0), ScrubIntent::None);
        s.release();
        assert!(!s.is_dragging());
        // after release, a move emits nothing.
        assert_eq!(s.drag_to(100.0), ScrubIntent::None);
    }

    #[test]
    fn tick_accumulator_steps_once_per_interval_and_pauses_at_end() {
        let mut s = Scrubber::new(3, 0.5);
        s.set_track(track());
        s.play();
        // not enough time -> no step.
        assert_eq!(s.tick(0.2), ScrubIntent::None);
        assert_eq!(s.pos(), 0);
        // crossing 0.5 total -> one step (pos 0->1).
        assert_eq!(s.tick(0.4), ScrubIntent::StepForward);
        assert_eq!(s.pos(), 1);
        // a big dt still emits only ONE step per call (no multi-advance burst).
        assert_eq!(s.tick(5.0), ScrubIntent::StepForward);
        assert_eq!(s.pos(), 2);
        // at the end now -> next tick pauses, emits nothing.
        assert_eq!(s.tick(5.0), ScrubIntent::None);
        assert!(
            !s.is_playing(),
            "auto-play pauses at the end of the series (no wrap)"
        );
    }

    #[test]
    fn thumb_x_tracks_position() {
        let mut s = Scrubber::new(5, 0.5);
        s.set_track(track());
        s.sync_pos(0);
        assert_eq!(s.thumb_x(), Some(100.0));
        s.sync_pos(4);
        assert_eq!(s.thumb_x(), Some(500.0));
        s.sync_pos(2);
        assert_eq!(s.thumb_x(), Some(300.0));
    }
}
