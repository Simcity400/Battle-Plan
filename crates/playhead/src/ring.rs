//! The steps built and not yet taken by every consumer, the steps consumers still hold, and the
//! steps made for one consumer alone. A step is shared, never copied: each consumer takes its own
//! reference, and a step comes back to the stepper once only the ring holds it.

use crate::{changes::Changes, playhead::Reader, step::Step};
use replay_data::Hour;
use std::{collections::VecDeque, sync::Arc};

/// One consumer: its reader, its next shared step, and the steps made for it alone, each handed
/// once every shared step before `before` is taken. A parked one ([`Ring::park_behind`]) takes
/// nothing and holds no step of its piece; it keeps the hour it stood at, and only steps past a
/// reset or a gap after that hour wait for it.
struct Seat {
    reader: Reader,
    next: u64,
    own: VecDeque<(u64, Arc<Step>)>,
    parked: Option<Hour>,
    /// The last step it took left every table at its last hour (one that took none holds no table):
    /// its tables stand where it stands.
    whole: bool,
    /// It has said where its frames stand ([`crate::Consumer::advance`]). One that never has takes
    /// steps at hours another reader gives it (a request on a map's behalf), so it is never parked:
    /// its steps are as far behind as that reader's, which is.
    asks: bool,
    /// It lands itself when far behind ([`crate::Consumer::lands_itself`]): never parked.
    lands_itself: bool,
}

/// A step built past the hour the frames are at: its seq, hours, and whether it is a gap.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Span {
    pub(crate) seq: u64,
    pub(crate) first: Hour,
    pub(crate) last: Hour,
    pub(crate) gap: bool,
}

pub(crate) struct Ring {
    pub(crate) epoch: u64,
    size: usize,
    next_seq: u64,
    built: VecDeque<Arc<Step>>,
    /// Taken by every consumer; back with the stepper once no consumer holds it.
    lent: Vec<Arc<Step>>,
    consumers: Vec<Option<Seat>>,
    /// Steps made for a reader none of whose consumers had joined yet: the first that joins takes them.
    orphans: Vec<(Reader, u64, Arc<Step>)>,
    /// The last hour of the newest shared step any consumer took in this epoch.
    taken: Option<Hour>,
    /// The epoch's newest reset (the seek's, or a piece's first hour): its seq and hour.
    reset: Option<(u64, Hour)>,
    /// The last hour of the newest step built.
    published: Option<Hour>,
    /// The most bytes it keeps for a consumer behind: the lanes' cells, the state its steps change.
    bound: usize,
}

/// Every table of `step` stands at its last hour.
fn whole(step: &Step) -> bool {
    step.tables.iter().all(|changes| changes.short() == 0 && changes.head().is_none_or(|head| head >= step.last))
}

fn next_of(built: &VecDeque<Arc<Step>>, next: u64) -> usize {
    built.partition_point(|step| step.seq < next)
}

/// Where `before` goes among `own`, after every step already there for the same place.
fn place(own: &VecDeque<(u64, Arc<Step>)>, before: u64) -> usize {
    own.partition_point(|(held, _)| *held <= before)
}

impl Ring {
    pub(crate) fn new(size: usize) -> Self {
        Self { epoch: 0, size, next_seq: 0, built: VecDeque::new(), lent: Vec::new(), consumers: Vec::new(), orphans: Vec::new(), taken: None, reset: None, published: None, bound: usize::MAX }
    }

    /// Fewer than its size of the steps built end past `head`, and fewer than its size are not yet
    /// taken by the consumer furthest on. The head bounds how far ahead of the clock the stepper
    /// builds; the furthest consumer bounds how far ahead of every reader, so a head driven to the
    /// newest step does not chase the stepper. A step the head has passed and a slower consumer has
    /// not taken waits for it, shared, and takes no room: a consumer late to take holds nobody, and
    /// once those steps hold more than the lanes' state it is parked ([`Self::park_behind`]).
    pub(crate) fn room(&self, head: Hour) -> bool {
        self.past(head) < self.size && self.untaken() < self.size
    }

