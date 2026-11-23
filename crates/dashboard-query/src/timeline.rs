
use replay::campaign::Replay;
use std::io;

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Join {
    Start,
    Load { parent: i32 },
    Break,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stretch {
    pub run: u32,
    pub first: i32,
    pub last: i32,
    pub join: Join,
}

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
    pub stretches: Vec<Stretch>,
    pub unheld: Vec<Unheld>,
}

impl Timeline {
    pub fn at(&self, hour: i32) -> Option<&Stretch> {
        let at = self.stretches.partition_point(|stretch| stretch.last < hour);
        self.stretches.get(at).filter(|stretch| stretch.first <= hour)
    }

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

    pub fn reason(&self, hour: i32) -> String {
        self.unheld
            .iter()
            .find(|gap| gap.first <= hour && hour <= gap.last)
            .map_or_else(|| "The timeline holds no such hour".to_owned(), |gap| gap.reason.clone())
    }
}

pub fn resolve(replay: &Replay, id: u64) -> io::Result<Timeline> {
    let timeline = replay
        .timelines()
        .and_then(|timelines| timelines.get(id))
        .ok_or_else(|| invalid(format!("The recording holds no history {id}")))?;
    let mut stretches: Vec<Stretch> = Vec::new();
    let mut unheld: Vec<Unheld> = Vec::new();
    for (at, piece) in timeline.pieces.iter().enumerate() {
        let mut blocks: Vec<_> = replay
            .pages()
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
                    reason: gap_reason(replay, id, previous.last + 1),
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
            reason: gap_reason(replay, id, first),
        });
    }
    let end = stretches[stretches.len() - 1].last;
    if end < last {
        unheld.push(Unheld {
            first: end + 1,
            last,
            reason: gap_reason(replay, id, end + 1),
        });
    }
    Ok(Timeline { id, first, last, stretches, unheld })
}

fn gap_reason(replay: &Replay, id: u64, hour: i32) -> String {
    replay
        .gaps
        .iter()
        .find(|gap| gap.branch == id && gap.first_hour <= hour && hour <= gap.last_hour)
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

    #[test]
    fn a_history_of_several_segments_holds_every_segment_s_hours() {
        use crate::fixture::segment;
        let records = [segment(1, 100, 50), segment(2, 150, 50), segment(3, 250, 50)];
        let replay = Replay::from_records(recorder_derive::writers(), b"tiles".to_vec(), &records).unwrap().into_timelines(&Default::default()).unwrap();
        let pieces: Vec<_> = replay.timelines().unwrap().get(1).unwrap().pieces.iter().map(|piece| piece.segment).collect();
        assert_eq!(pieces, [1, 2, 3]);
        let resolved = resolve(&replay, 1).unwrap();
        let stretches: Vec<_> = resolved.stretches.iter().map(|stretch| (stretch.first, stretch.last, stretch.join)).collect();
        assert_eq!(stretches, [(100, 149, Join::Start), (150, 199, Join::Load { parent: 149 }), (250, 299, Join::Break)]);
        assert_eq!(resolved.unheld, [Unheld { first: 200, last: 249, reason: "Not recorded: this play happened while the recorder wasn't watching".into() }]);
    }
}
