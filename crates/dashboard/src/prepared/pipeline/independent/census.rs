//! Country factories and dockyard repairs prepared on their own. Each joins
//! its hourly country table to the countries' keys and links: its observer
//! acts when a joined key, link, availability or row of its table changes, or
//! when the table's read flag does, and an hour between repeats the last
//! read's counts and advances its series in constant time. So a block is read
//! by its changes ([`prepass::Family`] fields: the countries' keys, every
//! link and the census operands): a pre-pass
//! worker keeps the frames of the hours the census observes (countries
//! reduced to the keys it reads), with a frame at each hour its table's read
//! is due to begin or end by the recording's cadence (none for a table read
//! every hour). Repairs also keep the daily production modifiers read
//! and variant naming boundaries. Each hour between carries the supplying table's slot read
//! from that hour's read evidence.
//!
//! The family's own thread folds each block in order: each kept hour is
//! observed on the carried census as hourly preparation observes it, and each
//! hour between takes the census's unchanged-hour step (`advance_quiet`),
//! which hourly preparation takes there. When the carried census does not
//! continue into such an hour (its read flag differs, its countries did not
//! join, or a dated read of an incomplete table names its reason from the
//! frame), the rest of the block is observed hour by hour.
//!
//! Each saves its continuation, which hourly preparation keeps in its branch
//! state, as `<family>-continuation-<branch>`, and publishes the series and
//! catalogue hourly preparation publishes, flushing where it flushes.

use super::super::compressed_json;
use super::{Block, Branch, Load, Observed, Own};
use crate::{
    prepared::{
        frame_changes::FrameChanges,
        prepass::{self, Extract, Folded},
    },
    products::{country_factories, dockyard_repairs},
};
use dashboard_base::prepared::kept::{Reduced, consecutive, kept_changes, observe_hours};
use replay::{container::QuietHours, container::Replay, container::TableFields};
use replay_data::{format::FormatVersion, rows::CountryRow, rows::TableId};
use replay_frame::frame::{DecodedFrame, FrameLog};
use serde::{Serialize, de::DeserializeOwned};
use std::{any::Any, io, ops::Range, sync::atomic::AtomicBool};

/// A census's branch state, observing as hourly preparation observes it.
trait Census: Default + Serialize + DeserializeOwned + 'static {
    /// Its pre-pass family.
    fn family() -> &'static prepass::Family;
    /// One hour, as hourly preparation observes it.
    fn observe(
        &mut self,
        frame: &DecodedFrame,
        replay: &Replay,
        changes: &FrameChanges,
    ) -> io::Result<()>;
    /// An hour following the last observed at which nothing it reads
    /// changed, its table read (`read`) or not: what observing it does, or
    /// `Ok(false)` with nothing changed when that needs the hour's frame.
    fn advance_quiet(&mut self, hour: i32, read: bool) -> io::Result<bool>;
    fn bytes(&self) -> usize;
    fn flush(
        &mut self,
        writer: &mut store::Writer,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()>;
}

/// [`Census`] for a prepared census of `family`, through its own methods.
macro_rules! census {
    ($prepared:ty, $family:ident) => {
        impl Census for $prepared {
            fn family() -> &'static prepass::Family {
                &$family
            }

            fn observe(
                &mut self,
                frame: &DecodedFrame,
                replay: &Replay,
                changes: &FrameChanges,
            ) -> io::Result<()> {
                Self::observe(self, frame, replay, changes)
            }

            fn advance_quiet(&mut self, hour: i32, read: bool) -> io::Result<bool> {
                Self::advance_quiet(self, hour, read)
            }

            fn bytes(&self) -> usize {
                Self::bytes(self)
            }

            fn flush(
                &mut self,
                writer: &mut store::Writer,
                branch: u64,
                cancelled: &AtomicBool,
            ) -> io::Result<()> {
                Self::flush(self, writer, branch, cancelled)
            }
        }
    };
}

census!(country_factories::prepared::Prepared, FACTORIES_FAMILY);
census!(dockyard_repairs::Prepared, REPAIRS_FAMILY);

// --- Families --------------------------------------------------------------

/// The tables the factory census reads.
const FACTORY_TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryFactories,
];

