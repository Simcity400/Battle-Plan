use super::{Line, RankOwner};
use crate::read;
use replay_columnar::{
    envelope::{SPAN_BLOCKS, UNIT_ORDINALS},
    manifest::Status,
    summary::{self, Piece},
};
use std::{collections::HashSet, io};

#[derive(Clone, Default)]
struct Bounds {
    winners: usize,
    weakest: Option<i64>,
    challenger: Option<i64>,
    uncertain: bool,
}

impl Bounds {
    fn proves(&self, winners: usize, n: usize, least: bool) -> bool {
        if self.uncertain || self.winners != winners {
            return false;
        }
        match (self.weakest, self.challenger) {
            (_, None) => true,
            (Some(winner), Some(challenger)) if winners == n => {
                if least {
                    winner < challenger
                } else {
                    winner > challenger
                }
            }
            _ => false,
        }
    }
}

struct WinnerSet<'a> {
    slots: &'a [u32],
    n: usize,
    least: bool,
}

impl Line<'_, '_> {
    pub(super) fn summarized_winners(
        &self,
        at: i32,
        limit: i32,
        n: usize,
        least: bool,
        winners: &[u32],
        check_cancel: &dyn Fn() -> io::Result<()>,
    ) -> io::Result<(i32, Option<i32>)> {
        let Some(stretch) = self.query.timeline.at(at) else {
            return Ok((at, None));
        };
        let recording = self.query.replay.recording();
        let run = recording.run(stretch.run)?;
        let span = read::ordinal(recording, stretch.run, at)? / (UNIT_ORDINALS * SPAN_BLOCKS);
        let start = run.block_first_hour(span * SPAN_BLOCKS);
        let end = start.saturating_add((UNIT_ORDINALS * SPAN_BLOCKS - 1) as i32);
        let holders = self.query.holders.interval(at);
        let edge = limit.clamp(start.max(holders.0).max(stretch.first), end.min(holders.1).min(stretch.last));
        let winners = WinnerSet { slots: winners, n, least };
        let proven = match winners.proven_through(self, stretch.run, span, (at, edge), check_cancel) {
            Err(error) if error.kind() == io::ErrorKind::InvalidData => None,
            result => result?,
        };
        Ok((edge, proven))
    }
}

impl WinnerSet<'_> {
    fn proven_through(
        &self,
        line: &Line<'_, '_>,
        run: u32,
        span: u32,
        (at, edge): (i32, i32),
        check_cancel: &dyn Fn() -> io::Result<()>,
    ) -> io::Result<Option<i32>> {
        let recording = line.query.replay.recording();
        let (first, last) = (at.min(edge), at.max(edge));
        let drop_zero = !self.least && line.measure.drops_zeros_in_top();
        if last.abs_diff(first) + 1 < UNIT_ORDINALS
            || (line.series.absent_none && !drop_zero)
            || line.withheld_at(run, first).is_some()
            || span as usize >= recording.run(run)?.directory().spans.len()
        {
            return Ok(None);
        }
        let coverage = read::statuses(recording, run, line.series.table, first, last)?;
        if !matches!(coverage.as_slice(), [held] if held.status == Status::Complete && (held.first, held.last) == (first, last)) {
            return Ok(None);
        }
        let bytes = crate::cost::storage(|| {
            recording
                .span_summaries(run, span, line.series.table.index())?
                .into_iter()
                .find(|&(level, _)| level == 1)
                .map(|(_, extent)| recording.stored_bytes(&extent))
                .transpose()
        })?;
        let Some(bytes) = bytes else { return Ok(None) };
        let piece = Piece::read(&bytes)?;
        let dir = &piece.directory;
        if dir.level != 1 || i64::from(first) < dir.first_hour || i64::from(last) >= dir.first_hour + i64::from(dir.hours) {
            return Ok(None);
        }
        let Some(column) = piece.column_of(line.column) else {
            return Ok(None);
        };
        let frame = piece.column(column)?;
        let bucket = |hour| (summary::bucket_of(i64::from(hour), dir.level) - dir.first_bucket) as usize;
        let (lo, hi) = (bucket(first), bucket(last));
        let mut bounds = vec![Bounds::default(); hi - lo + 1];
        let owners = line.rank_owners(first);
        let mut seen = HashSet::new();
        for (row, &key) in piece.keys.iter().enumerate() {
            check_cancel()?;
            if line.series.skips_row(key) {
                continue;
            }
            let owner = owners.get(&key);
            let winner = matches!(owner, Some(RankOwner::Tag(_, slot)) if self.slots.binary_search(slot).is_ok());
            if let Some(&RankOwner::Tag(_, slot)) = owner
                && winner
                && !seen.insert(slot)
            {
                return Ok(None);
            }
            let lives = piece.life(row)?;
            let values = frame.buckets(&piece, row)?;
            for (offset, bound) in bounds.iter_mut().enumerate() {
                let bin = lo + offset;
                let value = values[bin];
                if value.map_or(0, |value| value.hours) != lives[bin].hours {
                    bound.uncertain = true;
                }
                let Some(value) = value else { continue };
                let (min, max) = (value.min, value.max);
                if drop_zero && min == 0 && max == 0 {
                    continue;
                }
                if owner.is_none() {
                    bound.uncertain = true;
                }
                if winner {
                    if value.hours != dir.covered(bin as u32) || (drop_zero && min <= 0 && max >= 0) {
                        bound.uncertain = true;
                    }
                    bound.winners += 1;
                    bound.weakest = Some(if self.least {
                        bound.weakest.map_or(max, |v| v.max(max))
                    } else {
                        bound.weakest.map_or(min, |v| v.min(min))
                    });
                } else {
                    bound.challenger = Some(if self.least {
                        bound.challenger.map_or(min, |v| v.min(min))
                    } else {
                        bound.challenger.map_or(max, |v| v.max(max))
                    });
                }
            }
        }
        let uncertain = |bound: &Bounds| !bound.proves(self.slots.len(), self.n, self.least);
        let forward = at <= edge;
        let stop = if forward {
            bounds.iter().position(uncertain)
        } else {
            bounds.iter().rposition(uncertain)
        };
        let proven = match stop {
            None => Some(edge),
            Some(offset) => {
                let size = summary::bucket_hours(dir.level);
                let start = (dir.first_bucket + (lo + offset) as i64) * size;
                let last_proven = if forward { start - 1 } else { start + size };
                (i64::from(first)..=i64::from(last))
                    .contains(&last_proven)
                    .then_some(last_proven as i32)
            }
        };
        Ok(proven)
    }
}
