//! Naval losses on the naval family's series: each channel's values and ranks as runs, each
//! corrected holder's corrections and the sunk ships' and convoys' reads as events, and what
//! the replies name besides ([`Catalogue`]) as a scalar. Every key is the family's, its
//! dimension the key the losses name it by.

use super::{Correction, Metadata, Prepared};
use crate::products::sinkings;
use dashboard_series::{
    Cell, Event, Evidence, KeyHandle, Knownness, RunMode, Span, Writer,
    carry::Carry,
    family::{self, Actors, Directory, Place, Scalar},
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io, sync::atomic::AtomicBool};

/// The family's keys are spelled under it.
pub const SCHEMA: u64 = 1;
/// The actor of every key.
pub const FAMILY: u64 = u64::MAX;
/// Measures: channel `c`'s values are `VALUES + c` and its ranks `RANKS + c`.
pub const VALUES: u64 = 0;
pub const RANKS: u64 = 12;
pub const CORRECTIONS: u64 = 24;
pub const READS: u64 = 25;
/// The scalar holding the [`Catalogue`].
pub const CATALOGUE: u64 = 0;
/// Every key's evidence: each prepared hour is known.
const DOMAIN: u64 = 0;

/// Event kinds.
pub mod kind {
    /// `[Integer read hour, Integer month | Null, Integer ships, Integer convoys,
    /// Integer ship unknown, Integer convoy unknown]`, on a holder's CORRECTIONS key.
    pub const CORRECTION: u64 = 0;
    /// No fields, at the read's hour, on READS 0 (ships) or 1 (convoys).
    pub const READ: u64 = 1;
}

/// What the replies name besides the series: the holders by identity, and the metadata.
#[derive(Serialize, Deserialize)]
pub(super) struct Catalogue {
    pub(super) actors: BTreeMap<String, u64>,
    pub(super) metadata: Metadata,
}

/// The catalogue `directory` holds; None before the family's first cut wrote one.
pub(super) fn catalogue(directory: &Directory) -> io::Result<Option<Catalogue>> {
    let Some(scalar) = directory.scalar(CATALOGUE) else {
        return Ok(None);
    };
    let text = scalar
        .text()
        .ok_or_else(|| dashboard_core::invalid("Naval losses catalogue is not text"))?;
    Ok(Some(serde_json::from_slice(text)?))
}

/// The emit of naval losses on one branch: the keys it wrote, each one's open run, and the
/// first hour since its last cut.
pub struct Emit {
    actors: Actors,
    carry: Carry,
    /// The contiguous stretches of hours handed since the last cut.
    spans: Vec<Span>,
}

impl Default for Emit {
    fn default() -> Self {
        Self {
            actors: Actors::new(SCHEMA),
            carry: Carry::default(),
            spans: Vec::new(),
        }
    }
}

impl Emit {
    /// What it goes on with after a cut: each key's open run. Its keys are the directory's.
    pub fn save(&self, out: &mut Vec<u8>) -> io::Result<()> {
        if !self.spans.is_empty() {
            return Err(dashboard_core::invalid(
                "Naval losses are saved only after a cut",
            ));
        }
        self.carry.save(out)
    }

    /// The emit [`Self::save`] saved, going on with the publication `directory` reads.
    pub fn resume(directory: &Directory, saved: &[u8]) -> io::Result<Self> {
        Ok(Self {
            actors: Actors::resume(SCHEMA, directory)?,
            carry: Carry::load(saved)?,
            spans: Vec::new(),
        })
    }

    fn key(
        &mut self,
        writer: &mut Writer,
        measure: u64,
        dimension: u64,
    ) -> io::Result<KeyHandle> {
        self.actors.key(
            writer,
            Place {
                actor: FAMILY,
                measure,
                dimension: Some(dimension),
            },
            (RunMode::Exact, 0),
            Some(DOMAIN),
        )
    }

    /// Appends what `prepared` wrote at `hour`: its values' changes, corrections and reads.
    pub fn hour(
        &mut self,
        writer: &mut Writer,
        prepared: &mut Prepared,
        hour: i32,
    ) -> io::Result<()> {
        match self.spans.last_mut() {
            Some(span) if span.last.checked_add(1) == Some(hour) => span.last = hour,
            Some(span) if span.last >= hour => {
                return Err(dashboard_core::invalid(
                    "Naval losses were handed an hour twice",
                ));
            }
            _ => self.spans.push(Span::new(hour, hour)?),
        }
        for (index, channel) in prepared.channels.iter_mut().enumerate() {
            for (dimension, at, value, starts) in std::mem::take(&mut channel.values.written) {
                let key = self.key(writer, VALUES + index as u64, dimension)?;
                self.carry.value(writer, key, at, value, starts)?;
            }
        }
        for (dimension, at, correction) in std::mem::take(&mut prepared.corrections) {
            let key = self.key(writer, CORRECTIONS, dimension)?;
            let Correction {
                read_hour,
                month,
                delta,
            } = correction;
            let fields = writer.cells([
                Cell::Integer(i128::from(read_hour)),
                month.map_or(Cell::Null, |month| Cell::Integer(i128::from(month))),
                Cell::Integer(i128::from(delta.ships)),
                Cell::Integer(i128::from(delta.convoys)),
                Cell::Integer(i128::from(delta.ship_unknown)),
                Cell::Integer(i128::from(delta.convoy_unknown)),
            ])?;
            let sequence = writer.sequence()?;
            writer.event(Event {
                key,
                native_hour: at,
                read_hour: None,
                sequence,
                kind: kind::CORRECTION,
                fields,
            })?;
        }
        for (dimension, at) in std::mem::take(&mut prepared.reads) {
            let key = self.key(writer, READS, dimension)?;
            let fields = writer.cells([])?;
            let sequence = writer.sequence()?;
            writer.event(Event {
                key,
                native_hour: at,
                read_hour: None,
                sequence,
                kind: kind::READ,
                fields,
            })?;
        }
        Ok(())
    }

    /// Appends the ranks since the last cut and what is open through `last`, the hours since
    /// the last cut as known, and the catalogue.
    pub fn cut(
        &mut self,
        writer: &mut Writer,
        prepared: &mut Prepared,
        scan: &sinkings::Scanned,
        last: i32,
        flag: &AtomicBool,
    ) -> io::Result<()> {
        for (index, ranks) in prepared.ranks(flag)?.into_iter().enumerate() {
            for (dimension, first, through, value) in ranks {
                let key = self.key(writer, RANKS + index as u64, dimension)?;
                writer.run(key, Span::new(first, through)?, value, false, false)?;
            }
        }
        self.carry.cut(writer, last)?;
        if self.spans.is_empty() {
            return Ok(());
        }
        for span in std::mem::take(&mut self.spans) {
            writer.domain(
                DOMAIN,
                Evidence {
                    span,
                    known: Knownness::Known,
                    supplying_read: None,
                    holder: None,
                    identity_first: None,
                },
            )?;
        }
        let catalogue = Catalogue {
            actors: prepared.actors.clone(),
            metadata: prepared.metadata(scan),
        };
        let cell = family::cell(
            writer,
            &Scalar::Text(serde_json::to_vec(&catalogue)?.into()),
        )?;
        writer.catalogue(CATALOGUE, cell)
    }
}
