//! Air aces prepared block by block, on its own. A pre-pass worker reads a
//! block alone at the hours the fields the ace roster and variant catalogue
//! read change, and at every read of the roster and of the variants (daily),
//! and keeps those hours' frames reduced to what they read
//! ([`dashboard_raids::products::raids::kept`]); the fold observes them again on
//! the carried aces and each quiet hour between on the last kept frame, as
//! hourly preparation does, then publishes the block's window as hourly
//! preparation does at the block's end.

use super::*;
use dashboard_base::prepared::{
    frame_changes::{FrameChangeTracker, FrameChanges},
    prepass::{Extract, Family, Folded},
};
use dashboard_raids::products::raids::kept::{self, Kept, Reduced};
use replay::{
    container::{BlockIndex, QuietHours, TableFields},
    format::FormatVersion,
    rows::{AirWingRow, CountryAppearanceRow, RaidRow, TableRows, WingEquipmentRow},
};
use std::{any::Any, ops::Range};

/// The tables the roster and the variant catalogue read.
pub const TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::AirAces,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::AirWings,
    TableId::WingEquipment,
    TableId::Raids,
    TableId::RaidTooltipOperands,
    TableId::CountryAppearance,
    TableId::LexicalTokens,
    TableId::VariantDefinitions,
    TableId::VariantModules,
    TableId::VariantUpgrades,
];

pub static FAMILY: Family = Family {
    name: "aces",
    extract: "aces.extract",
    sequential: "aces.sequential",
    fallback: "aces.fallback",
    tables: TABLES,
    fields: Some(fields),
    start,
};

/// The fields of the aces' tables the roster and variant catalogue read.
/// Campaign is checked on every read and read by nothing else here.
fn fields() -> io::Result<Vec<TableFields>> {
    Ok(vec![
        // The roster: every field of every row (a run holds while its row
        // is equal), and every read, which dates the roster (daily).
        TableFields::all(TableId::AirAces).when_read(),
        // Country identity: the id (the row key), canonical slot and tag.
        // The balances change every hour and are not read.
        TableFields::named(TableId::Countries, &["canonical_slot", "tag"])?,
        TableFields::all(TableId::CountryLinks),
        // A wing's id (the row key) and stored name; its planes, missions
        // and eligibility move every hour and are not read.
        TableFields::named(TableId::AirWings, &["name"])?,
        // A wing's equipment variants by ordinal (the row key); the amounts
        // move and are not read.
        TableFields::named(TableId::WingEquipment, &["variant"])?,
        // Legacy wing names come from the active raids' wing, phase and unit
        // (keyed by id and reference type) and their tooltip operands.
        TableFields::named(TableId::Raids, &["phase", "unit", "air_wing"])?,
        TableFields::all(TableId::RaidTooltipOperands),
        // Variant owners' original tags.
        TableFields::named(TableId::CountryAppearance, &["name_tag"])?,
        TableFields::all(TableId::LexicalTokens),
        // The variant catalogue: every field, and every read (daily), which
        // dates the variants' names.
        TableFields::all(TableId::VariantDefinitions).when_read(),
        TableFields::all(TableId::VariantModules),
        TableFields::all(TableId::VariantUpgrades),
    ])
}

fn start(format: FormatVersion) -> Box<dyn Extract> {
    Box::new(BlockExtract {
        // The roster is read daily, and again at a read only when one of its
        // tables was decoded again there: an ace entering between reads
        // joins at the next read hourly preparation re-reads.
        kept: Kept::new(TABLES, format, kept::Rebuilt::Every),
        countries: kept::Countries::default(),
        wings: Reduced::new(TableId::AirWings),
        equipment: Reduced::new(TableId::WingEquipment),
        raids: Reduced::new(TableId::Raids),
        appearance: Reduced::new(TableId::CountryAppearance),
    })
}

struct BlockFacts {
    hours: Vec<kept::Hour>,
}

/// Kept frames hold only what the roster and catalogue read of the tables
/// that move every hour: the countries' keys, the wings' ids and names, the
/// wings' variants, the raids' wing, phase and unit, and the original tags,
/// each shared by kept hours where it is equal.
struct BlockExtract {
    kept: Kept,
    countries: kept::Countries,
    wings: Reduced<AirWingRow>,
    equipment: Reduced<WingEquipmentRow>,
    raids: Reduced<RaidRow>,
    appearance: Reduced<CountryAppearanceRow>,
}

impl BlockExtract {
    fn reduce(&mut self, frame: &DecodedFrame) -> DecodedFrame {
        let mut kept = frame.clone();
        self.countries.reduce(&mut kept);
        let wings = frame.complete::<tables::AirWings>().ok().map(|rows| {
            rows.iter()
                .map(|row| AirWingRow {
                    id: row.id,
                    name: row.name.clone(),
                    ..Default::default()
                })
                .collect()
        });
        self.wings.keep(&mut kept, wings, TableRows::AirWings);
        let equipment = frame.complete::<tables::WingEquipment>().ok().map(|rows| {
            rows.iter()
                .map(|row| WingEquipmentRow {
                    wing: row.wing,
                    ordinal: row.ordinal,
                    variant: row.variant,
                    ..Default::default()
                })
                .collect()
        });
        self.equipment
            .keep(&mut kept, equipment, TableRows::WingEquipment);
        let raids = frame.complete::<tables::Raids>().ok().map(|rows| {
            rows.iter()
                .map(|row| RaidRow {
                    reference_type: row.reference_type,
                    id: row.id,
                    phase: row.phase,
                    unit: row.unit,
                    air_wing: row.air_wing,
                    ..Default::default()
                })
                .collect()
        });
        self.raids.keep(&mut kept, raids, TableRows::Raids);
        let appearance = frame
            .complete::<tables::CountryAppearance>()
            .ok()
            .map(|rows| {
                rows.iter()
                    .map(|row| CountryAppearanceRow {
                        country: row.country,
                        name_tag: row.name_tag.clone(),
                        ..Default::default()
                    })
                    .collect()
            });
        self.appearance
            .keep(&mut kept, appearance, TableRows::CountryAppearance);
        kept
    }
}

