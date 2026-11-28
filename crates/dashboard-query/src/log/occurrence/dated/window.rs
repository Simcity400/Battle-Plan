use super::{Ask, DatedCursor, Holders, Index, Order, invalid};
use super::super::{Answer, Direction, PlacedGap, Shown};
use std::{cmp::Reverse, io, sync::Arc};

#[derive(Clone, Copy, Default)]
struct Block {
    shown: u64,
    live: u64,
}

#[derive(Clone, Copy)]
enum Rows {
    All,
    Live,
    History,
}

impl Rows {
    fn mask(self, block: Block) -> u64 {
        match self {
            Self::All => block.shown,
            Self::Live => block.live,
            Self::History => block.shown & !block.live,
        }
    }
}

pub struct DatedWindow<P> {
    cursor: DatedCursor<P>,
    blocks: Vec<Block>,
    direction: Direction,
    live_first: bool,
}

impl<P> Index<P> {
    pub fn dated_window(self: &Arc<Self>, holders: &Holders, ask: &Ask<'_>) -> io::Result<DatedWindow<P>> {
        let Order::Date(direction) = ask.order else { return Err(invalid("A dated window requires date order")); };
        Ok(DatedWindow {
            cursor: self.dated_cursor(holders, ask)?,
            blocks: vec![Block::default(); self.len().div_ceil(64)],
            direction,
            live_first: ask.live_first,
        })
    }
}

impl<P> DatedWindow<P> {
    pub fn advance(&mut self, hour: i32, offset: u64, limit: u32) -> io::Result<Answer> {
        if limit == 0 {
            return Err(invalid("An occurrence window holds one row or more"));
        }
        let index = Arc::clone(&self.cursor.index);
        let step = self.cursor.advance(hour);
        if step.reset {
            self.blocks.fill(Block::default());
        }
        for edit in step.changed {
            let at = edit.at as usize;
            let bit = 1 << (at % 64);
            let block = &mut self.blocks[at / 64];
            block.shown = (block.shown & !bit) | (bit * u64::from(step.rows.contains_key(&edit.at)));
            block.live = (block.live & !bit) | (bit * u64::from(step.is_live(edit.at)));
        }

        let total = step.rows.len() as u64;
        let offset = offset.min(total);
        let mut next_change = step.until;
        let mut rows = Vec::with_capacity((total - offset).min(u64::from(limit)) as usize);
        let ascending = self.direction == Direction::Ascending;
        let mut skip = offset;
        let groups: &[Rows] = if offset == total {
            &[]
        } else if !self.live_first {
            &[Rows::All]
        } else if offset >= step.live {
            skip -= step.live;
            &[Rows::History]
        } else {
            &[Rows::Live, Rows::History]
        };
        'page: for group in groups {
            for at in 0..self.blocks.len() {
                let at = if ascending { self.blocks.len() - 1 - at } else { at };
                let mut mask = group.mask(self.blocks[at]);
                let size = u64::from(mask.count_ones());
                if skip >= size {
                    skip -= size;
                    continue;
                }
                if ascending {
                    mask = mask.reverse_bits();
                }
                for _ in 0..std::mem::take(&mut skip) {
                    mask &= mask - 1;
                }
                while mask != 0 {
                    let bit = mask.trailing_zeros() as usize;
                    let at = (at * 64 + if ascending { 63 - bit } else { bit }) as u32;
                    let entry = index.get(at);
                    let after = entry.states.partition_point(|&(edge, _)| edge <= hour);
                    next_change = next_change.into_iter().chain(entry.states.get(after).map(|edge| edge.0)).min();
                    rows.push(Shown { at, ordinal: offset + rows.len() as u64, live: step.is_live(at), state: entry.state_at(hour) });
                    if rows.len() == limit as usize {
                        break 'page;
                    }
                    mask &= mask - 1;
                }
            }
        }

        let live_front = if self.live_first { step.live } else { 0 };
        let history = if self.live_first { Rows::History } else { Rows::All };
        let mut boundaries: Vec<_> = step.gaps.into_iter().map(|gap| {
            let end = index.entries.partition_point(|entry| if ascending { entry.date >= gap.first_hour } else { entry.date > gap.last_hour });
            (end, gap)
        }).collect();
        boundaries.sort_by_key(|(end, _)| *end);
        let (mut covered, mut count) = (0, 0u64);
        let mut gaps: Vec<_> = boundaries.into_iter().map(|(end, gap)| {
            let (whole, rest) = (end / 64, end % 64);
            count += self.blocks[covered..whole].iter().map(|&block| u64::from(history.mask(block).count_ones())).sum::<u64>();
            covered = whole;
            let prefix = count + self.blocks.get(whole)
                .map_or(0, |&block| u64::from((history.mask(block) & ((1 << rest) - 1)).count_ones()));
            let before = if ascending { total - prefix } else { live_front + prefix };
            (before, gap)
        }).collect();
        gaps.sort_by_key(|(before, gap)| (*before, Reverse(gap.last_hour), if ascending { i64::from(gap.first_hour) } else { -i64::from(gap.first_hour) }));
        let gaps = gaps.into_iter().map(|(before, gap)| PlacedGap {
            first_hour: gap.first_hour, last_hour: gap.last_hour, reason: gap.reason, before: before.to_string(),
        }).collect();
        Ok(Answer { hour, total, live: step.live, offset, rows, chips: step.chips.to_vec(), gaps, unattributed: step.unattributed, next_change })
    }
}
