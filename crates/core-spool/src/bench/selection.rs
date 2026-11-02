use core_spool::Config;

#[derive(Clone, Copy, Default, Debug)]
pub struct Range {
    pub mean: f64,
    pub min: f64,
    pub max: f64,
}
impl Range {
    pub fn plus(self, other: Self) -> Self {
        Self {
            mean: self.mean + other.mean,
            min: self.min + other.min,
            max: self.max + other.max,
        }
    }
    pub fn divided(self, n: f64) -> Self {
        Self {
            mean: self.mean / n,
            min: self.min / n,
            max: self.max / n,
        }
    }
    pub fn difference(self, other: Self) -> Self {
        Self {
            mean: self.mean - other.mean,
            min: self.min - other.max,
            max: self.max - other.min,
        }
    }
}
#[derive(Default)]
pub struct Samples {
    count: u64,
    sum: f64,
    min: f64,
    max: f64,
}
impl Samples {
    pub fn push(&mut self, value: f64) {
        if self.count == 0 {
            self.min = value;
            self.max = value;
        }
        self.count += 1;
        self.sum += value;
        self.min = self.min.min(value);
        self.max = self.max.max(value);
    }
    pub fn range(&self) -> Range {
        assert!(self.count != 0);
        Range {
            mean: self.sum / self.count as f64,
            min: self.min,
            max: self.max,
        }
    }
}
pub struct Candidate {
    pub config: Config,
    pub serial: Range,
    pub writer: Range,
    pub bytes: u64,
    pub passes_budgets: bool,
}
impl Candidate {
    pub fn cpu(&self) -> Range {
        self.serial.plus(self.writer)
    }
}
pub fn passes(serial: Range, writer: Range, writer_budget: f64) -> bool {
    serial.max <= 29.7
        && writer.min >= 0.0
        && writer.max <= writer_budget
        && serial.plus(writer).max <= 1860.0
}
pub fn choose(candidates: &[Candidate]) -> Option<(usize, usize)> {
    let best = candidates
        .iter()
        .enumerate()
        .filter(|(_, c)| c.passes_budgets)
        .min_by(|(_, a), (_, b)| a.cpu().mean.total_cmp(&b.cpu().mean))?
        .0;
    let mut tied = 0;
    let mut choice = best;
    for (i, candidate) in candidates.iter().enumerate() {
        if candidate.passes_budgets && candidate.cpu().min <= candidates[best].cpu().max {
            tied += 1;
            if candidate.bytes < candidates[choice].bytes {
                choice = i;
            }
        }
    }
    Some((choice, tied))
}