    fn furthest(&self) -> Option<u64> {
        self.consumers.iter().flatten().filter(|seat| seat.parked.is_none()).map(|seat| seat.next).max()
    }

    /// Where a seat holds the steps from: its next, or for a parked one the first reset or gap past
    /// the hour it stood at.
    fn holds_from(&self, seat: &Seat) -> u64 {
        match seat.parked {
            None => seat.next,
            Some(from) => self.piece_end(from).map_or(self.next_seq, |step| step.seq),
        }
    }

    /// The first step built after `from` that leaves its piece: a reset or a gap.
    fn piece_end(&self, from: Hour) -> Option<&Arc<Step>> {
        self.built.iter().find(|step| step.first > from && (step.reset || step.piece.is_none()))
    }

    /// The steps built that no consumer has taken yet (with no consumer, every one).
    fn untaken(&self) -> usize {
        self.built.len() - next_of(&self.built, self.furthest().unwrap_or(0))
    }

    /// The steps built that end past `head` (steps are in the order of their hours).
    pub(crate) fn past(&self, head: Hour) -> usize {
        self.built.len() - self.built.partition_point(|step| step.last <= head)
    }

    /// The steps built and not yet taken by every consumer.
    #[cfg(test)]
    pub(crate) fn steps_kept(&self) -> usize {
        self.built.len()
    }

    /// The bytes of the steps the consumer furthest on has taken and a slower one has not: what
    /// the ring keeps for consumers behind (the steps past the furthest are its read-ahead).
    #[cfg(test)]
    pub(crate) fn trailing_bytes(&self) -> usize {
        self.built.range(..self.trailing_end()).map(|step| step.payload()).sum()
    }

    pub(crate) fn set_bound(&mut self, bytes: usize) {
        self.bound = bytes;
    }

    #[cfg(test)]
    pub(crate) fn largest_kept(&self) -> usize {
        self.built.iter().map(|step| step.payload()).max().unwrap_or(0)
    }

    /// The place after the steps the consumer furthest on has taken: one that is there, not parked
    /// and holding no landing it has not taken yet.
    fn trailing_end(&self) -> usize {
        let furthest = self.consumers.iter().flatten().filter(|seat| seat.parked.is_none() && seat.own.is_empty()).map(|seat| seat.next).max();
        furthest.map_or(0, |next| next_of(&self.built, next))
    }

    /// Each consumer for whom it keeps more than its bound (the bytes of the steps the consumer
    /// furthest on has taken and it has not) is parked: it lets go of them and takes nothing until [`Self::land_at`] or
    /// [`Self::jump`]. Not one that never said where it stands, holds a step made for it alone,
    /// stands before a reset, or was left short by the last step it took (its tables stand before
    /// the hour it does).
    fn park_behind(&mut self) {
        let end = self.trailing_end();
        if self.built.range(..end).map(|step| step.payload()).sum::<usize>() <= self.bound {
            return;
        }
        let mut parks = Vec::new();
        for (id, seat) in self.consumers.iter().enumerate() {
            let Some(seat) = seat.as_ref().filter(|seat| seat.asks && !seat.lands_itself && seat.parked.is_none() && seat.own.is_empty() && seat.whole) else { continue };
            let at = next_of(&self.built, seat.next);
            let held: usize = self.built.range(at.min(end)..end).map(|step| step.payload()).sum();
            if let Some(first) = self.built.get(at).filter(|step| held > self.bound && !step.reset) {
                parks.push((id, first.first - 1));
            }
        }
        let next = self.next_seq;
        for (id, from) in parks {
            if let Some(seat) = self.consumers[id].as_mut() {
                (seat.next, seat.parked) = (next, Some(from));
            }
        }
    }

    /// Every step of the epoch is abandoned, those made for one consumer included: a new one begins.
    pub(crate) fn abandon(&mut self) -> u64 {
        self.epoch += 1;
        self.lent.extend(self.built.drain(..));
        (self.taken, self.reset) = (None, None);
        let next = self.next_seq;
        for seat in self.consumers.iter_mut().flatten() {
            seat.next = next;
            seat.parked = None;
            self.lent.extend(seat.own.drain(..).map(|(_, step)| step));
        }
        self.lent.extend(self.orphans.drain(..).map(|(_, _, step)| step));
        self.epoch
    }