static FACTORIES_FAMILY: prepass::Family = prepass::Family {
    name: "factories",
    extract: "factories.extract",
    sequential: "factories.sequential",
    fallback: "factories.fallback",
    tables: FACTORY_TABLES,
    fields: Some(factory_fields),
    start: factory_start,
};

/// What the factory census reads. Campaign is checked on every read and read
/// by nothing else here.
fn factory_fields() -> io::Result<Vec<TableFields>> {
    Ok(vec![
        // Country keys (`force_census::Countries`): the id (the row key),
        // canonical slot and tag. The balances move every hour and are not
        // read.
        TableFields::named(TableId::Countries, &["canonical_slot", "tag"])?,
        // Every link: holders are joined through their slots.
        TableFields::all(TableId::CountryLinks),
        // Every factory field: any row change is a new read of the holder's
        // counts. A read with equal rows only repeats them; its read flag is
        // kept with the quiet hours.
        TableFields::all(TableId::CountryFactories),
    ])
}

fn factory_start(format: FormatVersion) -> Box<dyn Extract> {
    CensusExtract::start(format, TableId::CountryFactories)
}

/// Hourly repair use with the production census's carried daily output.
static REPAIRS_FAMILY: prepass::Family = prepass::Family {
    name: "repairs",
    extract: "repairs.extract",
    sequential: "repairs.sequential",
    fallback: "repairs.fallback",
    tables: &[
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::NavalRepairCountries,
        TableId::CountryProductionModifiers,
        TableId::NavalRepairBases,
        TableId::NavalRepairQueue,
        TableId::NavalBases,
        TableId::Ships,
        TableId::ShipModels,
        TableId::TaskForces,
        TableId::Provinces,
        TableId::RelationLinks,
        TableId::FactionMembers,
        TableId::Subjects,
        TableId::WarRelations,
        TableId::CountryAppearance,
        TableId::LexicalTokens,
        TableId::VariantDefinitions,
        TableId::VariantModules,
        TableId::VariantUpgrades,
        TableId::VariantChanges,
    ],
    fields: Some(repair_fields),
    start: repair_start,
};

fn repair_fields() -> io::Result<Vec<TableFields>> {
    let mut fields = vec![
        TableFields::named(TableId::Countries, &["canonical_slot", "tag"])?,
        TableFields::named(TableId::NavalRepairCountries, &["num_used_dockyards"])?,
        TableFields::named(
            TableId::CountryProductionModifiers,
            &["power_raw", "factory_speed", "trees"],
        )?
        .when_read(),
    ];
    for &(table, names) in dockyard_repairs::SELECTION_FIELDS {
        fields.push(if names.is_empty() {
            TableFields::all(table)
        } else {
            TableFields::named(table, names)?
        });
    }
    for table in [
        TableId::CountryAppearance,
        TableId::LexicalTokens,
        TableId::VariantDefinitions,
        TableId::VariantModules,
        TableId::VariantUpgrades,
        TableId::VariantChanges,
    ] {
        let field = TableFields::all(table);
        fields.push(if table == TableId::VariantDefinitions {
            field.when_read()
        } else {
            field
        });
    }
    Ok(fields)
}

fn repair_start(format: FormatVersion) -> Box<dyn Extract> {
    let mut extract = CensusExtract::start(format, TableId::CountryProductionModifiers);
    extract.boundaries = &[TableId::VariantDefinitions];
    extract
}

// --- Extract and fold ------------------------------------------------------

/// One hour of a census block's facts, in hour order.
enum Hour {
    /// An hour the census observes, with its change facts; its frame is the
    /// next the block's kept frames hold.
    Observed { changes: FrameChanges },
    /// Consecutive recorded hours from `first`, after the hour before them,
    /// at which no key, link, availability or row the census reads changed:
    /// whether its table was read at each.
    Quiet { first: i32, reads: Vec<bool> },
}

/// A census block's hours as the extract kept them.
struct Facts {
    hours: Vec<Hour>,
    kept: FrameLog,
    /// Frames the extract was given or built.
    #[cfg(test)]
    frames: usize,
}

