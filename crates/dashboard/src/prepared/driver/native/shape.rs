//! Where a native recording's segments fall: each one's branch and hours, each branch's extent,
//! and the proof of the recorded input through an hour, by which a family's committed frontier
//! is taken up again only while the recording, as it grows, still holds what it accepted. A
//! segment's branch is the one its walks read it as ([`Segment::branch`]): a timeline reads
//! other branches' runs before its own, the last it reads of each brought earlier.

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

/// A run a branch reads: the last ordinal it reads of it, and the run's own branch.
#[derive(Clone, Copy, Debug)]
struct Read {
    run: u32,
    last: u32,
    recorded: u64,
}

/// [`Shape::proof_at`], read: of each run of another branch that `reads` names before `run`,
/// where it reads through and the chain there; then the chain of every earlier run of `run`'s
/// own branch, whole; then of `run` through `ordinal`.
fn proven(recording: &Recording, reads: &[Read], run: u32, ordinal: u32) -> io::Result<[u8; 32]> {
    let branch = recording.run(run)?.branch;
    let mut proof = blake3::Hasher::new();
    for read in reads.iter().take_while(|read| read.run < run) {
        if read.recorded != branch {
            proof.update(&read.run.to_le_bytes());
            proof.update(&read.last.to_le_bytes());
            proof.update(&recording.chain_through(read.run, read.last)?);
        }
    }
    for earlier in 0..run {
        let held = recording.run(earlier)?;
        if held.branch == branch {
            proof.update(&held.chain);
        }
    }
    proof.update(&recording.chain_through(run, ordinal)?);
    Ok(*proof.finalize().as_bytes())
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

/// A branch's segments, by their places in order, and the runs it reads, in order.
#[derive(Default)]
struct Of {
    places: Vec<usize>,
    reads: Vec<Read>,
}

pub(crate) struct Shape {
    placed: Vec<Placed>,
    branches: BTreeMap<u64, Extent>,
    of: BTreeMap<u64, Of>,
    /// The proofs read, by branch, run and ordinal: None where the recording cannot be read.
    proofs: Mutex<BTreeMap<(u64, u32, u32), Option<[u8; 32]>>>,
}

impl Shape {
    /// The shape of the segments `recording`'s walks take: each branch's in the order of its
    /// runs' ordinals, none overlapping another.
    pub(crate) fn new(recording: &Recording) -> io::Result<Self> {
        let segments = recording.segments();
        let mut placed: Vec<Placed> = engine_allocation::vec(segments.len())?;
        let mut branches: BTreeMap<u64, Extent> = BTreeMap::new();
        let mut of: BTreeMap<u64, Of> = BTreeMap::new();
        for (at, segment) in segments.iter().enumerate() {
            let run = recording.run(segment.run)?;
            let hour = |ordinal: u32| {
                i32::try_from(i64::from(run.first_hour) + i64::from(ordinal))
                    .map_err(|_| invalid("A segment's hours pass the clock"))
            };
            let (first_hour, last_hour) = (hour(segment.first)?, hour(segment.last)?);
            let held = of.entry(segment.branch).or_default();
            if let Some(&before) = held.places.last()
                && (placed[before].run, placed[before].last) >= (segment.run, segment.first)
            {
                return Err(invalid("A branch's segments go back in the recording"));
            }
            engine_allocation::reserve(&mut held.places, 1)?;
            held.places.push(at);
            match held.reads.last_mut() {
                Some(read) if read.run == segment.run => read.last = segment.last,
                _ => {
                    engine_allocation::reserve(&mut held.reads, 1)?;
                    held.reads.push(Read {
                        run: segment.run,
                        last: segment.last,
                        recorded: run.branch,
                    });
                }
            }
            placed.push(Placed {
                branch: segment.branch,
                run: segment.run,
                first: segment.first,
                last: segment.last,
                first_hour,
                last_hour,
                ends_branch: false,
            });
            branches
                .entry(segment.branch)
                .and_modify(|extent| {
                    extent.first_hour = extent.first_hour.min(first_hour);
                    extent.last_hour = extent.last_hour.max(last_hour);
                })
                .or_insert(Extent {
                    first_hour,
                    last_hour,
                });
        }
        for held in of.values() {
            if let Some(&last) = held.places.last() {
                placed[last].ends_branch = true;
            }
        }
        Ok(Self {
            placed,
            branches,
            of,
            proofs: Mutex::default(),
        })
    }

    /// Every segment, in the order the recording's walks take them.
    pub(crate) fn len(&self) -> usize {
        self.placed.len()
    }

    pub(crate) fn get(&self, at: usize) -> &Placed {
        &self.placed[at]
    }

    /// The place of `segment`, one of those the recording's walks take.
    pub(crate) fn at(&self, segment: &Segment) -> io::Result<usize> {
        let places = self.places(segment.branch);
        places
            .binary_search_by(|&at| {
                let placed = &self.placed[at];
                (placed.run, placed.first).cmp(&(segment.run, segment.first))
            })
            .map(|found| places[found])
            .map_err(|_| invalid("A segment the recording does not hold"))
    }

    pub(crate) fn branches(&self) -> &BTreeMap<u64, Extent> {
        &self.branches
    }

    /// The places of `branch`'s segments, in order.
    fn places(&self, branch: u64) -> &[usize] {
        self.of.get(&branch).map_or(&[][..], |held| held.places.as_slice())
    }

    /// The proof of the recorded input `branch` reads through ordinal `ordinal` of run `run`:
    /// the hour digest chain of each run of another branch it reads before, through where it
    /// reads it and named by that, then of every earlier run of `run`'s branch, whole, then of
    /// `run` through that ordinal ([`Recording::chain_through`]). A recording grown from this
    /// one, its branch read alike, proves the same there; one whose hours it reads to it differ,
    /// or that reads them from elsewhere, does not. None when it cannot be read.
    fn proof_at(
        &self,
        recording: &Recording,
        branch: u64,
        run: u32,
        ordinal: u32,
    ) -> Option<[u8; 32]> {
        let mut proofs = self.proofs.lock().unwrap_or_else(PoisonError::into_inner);
        *proofs.entry((branch, run, ordinal)).or_insert_with(|| {
            let reads = self.of.get(&branch).map_or(&[][..], |held| held.reads.as_slice());
            proven(recording, reads, run, ordinal).ok()
        })
    }

    /// The proof of the recorded input through segment `at`'s last hour: zeros, which no
    /// frontier is taken up again with, when it cannot be read.
    pub(crate) fn proof(&self, recording: &Recording, at: usize) -> [u8; 32] {
        self.proof_through(recording, at, self.placed[at].last)
    }

    /// The proof of the recorded input through ordinal `last` of segment `at`'s run, as
    /// [`Self::proof`].
    pub(crate) fn proof_through(&self, recording: &Recording, at: usize, last: u32) -> [u8; 32] {
        let placed = &self.placed[at];
        self.proof_at(recording, placed.branch, placed.run, last)
            .unwrap_or([0; 32])
    }

    /// Whether the recording holds hours of `branch` after `frontier`'s last.
    pub(crate) fn after(&self, branch: u64, frontier: &Frontier) -> bool {
        let next = (frontier.next.run, frontier.next.ordinal);
        self.places(branch).last().is_some_and(|&at| {
            let placed = &self.placed[at];
            (placed.run, placed.last) >= next
        })
    }

    /// The first segment of `branch` holding an hour after `frontier`, a family's last on it, or
    /// the branch's first segment without one, and the first ordinal of it after `frontier`. None
    /// when the recording holds nothing of the branch after it.
    pub(crate) fn unpublished(
        &self,
        branch: u64,
        frontier: Option<&Frontier>,
    ) -> Option<(usize, u32)> {
        let next = frontier.map(|frontier| (frontier.next.run, frontier.next.ordinal));
        self.places(branch).iter().find_map(|&at| {
            let placed = &self.placed[at];
            match next {
                None => Some((at, placed.first)),
                Some((run, ordinal)) if (placed.run, placed.last) >= (run, ordinal) => {
                    let first = if placed.run == run {
                        placed.first.max(ordinal)
                    } else {
                        placed.first
                    };
                    Some((at, first))
                }
                Some(_) => None,
            }
        })
    }

    /// Whether a family's committed `frontier` on `branch` stands for `recording` as it is: the
    /// recording still holds the branch's input it accepted through its last hour, and it ended
    /// there, or the recording holds nothing of the branch after it, or it keeps the state its
    /// family goes on with ([`Frontier::state`]).
    pub(crate) fn accepts(&self, recording: &Recording, branch: u64, frontier: &Frontier) -> bool {
        let (run, Some(last)) = (frontier.next.run, frontier.next.ordinal.checked_sub(1)) else {
            return false;
        };
        let places = self.places(branch);
        let at = places.partition_point(|&at| {
            let placed = &self.placed[at];
            (placed.run, placed.last) < (run, last)
        });
        let holds = places.get(at).is_some_and(|&at| {
            let placed = &self.placed[at];
            placed.run == run && placed.first <= last
        });
        holds
            && self.proof_at(recording, branch, run, last) == Some(frontier.proof)
            && (frontier.end.is_some() || frontier.state.is_some() || !self.after(branch, frontier))
    }
}
