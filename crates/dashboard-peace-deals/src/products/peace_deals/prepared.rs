//! Dated derived deals and finalizer outcomes share one branch event key.
use super::*;
use dashboard_base::prepared::series::{self, Accumulator, Reader};
use std::collections::BTreeMap;

#[derive(Serialize, Deserialize)]
pub struct Prepared {
    scan: Scan,
    events: Accumulator,
    ordinals: Vec<u64>,
    next_ordinal: u64,
    #[cfg(test)]
    #[serde(skip)]
    work: dashboard_base::prepared::observer_tests::Work,
}
// External variant tags preserve JSON's typed numeric map-key decoding for
// finalizer country references and victory-point maps.
#[derive(Serialize, Deserialize)]
enum Event {
    Deal { ordinal: u64, deal: PeaceDeal },
    Finalizer { capture: exact::Capture },
}
fn section(branch: u64) -> String {
    format!("peace-deals-{branch}")
}
fn continuation(branch: u64) -> String {
    format!("peace-deals-{branch}-open")
}

impl Prepared {
    pub fn new(replay: &Replay, branch: u64) -> Self {
        let start = replay
            .branches
            .iter()
            .find(|start| start.id == branch)
            .expect("validated prepared branch");
        let (first, last) = branch_bounds(replay, start);
        Self {
            scan: Scan::new(branch, first, last, replay.format.version()),
            events: Accumulator::default(),
            ordinals: Vec::new(),
            next_ordinal: 0,
            #[cfg(test)]
            work: Default::default(),
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn observe(&mut self, frame: &DecodedFrame) -> io::Result<()> {
        self.observe_changed(
            frame,
            &FrameChanges {
                rows_changed: [true; replay::frame::TABLE_COUNT],
                table_read: [true; replay::frame::TABLE_COUNT],
                slot_read: [true; replay::frame::TABLE_COUNT],
                updates: None,
            },
        )
    }
    pub fn observe_changed(
        &mut self,
        frame: &DecodedFrame,
        changes: &FrameChanges,
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
        self.events.advance(frame.hour)?;
        self.scan.last = self.scan.last.max(frame.hour);
        #[cfg(test)]
        let exact_before = self.scan.exact.history_visits;
        self.scan.observe_changed(frame, changes)?;
        #[cfg(test)]
        {
            self.work.row_visits =
                self.scan.snapshot_inputs.row_visits + self.scan.exact.row_visits;
            self.work.history_visits += self.scan.exact.history_visits - exact_before;
        }
        for capture in self.scan.exact.captures.drain(..) {
            self.events
                .event(0, frame.hour, &Event::Finalizer { capture })?;
        }
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
        #[cfg(test)]
        {
            self.work.history_visits += self.scan.reply.deals.len();
        }
        for (index, deal) in std::mem::take(&mut self.scan.reply.deals)
            .into_iter()
            .enumerate()
        {
            if open.contains(&index) {
                positions.insert(index, kept.len());
                ordinals.push(self.ordinals[index]);
                kept.push(deal);
            } else {
                self.events.event(
                    0,
                    frame.hour,
                    &Event::Deal {
                        ordinal: self.ordinals[index],
                        deal,
                    },
                )?;
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
        self.events.through(frame.hour);
        Ok(())
    }

    /// Hours `first..=last` that continue the last observed hour (`frame`)
    /// with no field the scan reads changing, the roster's slot read at
    /// `reads`: [`Self::observe_changed`] for each, at once
    /// ([`Scan::advance_quiet`]); no deal, comparison or finalizer event
    /// changes in them. False, with nothing changed, when the hours must be
    /// observed on their frames.
    pub fn advance_quiet(
        &mut self,
        frame: &DecodedFrame,
        first: i32,
        last: i32,
        reads: &[i32],
    ) -> io::Result<bool> {
        if !self.scan.advance_quiet(frame, first, last, reads) {
            return Ok(false);
        }
        self.events.advance_span(first, last)?;
        self.scan.last = self.scan.last.max(last);
        Ok(true)
    }

    pub fn buffered_bytes(&self) -> usize {
        self.events.bytes()
    }

    pub fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.scan.exact.seal();
        self.scan.seal_unobserved();
        self.events.flush(writer, &section(branch), 0, cancelled)?;
        series::write_catalogue(
            writer,
            "peace-deals",
            branch,
            &BTreeMap::from([(branch.to_string(), 0)]),
            &(),
        )?;
        series::write_reply(
            writer,
            &continuation(branch),
            0,
            &(&self.scan, &self.ordinals),
        )
    }
}

fn reply(
    store: &store::Store,
    source_key: &store::SourceKey,
    branch: u64,
    last: i32,
    cancelled: &AtomicBool,
) -> io::Result<Option<PeaceDealsReply>> {
    let Some((keys, ())) = series::read_catalogue::<()>(store, source_key, "peace-deals", branch)?
    else {
        return Ok(None);
    };
    let Some((mut scan, ordinals)) =
        series::read_reply::<(Scan, Vec<u64>)>(store, source_key, &continuation(branch), 0)?
    else {
        return Ok(None);
    };
    scan.last = last;
    let mut reader = Reader::new(store, source_key, section(branch), cancelled)?;
    let mut cursor = 0;
    let mut completed = Vec::new();
    loop {
        check_cancel(cancelled)?;
        let page = reader.events(
            &[keys[&branch.to_string()]],
            i32::MIN,
            i32::MAX,
            cursor,
            256,
        )?;
        if page
            .cursor
            .is_some_and(|next| next <= cursor || page.rows.is_empty())
        {
            return Err(dashboard_core::invalid(
                "Peace-deal event page made no progress",
            ));
        }
        for row in page.rows {
            match serde_json::from_value::<Event>(row.row)? {
                Event::Deal { ordinal, deal } => completed.push((ordinal, deal)),
                Event::Finalizer { capture } => scan.exact.captures.push(capture),
            }
        }
        let Some(next) = page.cursor else {
            break;
        };
        cursor = next;
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
    Ok(Some(scan.reply()))
}

pub fn query(
    source: &Source,
    context: &LoadContext,
    params: serde_json::Value,
    store: &store::Store,
    source_key: &store::SourceKey,
    cancelled: &AtomicBool,
) -> io::Result<serde_json::Value> {
    let params: PeaceDealsParams =
        serde_json::from_value(serde_json::json!({ "branchId": params.get("branchId") }))?;
    PeaceDeals::validate(&params).map_err(dashboard_core::invalid)?;
    let branch = params.branch_id.parse().expect("validated peace branch");
    let start = source
        .replay_ref()
        .branches
        .iter()
        .find(|start| start.id == branch)
        .ok_or_else(|| dashboard_core::invalid("Peace-deals branch is not in this recording"))?;
    let (_, last) = branch_bounds(source.replay_ref(), start);
    let Some(reply) = reply(store, source_key, branch, last, cancelled)? else {
        return Ok(serde_json::Value::Null);
    };
    serde_json::to_value(present(
        source,
        branch,
        context,
        cancelled,
        Arc::new(reply),
    )?)
    .map_err(Into::into)
}

pub mod family;

#[cfg(test)]
mod tests;
