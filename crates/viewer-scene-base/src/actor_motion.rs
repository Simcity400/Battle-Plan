
use std::collections::BTreeMap;
use viewer_gpu::camera::Camera;

pub struct Shown {
    pub key: (usize, u32, u32),
    pub at: [f32; 3],
    pub samples: u64,
}

struct Seen {
    frame: u64,
    at: [f32; 3],
    samples: u64,
    screen: Option<[f32; 2]>,
    moved: u64,
}

const MOVING: u64 = 240;

#[derive(Default)]
pub struct Motion {
    seen: BTreeMap<(&'static str, (usize, u32, u32)), Seen>,
    frame: u64,
    still: u64,
    changed: u64,
    movers: u64,
    movers_still: u64,
}

impl Motion {
    pub fn begin(&mut self) {
        self.frame += 1;
        self.changed = 0;
        self.movers = 0;
        self.movers_still = 0;
    }

    pub fn count(
        &mut self,
        label: &'static str,
        shown: &[Shown],
        camera: &Camera,
        size: [u32; 2],
    ) {
        use viewer_runtime::view_record::add;
        let (mut moved, mut posed, mut both_still) = (0u64, 0u64, 0u64);
        let (mut movers, mut movers_still) = (0u64, 0u64);
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

    pub fn end(&mut self) {
        use viewer_runtime::view_record::add;
        self.still = if self.changed == 0 { self.still + 1 } else { 0 };
        add("actors shown", "changed", self.changed);
        add("actors shown", "frame_is_new", u64::from(self.changed > 0));
        add("actors shown", "still_run", self.still);
        add("actors shown", "movers", self.movers);
        add("actors shown", "movers_still", self.movers_still);
        add("actors shown", "frame_holds_a_mover", u64::from(self.movers_still > 0));
        let frame = self.frame;
        self.seen.retain(|_, seen| seen.frame + 240 > frame);
    }
}