    /// The step built, unless its epoch was abandoned: then it comes back.
    pub(crate) fn publish(&mut self, mut step: Arc<Step>) -> Result<(), Arc<Step>> {
        if step.epoch != self.epoch {
            return Err(step);
        }
        let held = Arc::get_mut(&mut step).expect("a step being published is held once");
        held.seq = self.next_seq;
        if held.reset {
            self.reset = Some((held.seq, held.last));
        }
        self.published = Some(held.last);
        self.next_seq += 1;
        self.built.push_back(step);
        Ok(())
    }

    /// A step no consumer holds any more, to be built again, and the others no consumer holds, to
    /// be freed: one whose buffers are far larger than what it last held, and every free one past
    /// the ring's size, which stay for the builds to come (a burst of them after a frame let go of
    /// several makes none anew).
    pub(crate) fn reclaim(&mut self) -> (Option<Arc<Step>>, Vec<Arc<Step>>) {
        let (mut kept, mut spare, mut free) = (None, Vec::new(), 0);
        let mut at = 0;
        while at < self.lent.len() {
            if Arc::strong_count(&self.lent[at]) != 1 {
                at += 1;
                continue;
            }
            let oversized = self.lent[at].bytes_held() > 2 * self.lent[at].payload() + (1 << 20);
            if !oversized && kept.is_none() {
                kept = Some(self.lent.swap_remove(at));
            } else if !oversized && free < self.size {
                free += 1;
                at += 1;
            } else {
                spare.push(self.lent.swap_remove(at));
            }
        }
        (kept, spare)
    }

    /// A consumer of `reader`, from the oldest step kept on, with the steps made for its reader
    /// before it joined.
    pub(crate) fn join(&mut self, reader: Reader) -> usize {
        let start = self.built.front().map_or(self.next_seq, |step| step.seq);
        let mut own = VecDeque::new();
        let mut at = 0;
        while at < self.orphans.len() {
            if self.orphans[at].0 == reader {
                let (_, before, step) = self.orphans.remove(at);
                own.push_back((before, step));
            } else {
                at += 1;
            }
        }
        self.consumers.push(Some(Seat { reader, next: start, own, parked: None, whole: true, asks: false, lands_itself: false }));
        self.consumers.len() - 1
    }

    pub(crate) fn leave(&mut self, id: usize) {
        if let Some(seat) = self.consumers[id].take() {
            self.lent.extend(seat.own.into_iter().map(|(_, step)| step));
        }
        self.trim();
    }

    /// The steps made for `reader` and not yet given to a consumer are let go.
    pub(crate) fn forget(&mut self, reader: Reader) {
        let (gone, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut self.orphans).into_iter().partition(|(held, _, _)| *held == reader);
        self.orphans = kept;
        self.lent.extend(gone.into_iter().map(|(_, _, step)| step));
    }

    pub(crate) fn lands_itself(&mut self, id: usize) {
        if let Some(seat) = self.consumers.get_mut(id).and_then(Option::as_mut) {
            seat.lands_itself = true;
        }
    }

    /// Consumer `id` says where its frames stand.
    pub(crate) fn asks(&mut self, id: usize) {
        if let Some(seat) = self.consumers.get_mut(id).and_then(Option::as_mut) {
            seat.asks = true;
        }
    }

    pub(crate) fn reader_of(&self, id: usize) -> Option<Reader> {
        self.consumers.get(id)?.as_ref().map(|seat| seat.reader)
    }

    /// The hour consumer `id` stands at by the steps it took: before the first shared step it has not
    /// taken, or the newest step's last. None when parked, left short by its last step, or standing
    /// before a reset.
    pub(crate) fn stands_at(&self, id: usize) -> Option<Hour> {
        let seat = self.consumers.get(id)?.as_ref().filter(|seat| seat.parked.is_none() && seat.whole)?;
        match self.built.get(next_of(&self.built, seat.next)) {
            Some(step) => (!step.reset).then(|| step.first - 1),
            None => self.published,
        }
    }