struct CensusExtract {
    format: FormatVersion,
    /// The census table, whose read flag each quiet hour carries.
    table: TableId,
    /// Additional daily reads which need a retained frame, not only a read bit.
    boundaries: &'static [TableId],
    hours: Vec<Hour>,
    kept: FrameLog,
    countries: Reduced<CountryRow>,
    /// The last hour given, observed or quiet, and whether its table was
    /// read there.
    last: Option<(i32, bool)>,
    #[cfg(test)]
    frames: usize,
}

impl CensusExtract {
    fn start(format: FormatVersion, table: TableId) -> Box<Self> {
        Box::new(Self {
            format,
            table,
            boundaries: &[],
            hours: Vec::new(),
            kept: FrameLog::new(),
            countries: Reduced::new(TableId::Countries),
            last: None,
            #[cfg(test)]
            frames: 0,
        })
    }
}

impl Extract for CensusExtract {
    /// What the block's facts hold: each kept frame's views once, its change
    /// facts and the hours kept between them.
    fn bytes(&self) -> u64 {
        let mut held = dashboard_base::prepared::prepass::HeldBytes::default();
        held.add(self.hours.capacity() * size_of::<Hour>());
        for hour in &self.hours {
            match hour {
                Hour::Observed { changes } => {
                    held.changes(changes);
                }
                Hour::Quiet { reads, .. } => {
                    held.add(reads.capacity());
                }
            }
        }
        for view in self.kept.views() {
            held.view(view);
        }
        held.add(self.kept.bytes());
        held.total()
    }

    fn observe(
        &mut self,
        _: &Replay,
        frame: &DecodedFrame,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        #[cfg(test)]
        {
            self.frames += 1;
        }
        let read = std::iter::once(self.table)
            .chain(self.boundaries.iter().copied())
            .any(|table| {
                crate::metadata::slot_read(self.format, &frame.metrics, table, frame.hour)
            });
        self.last = Some((frame.hour, read));
        let mut kept = frame.clone();
        self.countries.countries(&mut kept);
        self.kept.push(&kept)?;
        self.hours.push(Hour::Observed {
            changes: kept_changes(changes),
        });
        Ok(())
    }

    /// Hours at which nothing the census reads changed: each hour's slot read
    /// of its table, from its read evidence, as the change facts of an
    /// observed hour carry it.
    fn quiet(&mut self, _: &Replay, hours: &QuietHours, span: Range<usize>) -> io::Result<()> {
        let Some((first, _)) = consecutive(hours, span.clone())? else {
            return Ok(());
        };
        let (format, table) = (self.format, self.table);
        let reads: Vec<bool> = span
            .map(|position| {
                std::iter::once(table)
                    .chain(self.boundaries.iter().copied())
                    .any(|table| {
                        crate::metadata::slot_read(
                            format,
                            &hours.metrics()[position],
                            table,
                            hours.hours()[position],
                        )
                    })
            })
            .collect();
        if let Some(&read) = reads.last() {
            self.last = Some((first + reads.len() as i32 - 1, read));
        }
        match self.hours.last_mut() {
            Some(Hour::Quiet {
                first: held,
                reads: held_reads,
            }) if i64::from(*held) + held_reads.len() as i64 == i64::from(first) => {
                held_reads.extend(reads);
            }
            _ => self.hours.push(Hour::Quiet { first, reads }),
        }
        Ok(())
    }

    /// The next hour the watched table's read is due to begin or end by its
    /// cadence in the recording's format (each hour of a table read daily in
    /// it, or never for one read every hour): observed with a frame, so the
    /// hours between share the read flag of the hour before them.
    fn deadline(&self) -> Option<i32> {
        let (hour, read) = self.last?;
        let due = |hour| {
            std::iter::once(self.table)
                .chain(self.boundaries.iter().copied())
                .any(|table| replay_data::cadence::cadence_in(table, self.format).is_refresh(hour))
        };
        (1..=replay_data::cadence::STAGGER_PERIOD)
            .filter_map(|ahead| hour.checked_add(ahead))
            .find(|&next| due(next) != read)
    }

