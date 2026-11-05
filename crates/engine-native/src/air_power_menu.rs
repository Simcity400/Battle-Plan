//! A strategic-air region entry's air-power breakdown, shared by its reader and the recorded row.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Contribution {
    pub country: u32,
    pub amount: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Menu {
    pub coverage: i64,
    /// Enemy AA, friendly AA, enemy missiles, friendly missiles.
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