    /// The consumers holding `step`, made for them alone.
    pub(crate) fn holding(&self, step: &Arc<Step>) -> Vec<usize> {
        self.consumers.iter().enumerate().filter(|(_, seat)| seat.as_ref().is_some_and(|seat| seat.own.iter().any(|(_, held)| Arc::ptr_eq(held, step)))).map(|(id, _)| id).collect()
    }

    /// Consumer `id`, still standing at `hour` and holding `revoke`, takes `step` before anything
    /// else, and `revoke` no more. Given back otherwise.
    pub(crate) fn land_standing(&mut self, id: usize, hour: Hour, step: Arc<Step>, revoke: &Arc<Step>) -> Result<(), Arc<Step>> {
        if self.stands_at(id) != Some(hour) {
            return Err(step);
        }
        let Some(seat) = self.consumers[id].as_mut() else { return Err(step) };
        let Some(at) = seat.own.iter().position(|(_, held)| Arc::ptr_eq(held, revoke)) else { return Err(step) };
        self.lent.extend(seat.own.remove(at).map(|(_, held)| held));
        seat.own.push_front((seat.next, step));
        Ok(())
    }

    /// The hour parked consumer `id` stood at.
    pub(crate) fn parked(&self, id: usize) -> Option<Hour> {
        self.consumers.get(id)?.as_ref()?.parked
    }

    /// Every parked consumer, its reader and the hour it stood at.
    pub(crate) fn parked_seats(&self) -> Vec<(usize, Reader, Hour)> {
        self.consumers.iter().enumerate().filter_map(|(id, seat)| seat.as_ref().and_then(|seat| Some((id, seat.reader, seat.parked?)))).collect()
    }

    /// Where the frames are, for parked consumer `id`: the hour the consumer furthest on stands at
    /// (with none, the newest step's), no later than the end of the piece it stood in.
    pub(crate) fn catch_hour(&self, id: usize) -> Option<Hour> {
        let from = self.parked(id)?;
        let hour = match self.furthest() {
            Some(next) => self.built.get(next_of(&self.built, next)).map(|step| step.first - 1).or(self.published)?,
            None => self.published?,
        };
        Some(self.piece_end(from).map_or(hour, |step| hour.min(step.first - 1)))
    }

    /// Parked consumer `id` takes `steps` first, then the steps built after `hour`: from the first
    /// step built past it (after any of no hours ending there, which caught a table up to it), or
    /// from the next built when `hour` is the newest step's last. Given back when neither holds.
    pub(crate) fn land_at(&mut self, id: usize, hour: Hour, steps: Vec<Arc<Step>>) -> Result<(), Vec<Arc<Step>>> {
        let position = match self.built.iter().find(|step| step.last > hour) {
            Some(step) if step.first == hour + 1 => step.seq,
            None if self.published == Some(hour) => self.next_seq,
            _ => return Err(steps),
        };
        let Some(seat) = self.consumers.get_mut(id).and_then(Option::as_mut).filter(|seat| seat.parked.is_some()) else { return Err(steps) };
        (seat.next, seat.parked) = (position, None);
        seat.own.extend(steps.into_iter().map(|step| (position, step)));
        self.park_behind();
        self.trim();
        Ok(())
    }

    /// The steps consumer `id` has not taken whose last hour `head` has reached, in order: the
    /// shared ones and those made for it alone, each of these once every shared step before its
    /// place is taken.
    pub(crate) fn take(&mut self, id: usize, head: Hour, out: &mut Vec<Arc<Step>>) -> usize {
        let Some(seat) = self.consumers[id].as_mut().filter(|seat| seat.parked.is_none()) else { return 0 };
        let mut taken = 0;
        let mut at = next_of(&self.built, seat.next);
        loop {
            if let Some((before, step)) = seat.own.front()
                && *before <= seat.next
            {
                if step.last > head {
                    break;
                }
                seat.whole = whole(step);
                out.push(Arc::clone(step));
                self.lent.push(seat.own.pop_front().expect("held").1);
                taken += 1;
                continue;
            }
            let Some(step) = self.built.get(at) else { break };
            if step.last > head {
                break;
            }
            self.taken = self.taken.max(Some(step.last));
            seat.whole = whole(step);
            out.push(Arc::clone(step));
            seat.next = step.seq + 1;
            at += 1;
            taken += 1;
        }
        if taken > 0 {
            self.park_behind();
            self.trim();
        }
        taken
    }

