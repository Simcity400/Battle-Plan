//! A timeline as the recording's runs hold it: each stretch of consecutive recorded hours of one
//! run, how it follows the stretch before (SPEC 3.4), and the hours of the timeline no run holds.

use replay::campaign::Replay;
use std::io;

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

/// How a stretch follows the one before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Join {
    /// The timeline's first stretch.
    Start,
    /// A load into a recorded hour of the stretch before, with no hour missing: derived columns
    /// carry across it by their declared rule; `parent` is the last hour the parent stretch holds.
    Load { parent: i32 },
    /// Anything else: hours missing between them, or a run that starts over. Nothing carries.
    Break,
}

/// Consecutive recorded hours of one run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stretch {
    pub run: u32,
    pub first: i32,
    pub last: i32,
    pub join: Join,
}

/// Hours of the timeline no run holds, with the recording's reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unheld {
    pub first: i32,
    pub last: i32,
    pub reason: String,
}

#[derive(Clone, Debug)]
pub struct Timeline {
    pub id: u64,
    pub first: i32,
    pub last: i32,
    /// In hour order, never overlapping.
    pub stretches: Vec<Stretch>,
    pub unheld: Vec<Unheld>,
}

impl Timeline {
    /// The stretch holding `hour`.
    pub fn at(&self, hour: i32) -> Option<&Stretch> {
        let at = self.stretches.partition_point(|stretch| stretch.last < hour);
        self.stretches.get(at).filter(|stretch| stretch.first <= hour)
    }

    /// The stretches meeting `from..=to`, each cut to it.
    pub fn within(&self, from: i32, to: i32) -> impl Iterator<Item = Stretch> + '_ {
        let start = self.stretches.partition_point(|stretch| stretch.last < from);
        self.stretches[start..]
            .iter()
            .take_while(move |stretch| stretch.first <= to)
            .map(move |stretch| Stretch {
                first: stretch.first.max(from),
                last: stretch.last.min(to),
                ..*stretch
            })
    }

    /// The unheld hours meeting `from..=to`, each cut to it.
    pub fn unheld_within(&self, from: i32, to: i32) -> Vec<Unheld> {
        self.unheld
            .iter()
            .filter(|gap| gap.first <= to && gap.last >= from)
            .map(|gap| Unheld {
                first: gap.first.max(from),
                last: gap.last.min(to),
                reason: gap.reason.clone(),
            })
            .collect()
    }

    /// Why `hour` holds no recorded value: the recording's reason for its gap.
    pub fn reason(&self, hour: i32) -> String {
        self.unheld
            .iter()
            .find(|gap| gap.first <= hour && hour <= gap.last)
            .map_or_else(|| "The timeline holds no such hour".to_owned(), |gap| gap.reason.clone())
    }
}

/// Timeline `id` of `replay` as its runs hold it.
pub fn resolve(replay: &Replay, id: u64) -> io::Result<Timeline> {
    let timeline = replay
        .timelines()
        .and_then(|timelines| timelines.get(id))
        .ok_or_else(|| invalid(format!("The recording holds no history {id}")))?;
    let mut stretches: Vec<Stretch> = Vec::new();
    let mut unheld: Vec<Unheld> = Vec::new();
    for (at, piece) in timeline.pieces.iter().enumerate() {
        let mut blocks: Vec<_> = replay
            .index
            .iter()
            .filter(|block| block.branch == piece.segment && block.last_hour >= piece.first_hour && block.first_hour <= piece.last_hour)
            .collect();
        blocks.sort_unstable_by_key(|block| block.first_hour);
        let mut first_of_piece = true;
        for block in blocks {
            let first = block.first_hour.max(piece.first_hour);
            let last = block.last_hour.min(piece.last_hour);
            if let Some(held) = stretches.last_mut()
                && held.run == block.run
                && i64::from(held.last) + 1 == i64::from(first)
            {
                held.last = last;
                first_of_piece = false;
                continue;
            }
            let join = match (stretches.last(), first_of_piece && at > 0) {
                (None, _) => Join::Start,
                (Some(previous), true) if first == previous.last || i64::from(first) == i64::from(previous.last) + 1 => Join::Load { parent: previous.last },
                (Some(_), _) => Join::Break,
            };
            if let Some(previous) = stretches.last()
                && i64::from(first) > i64::from(previous.last) + 1
            {
                unheld.push(Unheld {
                    first: previous.last + 1,
                    last: first - 1,
                    reason: gap_reason(replay, piece.segment, previous.last + 1),
                });
            }
            stretches.push(Stretch { run: block.run, first, last, join });
            first_of_piece = false;
        }
    }
    if stretches.is_empty() {
        return Err(invalid(format!("History {id} holds no recorded hour")));
    }
    let first = timeline.first_hour().min(stretches[0].first);
    let last = timeline.last_hour().max(stretches[stretches.len() - 1].last);
    if first < stretches[0].first {
        unheld.insert(0, Unheld {
            first,
            last: stretches[0].first - 1,
            reason: gap_reason(replay, timeline.pieces[0].segment, first),
        });
    }
    let end = stretches[stretches.len() - 1].last;
    if end < last {
        unheld.push(Unheld {
            first: end + 1,
            last,
            reason: gap_reason(replay, timeline.pieces[timeline.pieces.len() - 1].segment, end + 1),
        });
    }
    Ok(Timeline { id, first, last, stretches, unheld })
}

/// The recording's reason for the gap of `segment` holding `hour`.
fn gap_reason(replay: &Replay, segment: u64, hour: i32) -> String {
    replay
        .gaps
        .iter()
        .find(|gap| gap.branch == segment && gap.first_hour <= hour && hour <= gap.last_hour)
        .map_or_else(|| "The recording holds no hour here".to_owned(), |gap| gap.reason.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timeline() -> Timeline {
        Timeline {
            id: 1,
            first: 100,
            last: 399,
            stretches: vec![
                Stretch { run: 0, first: 100, last: 199, join: Join::Start },
                Stretch { run: 1, first: 200, last: 249, join: Join::Load { parent: 199 } },
                Stretch { run: 2, first: 300, last: 399, join: Join::Break },
            ],
            unheld: vec![Unheld { first: 250, last: 299, reason: "The recorder lost these hours".into() }],
        }
    }

    #[test]
    fn hours_over_a_gap_are_cut_to_their_stretches_and_the_gap_keeps_its_reason() {
        let timeline = timeline();
        let within: Vec<_> = timeline.within(190, 310).map(|stretch| (stretch.run, stretch.first, stretch.last)).collect();
        assert_eq!(within, vec![(0, 190, 199), (1, 200, 249), (2, 300, 310)]);
        assert!(timeline.at(260).is_none());
        assert_eq!(timeline.reason(260), "The recorder lost these hours");
        assert_eq!(timeline.unheld_within(240, 270), vec![Unheld { first: 250, last: 270, reason: "The recorder lost these hours".into() }]);
        assert_eq!(timeline.at(200).map(|stretch| stretch.join), Some(Join::Load { parent: 199 }));
    }
}
