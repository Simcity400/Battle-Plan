//! The histogram of recorded hours' game-thread block, over the same hours as callback_work.

const HISTOGRAM_FLOOR_MS: [u64; 12] = [0, 2, 4, 8, 16, 32, 50, 64, 100, 128, 256, 1000];

#[derive(Default)]
pub(crate) struct Recorder {
    histogram: [u64; HISTOGRAM_FLOOR_MS.len()],
}

impl Recorder {
    pub(crate) fn record(&mut self, callback_ns: u64) {
        let bucket =
            HISTOGRAM_FLOOR_MS.partition_point(|&floor| callback_ns >= floor * 1_000_000) - 1;
        self.histogram[bucket] = self.histogram[bucket].saturating_add(1);
    }

    pub(crate) fn status(&self) -> impl serde::Serialize + '_ {
        crate::status::status!({ "histogram": self.histogram.as_slice() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_hour_lands_in_the_bucket_of_its_floor() {
        let mut recorder = Recorder::default();
        for ms in [0, 1, 2, 49, 50, 999, 1000, 5000] {
            recorder.record(ms * 1_000_000);
        }
        let status = serde_json::to_value(recorder.status()).unwrap();
        assert_eq!(
            status["histogram"],
            serde_json::json!([2, 1, 0, 0, 0, 1, 1, 0, 0, 0, 1, 2])
        );
    }
}