    fn finish(self: Box<Self>) -> io::Result<Box<dyn Any + Send>> {
        if self.hours.is_empty() {
            return Err(io::Error::other("Census block has no hours"));
        }
        Ok(Box::new(Facts {
            hours: self.hours,
            kept: self.kept,
            #[cfg(test)]
            frames: self.frames,
        }))
    }
}

/// Fold one block's facts into the carried census. Returns how it folded and
/// the last hour it reached.
fn fold<C: Census>(
    census: &mut C,
    facts: Box<dyn Any + Send>,
    block: &Block<'_>,
    observed: &mut Observed<'_>,
    check: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<(Folded, Option<i32>)> {
    let _cpu = ::replay_frame::cpu::scope("fold driver", None);
    let facts = {
        let _cpu = ::replay_frame::cpu::scope("fold facts", None);
        facts
            .downcast::<Facts>()
            .map_err(|_| io::Error::other("Pre-pass facts of another family"))?
    };
    #[cfg(test)]
    dashboard_base::prepared::kept::note_frames(C::family().name, facts.frames);
    let Facts { hours, kept, .. } = *facts;
    let mut kept = kept.into_frames();
    let mut reached = None;
    for hour in hours {
        let _cpu = ::replay_frame::cpu::scope("fold driver", None);
        check()?;
        match hour {
            Hour::Observed { changes } => {
                let frame = kept.next().ok_or_else(|| {
                    io::Error::other("Census facts lack an observed hour's frame")
                })??;
                reached = Some(frame.hour);
                #[cfg(test)]
                dashboard_base::prepared::kept::note_observed(C::family().name);
                let result = {
                    let _cpu = ::replay_frame::cpu::scope("fold observe", None);
                    census.observe(&frame, block.replay, &changes)
                };
                if !observed(frame.hour, result)? {
                    return Ok((Folded::Fast, reached));
                }
            }
            Hour::Quiet { first, reads } => {
                for (offset, read) in reads.into_iter().enumerate() {
                    let _cpu = ::replay_frame::cpu::scope("fold driver", None);
                    // The span's hours are consecutive i32 hours.
                    let hour = first + offset as i32;
                    let advanced = {
                        let _cpu = ::replay_frame::cpu::scope("fold observe", None);
                        census.advance_quiet(hour, read)
                    };
                    match advanced {
                        Ok(true) => {
                            reached = Some(hour);
                            if !observed(hour, Ok(()))? {
                                return Ok((Folded::Fast, reached));
                            }
                        }
                        Ok(false) => {
                            let reached =
                                observe_block(census, block, Some(hour), observed, check)?;
                            return Ok((Folded::Fallback, reached));
                        }
                        // A refusal at this hour, as observing it would
                        // have refused there.
                        Err(error) => {
                            observed(hour, Err(error))?;
                            return Ok((Folded::Fast, Some(hour)));
                        }
                    }
                }
            }
        }
    }
    Ok((Folded::Fast, reached))
}

/// Observe the block hour by hour from `from` (every hour without it), with
/// the change facts of the block alone, as hourly preparation does. Returns
/// the last hour observed.
fn observe_block<C: Census>(
    census: &mut C,
    block: &Block<'_>,
    from: Option<i32>,
    observed: &mut Observed<'_>,
    check: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<Option<i32>> {
    let _cpu = ::replay_frame::cpu::scope("fold driver", None);
    let family = C::family();
    observe_hours(
        block.replay,
        block.index,
        family.tables,
        from,
        observed,
        check,
        |frame, changes| {
            #[cfg(test)]
            dashboard_base::prepared::kept::note_observed(family.name);
            census.observe(frame, block.replay, changes)
        },
    )
}

// --- Continuations ---------------------------------------------------------

/// A family's continuation section of one branch.
pub(super) fn continuation(family: &str, branch: u64) -> String {
    crate::prepared::driver::registry::continuation_section(family, branch)
}

/// Write a continuation, compressed as the hourly branch state holding it is.
pub(super) fn save<T: Serialize>(
    writer: &mut store::Writer,
    section: &str,
    value: &T,
) -> io::Result<()> {
    writer.write_at(section, 0, compressed_json(value)?.as_slice())
}

/// A continuation the last commit saved, if it has one.
pub(super) fn read<T: DeserializeOwned>(load: &Load<'_>, section: &str) -> io::Result<Option<T>> {
    let Some(file) = load.read(section)? else {
        return Ok(None);
    };
    let decoder = zstd::stream::Decoder::new(file)?;
    Ok(Some(serde_json::from_reader(io::BufReader::new(decoder))?))
}

// --- Registry entries ------------------------------------------------------

/// One branch's census.
struct CensusBranch<C>(Box<C>);

impl<C: Census> CensusBranch<C> {
    fn load(load: &Load<'_>, branch: u64) -> io::Result<Box<dyn Branch>> {
        let census = read(load, &continuation(C::family().name, branch))?.unwrap_or_default();
        Ok(Box::new(Self(Box::new(census))))
    }
}

impl<C: Census> Branch for CensusBranch<C> {
    fn fold(
        &mut self,
        facts: Box<dyn Any + Send>,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<(Folded, Option<i32>)> {
        fold(&mut *self.0, facts, block, observed, check)
    }

    fn observe_block(
        &mut self,
        block: &Block<'_>,
        observed: &mut Observed<'_>,
        check: &mut dyn FnMut() -> io::Result<()>,
    ) -> io::Result<Option<i32>> {
        observe_block(&mut *self.0, block, None, observed, check)
    }

    fn pending(&self) -> usize {
        self.0.bytes()
    }

    fn flush(
        &mut self,
        writer: &mut store::Writer,
        _: &Replay,
        branch: u64,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.0.flush(writer, branch, cancelled)
    }

    fn save_state(&self, writer: &mut store::Writer, branch: u64) -> io::Result<()> {
        save(writer, &continuation(C::family().name, branch), &*self.0)
    }
}

/// Country factories: counts, ranks, parts, compositions and supplying
/// reads. Faction totals of factories read them with the faction rosters
/// and the identity history (`registry::QUERIES`).
pub(in crate::prepared::pipeline) struct Factories;

pub(in crate::prepared::pipeline) static FACTORIES: Factories = Factories;

impl Own for Factories {
    fn family(&self) -> &'static prepass::Family {
        &FACTORIES_FAMILY
    }

    fn timing(&self) -> &'static str {
        "independent.factories"
    }

    fn load(&self, load: &Load<'_>, branch: u64) -> io::Result<Box<dyn Branch>> {
        CensusBranch::<country_factories::prepared::Prepared>::load(load, branch)
    }
}

/// Dockyard repair rates, joined with production by the spending query.
pub(in crate::prepared::pipeline) struct Repairs;
pub(in crate::prepared::pipeline) static REPAIRS: Repairs = Repairs;

impl Own for Repairs {
    fn family(&self) -> &'static prepass::Family {
        &REPAIRS_FAMILY
    }
    fn timing(&self) -> &'static str {
        "independent.repairs"
    }
    fn load(&self, load: &Load<'_>, branch: u64) -> io::Result<Box<dyn Branch>> {
        CensusBranch::<dockyard_repairs::Prepared>::load(load, branch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dashboard_base::prepared::series::tests::Fixture;
    use std::{collections::BTreeMap, io::Read};

    #[test]
    fn continuation_compression_preserves_saved_bytes() {
        let value: BTreeMap<_, _> = (0..5_000)
            .map(|id| (format!("holder {id}"), vec![id; 24]))
            .collect();
        let plain = serde_json::to_vec(&value).unwrap();
        let mut direct = zstd::stream::Encoder::new(Vec::new(), 1).unwrap();
        serde_json::to_writer(&mut direct, &value).unwrap();
        let expected = direct.finish().unwrap();
        let fixture = Fixture::new();
        let mut writer = fixture.writer();
        save(&mut writer, "census-continuation", &value).unwrap();
        assert_eq!(writer.bytes_written(), expected.len() as u64);
        writer.commit().unwrap();
        let mut actual = Vec::new();
        fixture
            .store
            .read(&fixture.key, "census-continuation", 0)
            .unwrap()
            .unwrap()
            .read_to_end(&mut actual)
            .unwrap();
        assert_eq!(actual, expected);
        assert_eq!(zstd::stream::decode_all(actual.as_slice()).unwrap(), plain);
    }
}