impl Extract for BlockExtract {
    fn observe(
        &mut self,
        _: &Replay,
        frame: &DecodedFrame,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        let kept = self.reduce(frame);
        self.kept.keep(kept, changes);
        Ok(())
    }

    fn quiet(&mut self, _: &Replay, hours: &QuietHours, span: Range<usize>) -> io::Result<()> {
        self.kept.quiet(hours, span)
    }

    fn finish(self: Box<Self>) -> io::Result<Box<dyn Any + Send>> {
        Ok(Box::new(BlockFacts {
            hours: self.kept.finish()?,
        }))
    }
}

/// The block's position in the recording.
fn position(replay: &Replay, index: &BlockIndex) -> io::Result<usize> {
    replay
        .index
        .iter()
        .position(|row| std::ptr::eq(row, index))
        .ok_or_else(|| io::Error::other("Ace block is not in the recording"))
}

/// Build the block's window at its end, as hourly preparation does while
/// the aces go on; held until the commit publishes it.
fn finish(
    prepared: &mut Prepared,
    source: &Source,
    replay: &Replay,
    index: &BlockIndex,
    context: &LoadContext,
    reached: Option<i32>,
    observed: &mut kept::Observed<'_>,
) -> io::Result<()> {
    let block = position(replay, index)?;
    let result = prepared.finish_window(source, replay, block, context, &AtomicBool::new(false));
    observed(reached.unwrap_or(index.last_hour), result)?;
    Ok(())
}

/// Fold one block's facts into the carried aces, then publish its window.
/// Returns how it folded and the last hour it reached.
#[allow(clippy::too_many_arguments)]
pub fn fold(
    prepared: &mut Prepared,
    facts: Box<dyn Any + Send>,
    source: &Source,
    replay: &Replay,
    index: &BlockIndex,
    context: &LoadContext,
    observed: &mut kept::Observed<'_>,
    check: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<(Folded, Option<i32>)> {
    let facts = facts
        .downcast::<BlockFacts>()
        .map_err(|_| io::Error::other("Pre-pass facts of another family"))?;
    let mut live = true;
    let (reached, kept) = kept::fold(
        facts.hours,
        &mut |frame: &DecodedFrame, changes: &FrameChanges| {
            prepared.observe_changed(frame, replay, changes)
        },
        &mut |hour: i32, result: io::Result<()>| -> io::Result<bool> {
            live = observed(hour, result)?;
            Ok(live)
        },
        check,
    )?;
    #[cfg(any(test, feature = "test-support"))]
    count_kept(kept);
    #[cfg(not(any(test, feature = "test-support")))]
    let _ = kept;
    if live {
        finish(prepared, source, replay, index, context, reached, observed)?;
    }
    Ok((Folded::Fast, reached))
}

/// Observe the block hour by hour with the change facts of the block alone,
/// as hourly preparation does, then publish its window. Returns the last
/// hour observed.
#[allow(clippy::too_many_arguments)]
pub fn observe_block(
    prepared: &mut Prepared,
    source: &Source,
    replay: &Replay,
    index: &BlockIndex,
    context: &LoadContext,
    observed: &mut kept::Observed<'_>,
    check: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<Option<i32>> {
    let mut tracker = FrameChangeTracker::new();
    let mut reached = None;
    let mut stopped = false;
    let mut failure = None;
    let read = replay.read_block_alone(
        index,
        TABLES,
        |raw, frame| {
            let changes = tracker.observe_updates(frame, Arc::new(raw), replay.format);
            if stopped {
                return Ok(());
            }
            reached = Some(frame.hour);
            let result = prepared.observe_changed(frame, replay, &changes);
            match observed(frame.hour, result) {
                Ok(live) => {
                    stopped = !live;
                    Ok(())
                }
                Err(error) => {
                    failure = Some(error);
                    Err(io::Error::other("Ace observation stopped"))
                }
            }
        },
        || check(),
    );
    if let Some(error) = failure {
        return Err(error);
    }
    read?;
    if !stopped {
        finish(prepared, source, replay, index, context, reached, observed)?;
    }
    Ok(reached)
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static KEPT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(any(test, feature = "test-support"))]
fn count_kept(hours: usize) {
    KEPT.with(|count| count.set(count.get() + hours));
}

/// Kept hours (frames) the folds on this thread observed.
#[cfg(any(test, feature = "test-support"))]
pub fn kept_hours() -> usize {
    KEPT.with(std::cell::Cell::get)
}
