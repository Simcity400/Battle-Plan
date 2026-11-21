
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct Contribution {
    pub country: u32,
    pub amount: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[derive(replay_spool::Spooled)]
pub struct Menu {
    pub coverage: i64,
    pub reductions: [i64; 4],
    pub contributions: [Vec<Contribution>; 2],
}

impl Menu {
    pub fn is_zero(&self) -> bool {
        self.coverage == 0
            && self.reductions == [0; 4]
            && self.contributions.iter().all(Vec::is_empty)
    }

    pub fn allocated_bytes(&self) -> usize {
        size_of::<Self>()
            + self
                .contributions
                .iter()
                .map(|rows| rows.capacity() * size_of::<Contribution>())
                .sum::<usize>()
    }
}
