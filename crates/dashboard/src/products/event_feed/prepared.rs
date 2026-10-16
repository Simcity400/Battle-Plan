//! Feed-only witnessed facts. Topic changes remain in their owning families.
use super::*;
use crate::prepared::series::Accumulator;
use dashboard_base::prepared::kept::Reduced;
use replay::{
    frame::DecodedFrame,
    rows::{
        CountryCounterInputRow, CountryExperienceGainRow, RelationTupleRow, SliceReadRow, TableId,
        TableRows, tables,
    },
};

pub(crate) const FACT_KEYS: [u64; 22] = [
    1,
    2,
    3,
    4,
    5,
    6,
    7,
    8,
    11,
    15,
    22,
    (1 << 63) | 1,
    (1 << 63) | 2,
    (1 << 63) | 3,
    (1 << 63) | 4,
    (1 << 63) | 5,
    (1 << 63) | 6,
    (1 << 63) | 7,
    (1 << 63) | 8,
    (1 << 63) | 11,
    (1 << 63) | 15,
    (1 << 63) | 22,
];

#[derive(Serialize, Deserialize)]
pub(crate) struct Fact {
    pub row: EventFeedRow,
    pub nuke_type: Option<u32>,
}

pub(crate) mod query;
pub(crate) use query::query;

#[cfg(test)]
pub(crate) mod tests;

/// Frames as the feed and the identity history it names countries through
/// read them, for a block's hours kept to be observed later. The countries
/// keep their keys, the owned-state counters their country and count, the
/// experience gains their country and instance, the relations the rows the
/// diplomacy observer reads at any hour and the slice reads those it reads
/// at the frame's hour; every other field of these tables moves hourly and
/// is read by neither. The links, the event packs and the strikes are kept
/// as read: the links are told apart by allocation, the packs and strikes
/// are this hour's. Kept hours whose reduced rows are equal share one view.
pub(crate) struct KeptFrames {
    countries: Reduced<replay::rows::CountryRow>,
    counters: Reduced<CountryCounterInputRow>,
    gains: Reduced<CountryExperienceGainRow>,
    relations: Reduced<RelationTupleRow>,
    slices: Reduced<SliceReadRow>,
}

impl Default for KeptFrames {
    fn default() -> Self {
        Self {
            countries: Reduced::new(TableId::Countries),
            counters: Reduced::new(TableId::CountryCounterInputs),
            gains: Reduced::new(TableId::CountryExperienceGains),
            relations: Reduced::new(TableId::RelationTuples),
            slices: Reduced::new(TableId::SliceReads),
        }
    }
}