    /// The steps consumer `id` has not taken, in the order its takes bring them, each shared and
    /// none taken by looking.
    pub(crate) fn ahead(&self, id: usize, out: &mut Vec<Arc<Step>>) {
        let Some(seat) = self.consumers[id].as_ref().filter(|seat| seat.parked.is_none()) else { return };
        let mut next = seat.next;
        let mut own = seat.own.iter().peekable();
        for step in self.built.range(next_of(&self.built, next)..) {
            while let Some((_, held)) = own.next_if(|(before, _)| *before <= next) {
                out.push(Arc::clone(held));
            }
            out.push(Arc::clone(step));
            next = step.seq + 1;
        }
        out.extend(own.map(|(_, held)| Arc::clone(held)));
    }

    /// Whether a step consumer `id` has not taken ends at or before `head`.
    pub(crate) fn reaches(&self, id: usize, head: Hour) -> bool {
        let Some(seat) = self.consumers[id].as_ref().filter(|seat| seat.parked.is_none()) else { return false };
        match seat.own.front().filter(|(before, _)| *before <= seat.next) {
            Some((_, step)) => step.last <= head,
            None => self.built.get(next_of(&self.built, seat.next)).is_some_and(|step| step.last <= head),
        }
    }

    /// Where the frames are: the place after every shared step any consumer took, and never before
    /// the epoch's reset; and the last hour of the step before that place. None before the reset.
    pub(crate) fn frames(&self) -> Option<(u64, Hour)> {
        let (reset, hour) = self.reset?;
        let position = self.furthest().unwrap_or(0).max(reset + 1);
        if position == reset + 1 {
            return Some((position, hour));
        }
        let before = next_of(&self.built, position).checked_sub(1).and_then(|at| self.built.get(at)).filter(|step| step.seq + 1 == position);
        Some((position, before.map(|step| step.last).or(self.taken).unwrap_or(hour)))
    }

    /// [`Self::frames`] and every step built past that place, unless one is a reset.
    pub(crate) fn plan(&self, epoch: u64) -> Option<(u64, Hour, Vec<Span>)> {
        if epoch != self.epoch {
            return None;
        }
        let (position, hour) = self.frames()?;
        let past = self.built.range(next_of(&self.built, position)..);
        if past.clone().any(|step| step.reset) {
            return None;
        }
        Some((position, hour, past.map(|step| Span { seq: step.seq, first: step.first, last: step.last, gap: step.piece.is_none() }).collect()))
    }

    /// Each step `seq` takes the change sets given (replacing those of the same tables, given back).
    /// No consumer has taken it; one that only looked at it ([`Self::ahead`]) keeps what it was
    /// lent: the ring holds a copy changed in its place.
    pub(crate) fn complete(&mut self, sets: Vec<(u64, Vec<Changes>)>) -> Vec<Changes> {
        let mut replaced = Vec::new();
        for (seq, set) in sets {
            let Ok(at) = self.built.binary_search_by_key(&seq, |step| step.seq) else {
                replaced.extend(set);
                continue;
            };
            let step = &mut self.built[at];
            if Arc::get_mut(step).is_none() {
                *step = Arc::new(step.copied());
            }
            let step = Arc::get_mut(step).expect("a step no consumer took is held by the ring alone once copied");
            for changes in set {
                match step.tables.binary_search_by_key(&changes.table(), Changes::table) {
                    Ok(place) => replaced.push(std::mem::replace(&mut step.tables[place], changes)),
                    Err(place) => step.tables.insert(place, changes),
                }
            }
        }
        replaced
    }

