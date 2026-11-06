//! The peace observer as the segment driver's emit steps it ([`super::emit`]): each completed
//! deal and finalizer outcome leaves it as an event, and what it goes on with is its
//! continuation, saved at each cut. A reply joins the events with the continuation.
use super::{reads::HourReads, *};
use dashboard_base::prepared::series::{compress, decompress};

#[derive(Serialize, Deserialize)]
pub struct Prepared {
    scan: Scan,
    ordinals: Vec<u64>,
    next_ordinal: u64,
}

// External variant tags preserve JSON's typed numeric map-key decoding for
// finalizer country references and victory-point maps.
#[derive(Serialize, Deserialize)]
pub(super) enum Event {
    Deal { ordinal: u64, deal: PeaceDeal },
    Finalizer { capture: exact::Capture },
}

impl Prepared {
    pub fn new(replay: &Replay, branch: u64) -> io::Result<Self> {
        let start = replay
            .branches
            .iter()
            .find(|start| start.id == branch)
            .ok_or_else(|| dashboard_core::invalid("Peace-deals branch is not in this recording"))?;
        let (first, last) = branch_bounds(replay, start);
        Ok(Self {
            scan: Scan::new(branch, first, last, replay.format.version()),
            ordinals: Vec::new(),
            next_ordinal: 0,
        })
    }

    /// `frame` observed as the row path reads it, the roster's slot read when `read`.
    #[cfg(test)]
    pub(super) fn observe_frame(
        &mut self,
        frame: &DecodedFrame,
        read: bool,
        events: &mut Vec<Event>,
    ) -> io::Result<()> {
        let reads = self
            .scan
            .reader
            .read(frame, &[true; replay_data::rows::TABLE_COUNT])?;
        self.observe_changed(frame.hour, reads, read, events)
    }

    /// `hour` observed with the reads it changed, the roster's slot read when `read`; the
    /// deals it completes and the finalizer outcomes it captures go to `events`.
    pub(super) fn observe_changed(
        &mut self,
        hour: i32,
        reads: HourReads,
        read: bool,
        events: &mut Vec<Event>,
    ) -> io::Result<()> {
        let counts = |scan: &Scan| {
            (
                scan.reply.deals.len(),
                scan.open.len(),
                scan.pending.len(),
                scan.processed.len(),
                scan.unassigned.len(),
            )
        };
        let before = counts(&self.scan);
        self.scan.last = self.scan.last.max(hour);
        self.scan.observe_changed(hour, reads, read)?;
        events.extend(
            self.scan
                .exact
                .captures
                .drain(..)
                .map(|capture| Event::Finalizer { capture }),
        );
        if before == counts(&self.scan) {
            return Ok(());
        }
        while self.ordinals.len() < self.scan.reply.deals.len() {
            self.ordinals.push(self.next_ordinal);
            self.next_ordinal += 1;
        }
        for conference in self.scan.open.values_mut() {
            if self.scan.unassigned.contains(&conference.index) {
                conference.before = None;
            }
        }
        for (&index, before) in &mut self.scan.pending {
            if self.scan.unassigned.contains(&index) {
                *before = None;
            }
        }
        let open: HashSet<_> = self
            .scan
            .open
            .values()
            .map(|conference| conference.index)
            .chain(self.scan.pending.keys().copied())
            .collect();
        let mut positions = HashMap::new();
        let mut kept = Vec::new();
        let mut ordinals = Vec::new();
        for (index, deal) in std::mem::take(&mut self.scan.reply.deals)
            .into_iter()
            .enumerate()
        {
            if open.contains(&index) {
                positions.insert(index, kept.len());
                ordinals.push(self.ordinals[index]);
                kept.push(deal);
            } else {
                events.push(Event::Deal {
                    ordinal: self.ordinals[index],
                    deal,
                });
            }
        }
        for conference in self.scan.open.values_mut() {
            conference.index = positions[&conference.index];
        }
        self.scan.pending = std::mem::take(&mut self.scan.pending)
            .into_iter()
            .map(|(index, value)| (positions[&index], value))
            .collect();
        self.scan.processed = self
            .scan
            .processed
            .iter()
            .filter_map(|index| positions.get(index).copied())
            .collect();
        self.scan.unassigned = self
            .scan
            .unassigned
            .iter()
            .filter_map(|index| positions.get(index).copied())
            .collect();
        self.scan.reply.deals = kept;
        self.ordinals = ordinals;
        Ok(())
    }

    /// Hours `first..=last` that continue the last observed hour with no field the scan reads
    /// changing, the roster's slot read at `reads`: [`Self::observe_changed`] for each, at once
    /// ([`Scan::advance_quiet`]); no deal or finalizer event leaves them. False, with nothing
    /// changed, when the hours must be observed one by one.
    pub(super) fn advance_quiet(&mut self, first: i32, last: i32, reads: &[i32]) -> bool {
        if !self.scan.advance_quiet(first, last, reads) {
            return false;
        }
        self.scan.last = self.scan.last.max(last);
        true
    }

    /// What a reply goes on from after the hours observed so far: the scan, its open deals'
    /// ordinals, compressed.
    pub(super) fn continuation(&mut self) -> io::Result<Vec<u8>> {
        self.scan.exact.seal();
        self.scan.seal_unobserved();
        compress(&serde_json::to_vec(&(&self.scan, &self.ordinals))?, 3)
    }
}

/// The reply through `last` from a `continuation` and every event published before it, in the
/// order they left the observer: the branch's last hour, or a prepared prefix's (`prefix`),
/// whose unobserved later hours are pending rather than missing.
pub(super) fn reply(
    continuation: &[u8],
    events: Vec<Event>,
    last: i32,
    prefix: bool,
) -> io::Result<PeaceDealsReply> {
    let (mut scan, ordinals): (Scan, Vec<u64>) =
        serde_json::from_slice(&decompress(continuation)?)?;
    scan.last = last;
    scan.prefix = prefix;
    let mut completed = Vec::new();
    for event in events {
        match event {
            Event::Deal { ordinal, deal } => completed.push((ordinal, deal)),
            Event::Finalizer { capture } => scan.exact.captures.push(capture),
        }
    }
    // The continuation's indices still address its compact vector. Finalization
    // runs before adding completed rows, then the common finalizer joins all
    // conferences and rebuilds every graph exactly as the reference does.
    scan.close_continuation();
    completed.extend(
        ordinals
            .into_iter()
            .zip(std::mem::take(&mut scan.reply.deals)),
    );
    completed.sort_by_key(|(ordinal, _)| *ordinal);
    scan.reply.deals = completed.into_iter().map(|(_, deal)| deal).collect();
    Ok(scan.reply())
}

#[cfg(test)]
mod tests;
