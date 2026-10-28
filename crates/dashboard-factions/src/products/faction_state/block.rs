//! The shared pre-pass retains only changed faction inputs and country keys.
use super::prepared::Prepared;
use dashboard_base::{
    Source,
    prepared::{
        frame_changes::FrameChanges,
        kept::{Observed, Reduced, consecutive, kept_changes},
        prepass::{Extract, Family, Folded, HeldBytes},
    },
    products::LoadContext,
};
use replay::container::{BlockIndex, QuietHours, Replay, TableFields};
use replay_data::{
    format::FormatVersion,
    rows::{CountryRow, TableId},
};
use replay_frame::frame::{DecodedFrame, FrameLog};
use std::{
    any::Any,
    io,
    ops::Range,
    sync::{LazyLock, atomic::AtomicBool},
};
/// Include trigger-kit tables: dynamic names can read beyond this family's tables.
pub fn whole_tables() -> &'static [TableId] {
    static TABLES_WITH_KIT: LazyLock<Vec<TableId>> = LazyLock::new(|| {
        let mut tables = TABLES.to_vec();
        tables.extend_from_slice(dashboard_base::triggers::RECORDED_TABLES);
        tables.sort_unstable();
        tables.dedup();
        tables
    });
    &TABLES_WITH_KIT
}
pub const TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::ReservedCountryNaming,
    TableId::Factions,
    TableId::FactionMembers,
    TableId::FactionAccounts,
    TableId::FactionProjection,
    TableId::FactionUnlocks,
    TableId::FactionGoalStatus,
    TableId::FactionGoalEntries,
    TableId::FactionRules,
    TableId::FactionPrograms,
    TableId::FactionMinimumUpgrades,
    TableId::FactionDoctrine,
    TableId::FactionManpower,
    TableId::LexicalTokens,
    TableId::ProjectPrograms,
    TableId::Projects,
    // A dynamic rule or doctrine name is expanded for the viewing country.
    TableId::LocalPlayer,
];
pub static FAMILY: Family = Family {
    name: "faction_state",
    extract: "faction_state.extract",
    sequential: "faction_state.sequential",
    fallback: "faction_state.fallback",
    tables: TABLES,
    fields: Some(fields),
    start,
};
fn fields() -> io::Result<Vec<TableFields>> {
    TABLES
        .iter()
        .filter(|table| **table != TableId::Campaign)
        .map(|table| {
            if *table == TableId::Countries {
                TableFields::named(*table, &["canonical_slot", "tag"])
            } else if *table == TableId::FactionAccounts {
                TableFields::named(
                    *table,
                    &[
                        "country",
                        "upgrade_groups",
                        "upgrade_tokens",
                        "influence_share_raw",
                        "influence_numerator_raw",
                        "influence_denominator_raw",
                        "influence_refresh",
                        "influence_faction",
                    ],
                )
            } else {
                Ok(if *table == TableId::ProjectPrograms {
                    TableFields::all(*table).when_read()
                } else {
                    TableFields::all(*table)
                })
            }
        })
        .collect()
}
enum Hour {
    /// Its frame is the next the block's kept frames hold.
    Observed { changes: FrameChanges },
    Quiet {
        first: i32,
        last: i32,
        reads: [bool; super::prepared::CHANNELS.len()],
    },
}
struct Facts {
    hours: Vec<Hour>,
    kept: FrameLog,
}
struct Extraction {
    hours: Vec<Hour>,
    kept: FrameLog,
    countries: Reduced<CountryRow>,
    format: FormatVersion,
    deadline: Option<i32>,
}
fn start(format: FormatVersion) -> Box<dyn Extract> {
    Box::new(Extraction {
        hours: Vec::new(),
        kept: FrameLog::new(),
        countries: Reduced::new(TableId::Countries),
        format,
        deadline: None,
    })
}
impl Extract for Extraction {
    fn bytes(&self) -> u64 {
        let mut bytes = HeldBytes::default();
        bytes.add(self.hours.capacity() * size_of::<Hour>());
        for hour in &self.hours {
            if let Hour::Observed { changes } = hour {
                bytes.changes(changes);
            }
        }
        for view in self.kept.views() {
            bytes.view(view);
        }
        bytes.add(self.kept.bytes());
        bytes.total()
    }
    fn observe(
        &mut self,
        _: &Replay,
        frame: &DecodedFrame,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        self.deadline = if frame
            .complete::<replay_data::rows::tables::FactionPrograms>()
            .is_ok_and(|rows| !rows.is_empty())
        {
            let cadence = replay_data::cadence::cadence_in(TableId::ProjectPrograms, self.format);
            (1..=replay_data::cadence::STAGGER_PERIOD)
                .filter_map(|offset| frame.hour.checked_add(offset))
                .find(|hour| cadence.is_refresh(*hour))
        } else {
            None
        };
        let mut kept = frame.clone();
        self.countries.countries(&mut kept);
        self.kept.push(&kept)?;
        self.hours.push(Hour::Observed {
            changes: kept_changes(changes),
        });
        Ok(())
    }
    fn deadline(&self) -> Option<i32> {
        self.deadline
    }
    fn quiet(&mut self, _: &Replay, hours: &QuietHours, span: Range<usize>) -> io::Result<()> {
        consecutive(hours, span.clone())?;
        for at in span {
            let hour = hours.hours()[at];
            let reads = super::prepared::CHANNELS.map(|(_, table)| {
                dashboard_base::metadata::slot_read(self.format, &hours.metrics()[at], table, hour)
            });
            if let Some(Hour::Quiet {
                last,
                reads: before,
                ..
            }) = self.hours.last_mut()
                && last.checked_add(1) == Some(hour)
                && *before == reads
            {
                *last = hour;
            } else {
                self.hours.push(Hour::Quiet {
                    first: hour,
                    last: hour,
                    reads,
                });
            }
        }
        Ok(())
    }
    fn finish(self: Box<Self>) -> io::Result<Box<dyn Any + Send>> {
        Ok(Box::new(Facts {
            hours: self.hours,
            kept: self.kept,
        }))
    }
}
#[allow(clippy::too_many_arguments)]
pub fn fold(
    state: &mut Prepared,
    facts: Box<dyn Any + Send>,
    source: &Source,
    context: &LoadContext,
    replay: &Replay,
    index: &BlockIndex,
    observed: &mut Observed<'_>,
    check: &mut dyn FnMut() -> io::Result<()>,
    cancelled: &AtomicBool,
) -> io::Result<(Folded, Option<i32>)> {
    let facts = facts
        .downcast::<Facts>()
        .map_err(|_| io::Error::other("Faction pre-pass facts have another family"))?;
    let Facts { hours, kept } = *facts;
    let mut kept = kept.into_frames();
    let mut reached = None;
    for hour in hours {
        check()?;
        match hour {
            Hour::Observed { changes } => {
                let frame = kept.next().ok_or_else(|| {
                    io::Error::other("Faction pre-pass facts lack an observed hour's frame")
                })??;
                // Dynamic names need whole frames; kept frames may contain stale trigger inputs.
                if state.needs_whole(source, context, &frame) {
                    let reached = observe_block(
                        state,
                        source,
                        context,
                        replay,
                        index,
                        Some(frame.hour),
                        observed,
                        check,
                        cancelled,
                    )?;
                    return Ok((Folded::Fallback, reached));
                }
                reached = Some(frame.hour);
                if !observed(
                    frame.hour,
                    state.observe(source, context, &frame, &changes, cancelled),
                )? {
                    break;
                }
            }
            Hour::Quiet { first, last, reads } => {
                if state.quiet(first, last, reads).is_err() {
                    let reached = observe_block(
                        state,
                        source,
                        context,
                        replay,
                        index,
                        Some(first),
                        observed,
                        check,
                        cancelled,
                    )?;
                    return Ok((Folded::Fallback, reached));
                }
                reached = Some(last);
            }
        }
    }
    Ok((Folded::Fast, reached))
}
#[allow(clippy::too_many_arguments)]
pub fn observe_block(
    state: &mut Prepared,
    source: &Source,
    context: &LoadContext,
    replay: &Replay,
    index: &BlockIndex,
    from: Option<i32>,
    observed: &mut Observed<'_>,
    check: &mut dyn FnMut() -> io::Result<()>,
    cancelled: &AtomicBool,
) -> io::Result<Option<i32>> {
    dashboard_base::prepared::kept::observe_hours(
        replay,
        index,
        whole_tables(),
        from,
        observed,
        check,
        |frame, changes| state.observe(source, context, frame, changes, cancelled),
    )
}