    /// `steps`, in order, for every consumer of `reader` alone, each handed once every shared step
    /// before `before` is taken; kept for the first consumer of it to join when none has.
    pub(crate) fn give(&mut self, reader: Reader, before: u64, steps: &[Arc<Step>]) {
        let mut given = false;
        for seat in self.consumers.iter_mut().flatten().filter(|seat| seat.reader == reader) {
            let at = place(&seat.own, before);
            for (offset, step) in steps.iter().enumerate() {
                seat.own.insert(at + offset, (before, Arc::clone(step)));
            }
            given = true;
        }
        if !given {
            self.orphans.extend(steps.iter().map(|step| (reader, before, Arc::clone(step))));
        }
    }

    /// Whether `step`, made for one consumer alone, is still to be taken by a consumer of its reader.
    pub(crate) fn holds_own(&self, step: &Arc<Step>) -> bool {
        self.consumers.iter().flatten().any(|seat| seat.own.iter().any(|(_, held)| Arc::ptr_eq(held, step))) || self.orphans.iter().any(|(_, _, held)| Arc::ptr_eq(held, step))
    }

    /// Consumer `id` passes over every step built and every step made for it alone; the next it
    /// takes is `step`, once its head reaches it, then the steps built after.
    pub(crate) fn jump(&mut self, id: usize, step: Arc<Step>) {
        let next = self.next_seq;
        let Some(seat) = self.consumers[id].as_mut() else {
            self.lent.push(step);
            return;
        };
        (seat.next, seat.parked) = (next, None);
        self.lent.extend(seat.own.drain(..).map(|(_, held)| held));
        seat.own.push_back((next, step));
        self.park_behind();
        self.trim();
    }

    /// Every step built from `position` on (none taken by any consumer) leaves: the stepper builds
    /// them again from the hour before it. A step made for one consumer alone placed past it (a
    /// landing at the newest step built) leaves too, as does the place of a consumer that landed
    /// there: the readers and the tables of those steps, and the consumers that landed, are given
    /// back to be landed again.
    pub(crate) fn cut(&mut self, position: u64) -> (Vec<(Reader, Vec<usize>)>, Vec<usize>) {
        let at = next_of(&self.built, position);
        self.lent.extend(self.built.drain(at..));
        let (mut owed, mut landing) = (Vec::new(), Vec::new());
        let tables = |step: &Step| step.tables.iter().map(Changes::table).collect::<Vec<_>>();
        for (id, seat) in self.consumers.iter_mut().enumerate() {
            let Some(seat) = seat.as_mut().filter(|seat| seat.parked.is_none()) else { continue };
            if seat.next > position {
                seat.next = position;
                landing.push(id);
                self.lent.extend(seat.own.drain(..).map(|(_, step)| step));
                continue;
            }
            let kept = place(&seat.own, position);
            for (_, step) in seat.own.drain(kept..) {
                owed.push((seat.reader, tables(&step)));
                self.lent.push(step);
            }
        }
        let (gone, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut self.orphans).into_iter().partition(|(_, before, _)| *before > position);
        self.orphans = kept;
        for (reader, _, step) in gone {
            owed.push((reader, tables(&step)));
            self.lent.push(step);
        }
        (owed, landing)
    }

    /// The seq the next step built will have.
    pub(crate) fn next_seq(&self) -> u64 {
        self.next_seq
    }

    /// Steps every consumer has taken leave the ring. With no consumer, none does.
    fn trim(&mut self) {
        let Some(least) = self.consumers.iter().flatten().map(|seat| self.holds_from(seat)).min() else { return };
        while self.built.front().is_some_and(|step| step.seq < least) {
            self.lent.push(self.built.pop_front().expect("held"));
        }
    }

    /// Every step it holds: built, lent, and made for one consumer.
    pub(crate) fn steps(&self) -> Vec<Arc<Step>> {
        let own = self.consumers.iter().flatten().flat_map(|seat| seat.own.iter().map(|(_, step)| step));
        self.built.iter().chain(self.lent.iter()).chain(own).chain(self.orphans.iter().map(|(_, _, step)| step)).cloned().collect()
    }
}
