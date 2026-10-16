//! Filtered graphics-frame delta for this scene. Initialize 32 f32 samples and the cursor to zero.
//! Replace the cursor sample, advance modulo 32 and sort the retained storage in place. Average
//! indices 3 through 28, divide by 26, blend equally with the current raw frame time and clamp to
//! zero through 0.5. Sorted samples, including startup zeros, persist into the next frame.

#[derive(Default)]
pub struct RenderDelta {
    samples: [f32; 32],
    next: usize,
    last_time: Option<f32>,
}

impl RenderDelta {
    pub fn advance(&mut self, now: f32) -> f32 {
        let previous = self.last_time.replace(now);
        let Some(previous) = previous else {
            return 0.0;
        };
        if now < previous {
            // An explicitly restarted presentation clock has a new epoch.
            self.samples.fill(0.0);
            self.next = 0;
            return 0.0;
        }
        if now == previous {
            // Export/allocation retries at one frozen timestamp do not admit
            // another graphics frame or consume the smoothing history.
            return 0.0;
        }
        self.admit(now - previous)
    }

    fn admit(&mut self, frame_time: f32) -> f32 {
        self.samples[self.next] = frame_time;
        self.next = (self.next + 1) % self.samples.len();
        self.samples.sort_unstable_by(f32::total_cmp);
        let mut sum = 0.0_f32;
        for &sample in &self.samples[3..29] {
            sum += sample;
        }
        ((sum / 26.0) * 0.5 + frame_time * 0.5).clamp(0.0, 0.5)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weather_render_delta_trims_extremes_and_weights_the_current_frame() {
        let mut delta = RenderDelta {
            samples: std::array::from_fn(|index| match index {
                0..=2 => 0.0,
                3..=28 => 0.125,
                _ => 8.0,
            }),
            next: 31,
            last_time: Some(1.0),
        };
        assert_eq!(
            delta.advance(1.25),
            0.1875,
            "middle 26 stay 0.125; current 0.25 contributes its separate half"
        );
        assert_eq!(delta.next, 0, "native cursor wraps after slot 31");
        assert_eq!(&delta.samples[29..], &[0.25, 8.0, 8.0]);
        assert_eq!(
            delta.advance(1.375),
            0.125,
            "cursor now overwrites the first slot in the sorted storage"
        );
    }

    #[test]
    fn weather_render_delta_keeps_native_startup_zeros_and_irregular_frames() {
        let mut delta = RenderDelta::default();
        assert_eq!(delta.advance(0.0), 0.0);
        assert_eq!(delta.advance(0.125), 0.0625);
        assert_eq!(delta.advance(0.25), 0.0625);
        assert_eq!(delta.advance(0.5), 0.125);
        assert_eq!(
            delta.advance(0.75),
            f32::from_bits(0x3e02_7627),
            "fourth sample reaches the trimmed mean; raw delta alone is 0.25"
        );
        let samples = delta.samples;
        let next = delta.next;
        assert_eq!(delta.advance(0.75), 0.0, "frozen export clock");
        assert_eq!(delta.samples, samples);
        assert_eq!(delta.next, next);
        assert_eq!(delta.advance(0.0), 0.0, "clock restart clears its history");
        assert_eq!(delta.samples, [0.0; 32]);
        assert_eq!(delta.next, 0);
        assert_eq!(delta.advance(0.125), 0.0625);
    }

    #[test]
    fn weather_render_delta_overwrites_sorted_history_on_each_frame() {
        let mut delta = RenderDelta {
            samples: std::array::from_fn(|index| index as f32 / 64.0),
            next: 0,
            last_time: Some(0.0),
        };
        assert_eq!(delta.advance(0.5), 0.37890625);
        assert_eq!(delta.advance(1.0), 0.38671875);
        assert_eq!(
            delta.samples[0],
            1.0 / 64.0,
            "second frame replaces sorted slot 1 (2/64), retaining 1/64"
        );
        assert_eq!(delta.samples[1], 3.0 / 64.0);
        // Sorting only a temporary copy would instead overwrite 1/64, retain
        // 2/64, and produce a different subsequent trimmed mean.
    }

    #[test]
    fn weather_render_delta_caps_stalls_after_weighting() {
        let mut delta = RenderDelta {
            samples: [0.125; 32],
            next: 0,
            last_time: Some(1.0),
        };
        assert_eq!(delta.advance(3.0), 0.5);
        assert_eq!(delta.samples[31], 2.0, "store raw stall before clamping");
        assert_eq!(delta.advance(3.125), 0.125, "outlier is trimmed next frame");
        // The producer clamps negative supplied frame time at zero too.
        let mut negative = RenderDelta::default();
        assert_eq!(negative.admit(-1.0), 0.0);
    }
}
