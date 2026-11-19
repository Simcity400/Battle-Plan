//! What the actors show a watcher, frame by frame.
//!
//! Smooth playback is not a microsecond figure: it is that every display frame is a new picture
//! of the replay at the top speed. So
//! this counts, for the figures whose groups the view reaches, how many changed their root or
//! their play's samples since the frame before, by how many pixels on screen, and how many
//! frames in a row showed them all standing. A reader that splits the frames by the publication
//! they drew ([`viewer_foundation::map`]'s `layers.made`) then says which of them are placed
//! from the publication's hour and which from the frame's own clock.

use std::collections::BTreeMap;
use viewer_foundation::map::camera::Camera;

/// One actor as a frame showed it.
pub(crate) struct Shown {
    pub key: (usize, u32, u32),
    /// Its root in the world, on the map copy the view reaches.
    pub at: [f32; 3],
    /// A number that changes when its play's samples do, `0` for an actor with no play.
    pub samples: u64,
}

struct Seen {
    frame: u64,
    at: [f32; 3],
    samples: u64,
    screen: Option<[f32; 2]>,
    /// The last frame its root moved: within [`MOVING`] frames of this one, its hours are moving
    /// it, and a frame that leaves it standing is a frame the replay did not advance for it.
    moved: u64,
}

/// How many frames back a root must have moved for the count to call it a mover: one second at
/// 240 frames a second, so a figure crossing a province is a mover throughout.
const MOVING: u64 = 240;

#[derive(Default)]
pub(crate) struct Motion {
    seen: BTreeMap<(&'static str, (usize, u32, u32)), Seen>,
    frame: u64,
    /// Frames in a row, up to and including this one, in which nothing of the actors in view
    /// changed its root or its samples.
    still: u64,
    /// This frame's actors that moved or posed, over every pass.
    changed: u64,
    /// This frame's actors whose hours are moving them, and those of them it left standing.
    movers: u64,
    movers_still: u64,
}

impl Motion {
    pub(crate) fn begin(&mut self) {
        self.frame += 1;
        self.changed = 0;
        self.movers = 0;
        self.movers_still = 0;
    }

    /// Counts one pass's shown actors against the frame before.
    pub(crate) fn count(
        &mut self,
        label: &'static str,
        shown: &[Shown],
        camera: &Camera,
        size: [u32; 2],
    ) {
        use viewer_foundation::view_record::add;
        let (mut moved, mut posed, mut both_still) = (0u64, 0u64, 0u64);
        // Of the actors whose hours are moving them, how many this frame moved and how many it
        // left standing: none should stand.
        let (mut movers, mut movers_still) = (0u64, 0u64);
        // Pixels a moved actor's root travelled on screen, in thousandths, and by band: under a
        // quarter of a pixel, under one, under four, and more.
        let mut bands = [0u64; 4];
        let mut milli = 0u64;
        for actor in shown {
            let screen = camera.project_wrapped(actor.at, size);
            let before = self.seen.insert(
                (label, actor.key),
                Seen { frame: self.frame, at: actor.at, samples: actor.samples, screen, moved: 0 },
            );
            let Some(before) = before.filter(|before| before.frame + 1 == self.frame) else {
                continue;
            };
            let shifted = before.at != actor.at;
            // When its root last moved, carried from the frame before, so a figure between two of
            // its hours' own moves is still counted as one the hours are moving.
            let moved_at = if shifted { self.frame } else { before.moved };
            if let Some(held) = self.seen.get_mut(&(label, actor.key)) {
                held.moved = moved_at;
            }
            if moved_at != 0 && moved_at + MOVING > self.frame {
                movers += 1;
                movers_still += u64::from(!shifted);
            }
            if shifted {
                moved += 1;
                let pixels = match (before.screen, screen) {
                    (Some(was), Some(now)) => {
                        ((now[0] - was[0]).powi(2) + (now[1] - was[1]).powi(2)).sqrt()
                    }
                    // A root that moved while off the projection counts as moved, unmeasured.
                    _ => f32::NAN,
                };
                if pixels.is_finite() {
                    milli += (f64::from(pixels) * 1000.0).round().max(0.0) as u64;
                    bands[match pixels {
                        p if p < 0.25 => 0,
                        p if p < 1.0 => 1,
                        p if p < 4.0 => 2,
                        _ => 3,
                    }] += 1;
                }
            }
            let played = before.samples != actor.samples;
            if played {
                posed += 1;
            }
            if !shifted && !played {
                both_still += 1;
            }
        }
        self.changed += moved + posed;
        self.movers += movers;
        self.movers_still += movers_still;
        add(label, "shown_in_view", shown.len() as u64);
        add(label, "shown_movers", movers);
        add(label, "shown_movers_still", movers_still);
        add(label, "shown_moved", moved);
        add(label, "shown_posed", posed);
        add(label, "shown_still", both_still);
        add(label, "shown_moved_milli_px", milli);
        for (band, count) in ["px_under_quarter", "px_under_one", "px_under_four", "px_four_up"]
            .into_iter()
            .zip(bands)
        {
            add(label, band, count);
        }
    }

    /// Ends the frame's count: how many actors changed anything, and how many frames in a row
    /// have now shown none changing.
    pub(crate) fn end(&mut self) {
        use viewer_foundation::view_record::add;
        self.still = if self.changed == 0 { self.still + 1 } else { 0 };
        add("actors shown", "changed", self.changed);
        add("actors shown", "frame_is_new", u64::from(self.changed > 0));
        add("actors shown", "still_run", self.still);
        add("actors shown", "movers", self.movers);
        add("actors shown", "movers_still", self.movers_still);
        add("actors shown", "frame_holds_a_mover", u64::from(self.movers_still > 0));
        // An actor no frame has shown for a while is forgotten, so the table does not grow with
        // the hours played.
        let frame = self.frame;
        self.seen.retain(|_, seen| seen.frame + 240 > frame);
    }
}
