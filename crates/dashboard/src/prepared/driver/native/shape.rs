//! Where a native recording's segments fall: each one's branch and hours, each branch's extent,
//! and the proof of the recorded input through an hour, by which a family's committed frontier
//! is taken up again only while the recording, as it grows, still holds what it accepted.

use dashboard_base::prepared::roots::Frontier;
use replay::columnar_frames::{Recording, Segment};
use std::{
    collections::BTreeMap,
    io,
    sync::{Mutex, PoisonError},
};

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_owned())
}

/// A segment's place in its branch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Placed {
    pub branch: u64,
    pub run: u32,
    /// Ordinals of the run, inclusive.
    pub first: u32,
    pub last: u32,
    pub first_hour: i32,
    pub last_hour: i32,
    /// No segment of its branch follows it.
    pub ends_branch: bool,
}

/// A branch's hours over its readable runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Extent {
    pub first_hour: i32,
    pub last_hour: i32,
}

pub(crate) struct Shape {
    placed: Vec<Placed>,
    branches: BTreeMap<u64, Extent>,
    /// The proofs read, by run and ordinal: None where the recording cannot be read.
    proofs: Mutex<BTreeMap<(u32, u32), Option<[u8; 32]>>>,
}

impl Shape {
    pub(crate) fn new(recording: &Recording) -> io::Result<Self> {
        let segments = recording.segments();
        let mut placed: Vec<Placed> = engine_allocation::vec(segments.len())?;
        let mut branches: BTreeMap<u64, Extent> = BTreeMap::new();
        let mut latest: BTreeMap<u64, usize> = BTreeMap::new();
        for (at, segment) in segments.iter().enumerate() {
            let run = recording.run(segment.run)?;
            let hour = |ordinal: u32| {
                i32::try_from(i64::from(run.first_hour) + i64::from(ordinal))
                    .map_err(|_| invalid("A segment's hours pass the clock"))
            };
            let (first_hour, last_hour) = (hour(segment.first)?, hour(segment.last)?);
            placed.push(Placed {
                branch: run.branch,
                run: segment.run,
                first: segment.first,
                last: segment.last,
                first_hour,
                last_hour,
                ends_branch: false,
            });
            branches
                .entry(run.branch)
                .and_modify(|extent| {
                    extent.first_hour = extent.first_hour.min(first_hour);
                    extent.last_hour = extent.last_hour.max(last_hour);
                })
                .or_insert(Extent {
                    first_hour,
                    last_hour,
                });
            latest.insert(run.branch, at);
        }
        for at in latest.into_values() {
            placed[at].ends_branch = true;
        }
        Ok(Self {
            placed,
            branches,
            proofs: Mutex::default(),
        })
    }

    /// Every segment, in the recording's order.
    pub(crate) fn len(&self) -> usize {
        self.placed.len()
    }

    pub(crate) fn get(&self, at: usize) -> &Placed {
        &self.placed[at]
    }

    /// The place of `segment`, one of the recording's.
    pub(crate) fn at(&self, segment: &Segment) -> io::Result<usize> {
        self.placed
            .binary_search_by(|placed| {
                (placed.run, placed.first).cmp(&(segment.run, segment.first))
            })
            .map_err(|_| invalid("A segment the recording does not hold"))
    }

    pub(crate) fn branches(&self) -> &BTreeMap<u64, Extent> {
        &self.branches
    }

    /// The proof of the recorded input through ordinal `ordinal` of run `run`: the hour digest
    /// chain the recording holds before the page holding it, which a recording grown from this
    /// one holds alike; None when it cannot be read.
    fn proof_at(&self, recording: &Recording, run: u32, ordinal: u32) -> Option<[u8; 32]> {
        let mut proofs = self.proofs.lock().unwrap_or_else(PoisonError::into_inner);
        *proofs
            .entry((run, ordinal))
            .or_insert_with(|| recording.chain_before(run, ordinal).ok())
    }

    /// The proof of the recorded input through segment `at`'s last hour: zeros, which no
    /// frontier is taken up again with, when it cannot be read.
    pub(crate) fn proof(&self, recording: &Recording, at: usize) -> [u8; 32] {
        self.proof_through(recording, at, self.placed[at].last)
    }

    /// The proof of the recorded input through ordinal `last` of segment `at`'s run, as
    /// [`Self::proof`].
    pub(crate) fn proof_through(&self, recording: &Recording, at: usize, last: u32) -> [u8; 32] {
        self.proof_at(recording, self.placed[at].run, last)
            .unwrap_or([0; 32])
    }

    /// Whether the recording holds hours of `branch` after `frontier`'s last.
    pub(crate) fn after(&self, branch: u64, frontier: &Frontier) -> bool {
        let next = (frontier.next.run, frontier.next.ordinal);
        self.placed
            .iter()
            .rev()
            .find(|placed| placed.branch == branch)
            .is_some_and(|placed| (placed.run, placed.last) >= next)
    }

    /// Whether a family's committed `frontier` on `branch` stands for `recording` as it is: the
    /// recording still holds the branch's input it accepted through its last hour, and it ended
    /// there, or the recording holds nothing of the branch after it, or it keeps the state its
    /// family goes on with ([`Frontier::state`]).
    pub(crate) fn accepts(&self, recording: &Recording, branch: u64, frontier: &Frontier) -> bool {
        let (run, Some(last)) = (frontier.next.run, frontier.next.ordinal.checked_sub(1)) else {
            return false;
        };
        let at = self
            .placed
            .partition_point(|placed| (placed.run, placed.last) < (run, last));
        let holds = self.placed.get(at).is_some_and(|placed| {
            placed.run == run && placed.first <= last && placed.branch == branch
        });
        holds
            && self.proof_at(recording, run, last) == Some(frontier.proof)
            && (frontier.end.is_some() || frontier.state.is_some() || !self.after(branch, frontier))
    }
}
