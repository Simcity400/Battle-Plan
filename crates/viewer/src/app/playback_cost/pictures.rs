
use crate::screen_count::{COUNTS, Compare, PARTS, TILES};
use eframe::wgpu;
use serde_json::{Value, json};
use std::{io, time::Instant};
use viewer_gpu::gpu_contract::Targets;

#[derive(Clone, Copy)]
pub(crate) struct Counting {
    pub frames: u64,
    pub every: u64,
}

impl Counting {
    pub(crate) fn parse(value: &str) -> io::Result<Self> {
        value
            .split_once(':')
            .and_then(|(frames, every)| {
                Some(Self {
                    frames: frames.trim().parse().ok()?,
                    every: every.trim().parse().ok()?,
                })
            })
            .filter(|counting| counting.frames > 0 && counting.every >= counting.frames)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "--count-pictures needs <frames counted>:<frames between the starts of two windows>, the first no larger than the second",
                )
            })
    }

    pub(crate) fn counts(self, index: u64) -> bool {
        index % self.every < self.frames
    }
}

const LIMIT: usize = 16_000;

pub(crate) struct Pictures {
    counting: Counting,
    compare: Compare,
    counted: Vec<u64>,
    skipped: u64,
    cost_us: f64,
}

impl Pictures {
    pub(crate) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        targets: &Targets,
        counting: Counting,
    ) -> io::Result<Self> {
        Ok(Self {
            counting,
            compare: Compare::new(device, queue, targets, LIMIT)?,
            counted: Vec::new(),
            skipped: 0,
            cost_us: 0.0,
        })
    }

    pub(crate) fn count(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        targets: &Targets,
        index: u64,
    ) {
        if self.counted.len() >= LIMIT {
            self.skipped += 1;
            return;
        }
        let started = Instant::now();
        if self
            .compare
            .take(device, queue, targets, index, self.counted.len())
        {
            self.counted.push(index);
        }
        self.cost_us += started.elapsed().as_secs_f64() * 1e6;
    }

    pub(crate) fn finish(mut self, device: &wgpu::Device, queue: &wgpu::Queue) -> io::Result<Value> {
        let started = Instant::now();
        let taken = self.compare.read(device, queue, self.counted.len())?;
        let mut frames = Vec::new();
        for (slot, index) in self.counted.iter().copied().enumerate() {
            let mut parts = serde_json::Map::new();
            for (part, name) in PARTS.iter().enumerate() {
                parts.insert((*name).to_owned(), part_record(&taken[slot * PARTS.len() + part]));
            }
            frames.push(json!({"frame": index, "parts": Value::Object(parts)}));
        }
        self.cost_us += started.elapsed().as_secs_f64() * 1e6;
        let [world, screen] = self.compare.sizes();
        Ok(json!({
            "window_frames": self.counting.frames,
            "window_every": self.counting.every,
            "counted_frames": self.counted.len(),
            "skipped_frames": self.skipped,
            "thread_us": self.cost_us,
            "sizes": {"world": world, "screen": screen},
            "frames": frames,
            "scope": "Each counted frame's picture against the picture of the frame counted before it, texel by texel on the GPU after the frame was timed and drained. world is the resolved float scene (the map's layers, the actors, the weather and the water, before the grade); screen is the 8-bit output the window presents, which is the graded world with the overlay's plates, icons, labels and text over it. differing is texels differing at all; steps_1, steps_2, steps_8 and steps_32 those differing by at least that many steps of 1/255 in any channel (the world's are steps of 1/255 of linear radiance, which the grade does not scale one for one); largest is the largest such difference, sum the sum over the texels, and rectangle the bounds of the differing texels ([x0, y0, x1, y1], null when none differ); tiles is how many of a 16 by 8 grid over the picture hold a texel differing by two steps or more, so a change in a few places is told from one everywhere. The first frame of every window has no frame before it and is not counted. A frame counted costs the window thread thread_us over the run and is charged to the harness, never to the frame's work.",
        }))
    }
}

fn part_record(counts: &[u32; COUNTS]) -> Value {
    let rectangle = (counts[0] > 0).then(|| json!([counts[7], counts[8], counts[9], counts[10]]));
    let tiles = counts[11..].iter().filter(|count| **count > 0).count();
    json!({
        "differing": counts[0],
        "steps_1": counts[1],
        "steps_2": counts[2],
        "steps_8": counts[3],
        "steps_32": counts[4],
        "largest": counts[5],
        "sum": counts[6],
        "rectangle": rectangle,
        "tiles": tiles,
        "tiles_of": TILES,
    })
}

#[cfg(test)]
mod tests {
    use super::Counting;

    #[test]
    fn a_counting_window_is_frames_counted_in_frames_between_windows() {
        let counting = Counting::parse("240:1200").expect("a window of 240 in every 1200");
        assert!(counting.counts(0) && counting.counts(239));
        assert!(!counting.counts(240) && !counting.counts(1199));
        assert!(counting.counts(1200));
        assert!(Counting::parse("1200:240").is_err(), "a window larger than its period");
        assert!(Counting::parse("0:240").is_err(), "a window of no frames");
        assert!(Counting::parse("240").is_err());
    }
}
