//! The steps built and not yet taken by every consumer, the steps consumers still hold, and the
//! steps made for one consumer alone. A step is shared, never copied: each consumer takes its own
//! reference, and a step comes back to the stepper once only the ring holds it.

use crate::{changes::Changes, playhead::Reader, step::Step};
use replay_data::Hour;
use std::{collections::VecDeque, sync::Arc};

/// One consumer: its reader, its next shared step, and the steps made for it alone, each handed
/// once every shared step before `before` is taken.
struct Seat {
    reader: Reader,
    next: u64,
    own: VecDeque<(u64, Arc<Step>)>,
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
        Self { epoch: 0, size, next_seq: 0, built: VecDeque::new(), lent: Vec::new(), consumers: Vec::new(), orphans: Vec::new(), taken: None, reset: None }
    }

    /// Fewer than its size of the steps built end past `head`, and fewer than its size are not yet
    /// taken by the consumer furthest on. The head bounds how far ahead of the clock the stepper
    /// builds; the furthest consumer bounds how far ahead of every reader, so a head driven to the
    /// newest step does not chase the stepper. A step the head has passed and a slower consumer has
    /// not taken waits for it, shared, and takes no room: a consumer late to take holds nobody.
    pub(crate) fn room(&self, head: Hour) -> bool {
        self.past(head) < self.size && self.untaken() < self.size
    }

    fn furthest(&self) -> Option<u64> {
        self.consumers.iter().flatten().map(|seat| seat.next).max()
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

    /// Every step of the epoch is abandoned, those made for one consumer included: a new one begins.
    pub(crate) fn abandon(&mut self) -> u64 {
        self.epoch += 1;
        self.lent.extend(self.built.drain(..));
        (self.taken, self.reset) = (None, None);
        let next = self.next_seq;
        for seat in self.consumers.iter_mut().flatten() {
            seat.next = next;
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
        self.consumers.push(Some(Seat { reader, next: start, own }));
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

    pub(crate) fn reader_of(&self, id: usize) -> Option<Reader> {
        self.consumers.get(id)?.as_ref().map(|seat| seat.reader)
    }

    /// The steps consumer `id` has not taken whose last hour `head` has reached, in order: the
    /// shared ones and those made for it alone, each of these once every shared step before its
    /// place is taken.
    pub(crate) fn take(&mut self, id: usize, head: Hour, out: &mut Vec<Arc<Step>>) -> usize {
        let Some(seat) = self.consumers[id].as_mut() else { return 0 };
        let mut taken = 0;
        let mut at = next_of(&self.built, seat.next);
        loop {
            if let Some((before, step)) = seat.own.front()
                && *before <= seat.next
            {
                if step.last > head {
                    break;
                }
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
            out.push(Arc::clone(step));
            seat.next = step.seq + 1;
            at += 1;
            taken += 1;
        }
        if taken > 0 {
            self.trim();
        }
        taken
    }

    /// The steps consumer `id` has not taken, in the order its takes bring them, each shared and
    /// none taken by looking.
    pub(crate) fn ahead(&self, id: usize, out: &mut Vec<Arc<Step>>) {
        let Some(seat) = &self.consumers[id] else { return };
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
        let Some(seat) = &self.consumers[id] else { return false };
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

    /// `step`, made for one consumer alone and not yet taken, is withdrawn.
    pub(crate) fn revoke(&mut self, step: &Arc<Step>) {
        for seat in self.consumers.iter_mut().flatten() {
            if let Some(at) = seat.own.iter().position(|(_, held)| Arc::ptr_eq(held, step)) {
                self.lent.extend(seat.own.remove(at).map(|(_, held)| held));
            }
        }
        if let Some(at) = self.orphans.iter().position(|(_, _, held)| Arc::ptr_eq(held, step)) {
            self.lent.push(self.orphans.remove(at).2);
        }
    }

    /// Consumer `id` passes over every step built and every step made for it alone; the next it
    /// takes is `step`, once its head reaches it, then the steps built after.
    pub(crate) fn jump(&mut self, id: usize, step: Arc<Step>) {
        let next = self.next_seq;
        let Some(seat) = self.consumers[id].as_mut() else {
            self.lent.push(step);
            return;
        };
        seat.next = next;
        self.lent.extend(seat.own.drain(..).map(|(_, held)| held));
        seat.own.push_back((next, step));
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
            let Some(seat) = seat else { continue };
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
        let Some(least) = self.consumers.iter().flatten().map(|seat| seat.next).min() else { return };
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