impl KeptFrames {
    /// `frame` as the feed and the identity history read it.
    pub(crate) fn kept(&mut self, frame: &DecodedFrame) -> DecodedFrame {
        let mut kept = frame.clone();
        self.countries.countries(&mut kept);
        let counters = frame
            .complete::<tables::CountryCounterInputs>()
            .ok()
            .map(|rows| {
                rows.iter()
                    .map(|row| CountryCounterInputRow {
                        country: row.country,
                        owned_state_count: row.owned_state_count,
                        ..Default::default()
                    })
                    .collect()
            });
        self.counters
            .apply(&mut kept, counters, TableRows::CountryCounterInputs);
        let gains = frame
            .complete::<tables::CountryExperienceGains>()
            .ok()
            .map(|rows| {
                rows.iter()
                    .map(|row| CountryExperienceGainRow {
                        country: row.country,
                        instance: row.instance,
                        ..Default::default()
                    })
                    .collect()
            });
        self.gains
            .apply(&mut kept, gains, TableRows::CountryExperienceGains);
        let relations = frame.complete::<tables::RelationTuples>().ok().map(|rows| {
            rows.iter()
                .filter(|row| super::diplomacy::reads_relation(row))
                .cloned()
                .collect()
        });
        self.relations
            .apply(&mut kept, relations, TableRows::RelationTuples);
        let slices = frame.complete::<tables::SliceReads>().ok().map(|rows| {
            rows.iter()
                .filter(|read| super::diplomacy::reads_slice(read, frame.hour))
                .cloned()
                .collect()
        });
        self.slices.apply(&mut kept, slices, TableRows::SliceReads);
        kept
    }
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Observer {
    native: native::Projection,
    diplomacy: diplomacy::Observer,
    events: Accumulator,
    late: crate::prepared::series::LateIndex,
    diagnostics: Builder,
    observed: BTreeSet<String>,
}
impl Default for Observer {
    fn default() -> Self {
        Self {
            native: Default::default(),
            diplomacy: Default::default(),
            events: Default::default(),
            late: Default::default(),
            diagnostics: Builder::new(0, 0),
            observed: BTreeSet::new(),
        }
    }
}
impl Observer {
    pub(crate) fn observe(
        &mut self,
        replay: &replay::container::Replay,
        frame: &DecodedFrame,
        identities: &crate::products::country_history::HistoryScan,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        check_cancel(cancelled)?;
        let start = replay
            .branches
            .iter()
            .find(|start| start.id == frame.branch)
            .ok_or_else(|| crate::invalid("Event feed branch is not recorded"))?;
        let (first, last) = branch_bounds(replay, start);
        self.diagnostics.feed.first = first;
        self.diagnostics.feed.last = last;
        self.native.losses(replay, frame.branch);
        self.events.through(frame.hour);
        let tag_at = |hour, country| identities.native_tag(hour, country);
        let pack = frame.complete::<tables::EventFeedHistory>();
        let read = match pack {
            Ok([pack]) if pack.read_hour == frame.hour => {
                self.native
                    .append_named(&mut self.diagnostics, pack, tag_at, cancelled)
            }
            Ok(_) => Err(crate::invalid(
                "Event history has no unique pack for its supplying hour.",
            )),
            Err(reason) => Err(reason),
        };
        if let Err(reason) = read {
            check_cancel(cancelled)?;
            self.diagnostics.sampled_unavailable(
                "nativeEventCoverage",
                "Native event coverage",
                reason.to_string(),
                1,
            );
        }
        self.diagnostics.feed.rows.sort_by(row_order);
        for row in self.diagnostics.feed.rows.drain(..) {
            let key = (row.kind as u64) | (u64::from(row.read_hour > row.hour) << 63);
            let ordinal = self.events.state(key).map_or(0, |state| state.events);
            self.late.insert(row.hour, row.read_hour, key, ordinal);
            self.events.event(
                key,
                row.read_hour,
                &Fact {
                    row,
                    nuke_type: None,
                },
            )?;
        }
        self.diagnostics.war_ends.clear();
        let mut changes = country_groups::changes::Scan {
            moments: Vec::new(),
            observed: BTreeSet::new(),
            unavailable: Vec::new(),
        };
        self.diplomacy.observe(frame, replay.format, &mut changes)?;
        self.observed.extend(
            changes
                .observed
                .into_iter()
                .map(|table| format!("{table:?}")),
        );
        for refusal in changes.unavailable {
            self.diagnostics.unavailable(
                &refusal.key,
                &refusal.label,
                refusal.reason,
                Some(
                    u32::try_from(refusal.count)
                        .map_err(|_| crate::invalid("Diplomacy refusal count exceeds u32"))?,
                ),
            );
        }
        for change in changes.moments {
            group_row(&mut self.diagnostics, &change, None)?;
        }
        self.diagnostics.feed.rows.sort_by(row_order);
        for mut row in self.diagnostics.feed.rows.drain(..) {
            row.id.clear(); // The shared reader's ordinal owns non-native row identities.
            let key = (row.kind as u64) | (u64::from(row.read_hour > row.hour) << 63);
            let ordinal = self.events.state(key).map_or(0, |state| state.events);
            self.late.insert(row.hour, row.read_hour, key, ordinal);
            self.events.event(
                key,
                row.read_hour,
                &Fact {
                    row,
                    nuke_type: None,
                },
            )?;
        }
        let mut strikes = Vec::new();
        let type_operands =
            country_nukes::native_type_info(&country_nukes::ScannedNukes::default(), |_| None)
                .into_iter()
                .map(|info| (info.ordinal, info.ordinal))
                .collect();
        for strike in nuclear::read(frame, &mut self.diagnostics, cancelled)? {
            // The type is an operand, not an item identity; its family names it at query time.
            nuclear::project(
                frame.hour,
                &strike,
                tag_at,
                &mut self.diagnostics,
                &type_operands,
            );
            for mut row in self.diagnostics.feed.rows.drain(..) {
                row.item = None;
                strikes.push(Fact {
                    row,
                    nuke_type: Some(strike.nuke_type),
                });
            }
        }
        strikes.sort_by(|a, b| row_order(&a.row, &b.row));
        for strike in strikes {
            let key = (strike.row.kind as u64)
                | (u64::from(strike.row.read_hour > strike.row.hour) << 63);
            let ordinal = self.events.state(key).map_or(0, |state| state.events);
            self.late
                .insert(strike.row.hour, strike.row.read_hour, key, ordinal);
            self.events.event(key, strike.row.read_hour, &strike)?;
        }
        Ok(())
    }
    pub(crate) fn bytes(&self) -> usize {
        self.events.bytes()
    }
    pub(crate) fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.events
            .flush(writer, &format!("event-feed-facts-{branch}"), 0, cancelled)?;
        crate::prepared::series::write_catalogue(
            writer,
            "event-feed-facts",
            branch,
            &BTreeMap::new(),
            &(&self.diagnostics, &self.observed, &self.late),
        )
    }
}
