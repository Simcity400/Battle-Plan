//! A country's whole-branch production scan as it is persisted ([`SavedLines`]).
//!
//! Every value of [`ScannedLines`] is kept but the handles its modifier runs hold
//! of the content they were read with (its localisation) and of the recording (its
//! language). Replay rows keep the recording's own row encoding, one row at a
//! time, and a country's modifier row that several runs share is kept once.

use super::{RawRun, ScannedLine, ScannedLines};
use crate::products::{country_groups::HourSpan, equipment::VariantCatalogue};
use replay_data::rows::{CountryProductionModifiersRow, TableRows};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

/// The saved layout's version.
pub const SCHEMA: u32 = 5;

#[cfg(test)]
thread_local! {
    /// Country rows looked up while saving on this thread, and rows encoded.
    pub(super) static SAVE_WORK: std::cell::Cell<(usize, usize)> =
        const { std::cell::Cell::new((0, 0)) };
}

/// One modifier run as it is persisted.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct SavedModifierRun {
    pub first_hour: i32,
    pub last_hour: i32,
    /// Its country's modifier row, by its place in [`SavedLines::countries`].
    pub country: usize,
    /// Its line's modifier operands, in the recording's row encoding.
    pub line: Vec<u8>,
    pub run: RawRun,
    pub organisation: Option<String>,
}

/// A whole-branch scan as it is persisted.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SavedLines {
    pub schema: u32,
    pub lines: Vec<ScannedLine>,
    pub designs: BTreeMap<u64, Vec<super::DesignRead>>,
    pub modifiers: BTreeMap<String, Vec<SavedModifierRun>>,
    /// The countries' modifier rows the runs name, each once, in the
    /// recording's row encoding.
    pub countries: Vec<Vec<u8>>,
    /// The scan read the content's localisation for its modifier runs.
    pub localised: bool,
    pub modifier_layout_refusals: BTreeMap<String, usize>,
    pub variants: VariantCatalogue,
    pub icons: BTreeMap<u64, Vec<super::IconAt>>,
    pub organisations: BTreeMap<u64, Vec<super::RawOrganisation>>,
    pub reads: Vec<i32>,
    pub output_hours: Vec<HourSpan>,
    pub line_hours: Vec<HourSpan>,
    pub organisation_hours: Vec<HourSpan>,
    pub unknown_kinds: usize,
    pub unlisted_output: usize,
    pub unread_output_reads: usize,
    pub unreadable_stretches: usize,
    pub unlinked_rows: usize,
    pub unread_output: Vec<HourSpan>,
}

pub(super) fn encode(rows: TableRows) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    rows.encode_rows(&mut bytes)?;
    Ok(bytes)
}

fn invalid(what: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("saved production scan: {what}"),
    )
}

impl SavedLines {
    /// `scanned` as it is persisted. Each run finds its country's row by the
    /// allocation it shares in one lookup, so the work is one step per run.
    pub fn save(scanned: &ScannedLines, cancelled: &AtomicBool) -> io::Result<Self> {
        let mut countries: Vec<Vec<u8>> = Vec::new();
        // Each shared row once, by the allocation the runs share.
        let mut placed: HashMap<*const CountryProductionModifiersRow, usize> = HashMap::new();
        let mut localised = None;
        let mut modifiers = BTreeMap::new();
        for (key, runs) in &scanned.modifiers {
            if cancelled.load(Ordering::Acquire) {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "Production request cancelled",
                ));
            }
            let mut saved = Vec::with_capacity(runs.len());
            for run in runs {
                #[cfg(test)]
                SAVE_WORK.with(|work| work.set((work.get().0 + 1, work.get().1)));
                let country = match placed.entry(Arc::as_ptr(&run.country)) {
                    std::collections::hash_map::Entry::Occupied(at) => *at.get(),
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        #[cfg(test)]
                        SAVE_WORK.with(|work| work.set((work.get().0, work.get().1 + 1)));
                        countries.push(encode(TableRows::CountryProductionModifiers(vec![
                            (*run.country).clone(),
                        ]))?);
                        *entry.insert(countries.len() - 1)
                    }
                };
                // One scan reads one content: every run is localised alike.
                match localised {
                    None => localised = Some(run.localisation.is_some()),
                    Some(known) if known != run.localisation.is_some() => {
                        return Err(invalid("modifier runs read different content"));
                    }
                    Some(_) => {}
                }
                saved.push(SavedModifierRun {
                    first_hour: run.first_hour,
                    last_hour: run.last_hour,
                    country,
                    line: encode(TableRows::ProductionLineModifiers(vec![run.line.clone()]))?,
                    run: run.run,
                    organisation: run.organisation.clone(),
                });
            }
            modifiers.insert(key.clone(), saved);
        }
        Ok(Self {
            schema: SCHEMA,
            lines: scanned.lines.clone(),
            designs: scanned.designs.clone(),
            modifiers,
            countries,
            localised: localised.unwrap_or(false),
            modifier_layout_refusals: scanned.modifier_layout_refusals.clone(),
            variants: scanned.variants.clone(),
            icons: scanned.icons.clone(),
            organisations: scanned.organisations.clone(),
            reads: scanned.reads.clone(),
            output_hours: scanned.output_hours.clone(),
            line_hours: scanned.line_hours.clone(),
            organisation_hours: scanned.organisation_hours.clone(),
            unknown_kinds: scanned.unknown_kinds,
            unlisted_output: scanned.unlisted_output,
            unread_output_reads: scanned.unread_output_reads,
            unreadable_stretches: scanned.unreadable_stretches,
            unlinked_rows: scanned.unlinked_rows,
            unread_output: scanned.unread_output.clone(),
        })
    }
}
