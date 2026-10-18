//! Raids prepared block by block, on its own. A pre-pass worker reads a block
//! alone, only at the hours the fields the raid scan reads change
//! ([`FAMILY`]'s fields), and keeps those hours' frames reduced to what the
//! scan reads ([`kept`]); the fold observes them again on the carried scan
//! and each quiet hour between on the last kept frame, as hourly
//! preparation does. A raid's preparation hours and flight distance move
//! every hour it is live, so its hours are kept while any raid is live; the
//! hours of a world without live raids are quiet.

use super::super::kept::{self, Kept, Reduced};
use super::*;
use dashboard_base::prepared::{
    frame_changes::{FrameChangeTracker, FrameChanges},
    prepass::{Extract, Family, Folded},
};
use replay::{
    container::BlockIndex, container::QuietHours, container::Replay, container::TableFields,
};
use replay_data::{
    format::FormatVersion, rows::CountryAppearanceRow, rows::ProvinceRow, rows::RaidCountryRow,
    rows::StateRow, rows::TableRows,
};
use std::{any::Any, ops::Range};

/// The tables the raid scan reads, its target names' included: a target
/// state owned by country zero is named by the reserved country's original
/// tag.
pub const TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::RaidCountries,
    TableId::Raids,
    TableId::RaidTargets,
    TableId::LexicalTokens,
    TableId::Provinces,
    TableId::States,
    TableId::CountryAppearance,
    TableId::ReservedCountryNaming,
];

pub static FAMILY: Family = Family {
    name: "raids",
    extract: "raids.extract",
    sequential: "raids.sequential",
    fallback: "raids.fallback",
    tables: TABLES,
    fields: Some(fields),
    start,
};

/// The fields of the raid tables the scan reads. Campaign is checked on
/// every read and read by nothing else here.
fn fields() -> io::Result<Vec<TableFields>> {
    Ok(vec![
        // Country identity: the id (the row key), canonical slot and tag.
        // The balances change every hour and are not read.
        TableFields::named(TableId::Countries, &["canonical_slot", "tag"])?,
        TableFields::all(TableId::CountryLinks),
        // Each country's slot (the row key), its country and eligible raid
        // types; the used types are not read.
        TableFields::named(TableId::RaidCountries, &["country", "eligible_types"])?,
        // Each raid (keyed by id and reference type): its parent, phase,
        // outcome, flight distance and preparation hours, the last two
        // moving every hour a raid is live. Its source, ordinal, visibility,
        // unit and wing are not read.
        TableFields::named(
            TableId::Raids,
            &[
                "parent_slot",
                "phase",
                "outcome",
                "travelled",
                "preparation_hours",
            ],
        )?,
        // Each target's scope and type; the detected byte is watched too,
        // as the scope's fields are nested.
        TableFields::all(TableId::RaidTargets),
        // Raid type and building tokens.
        TableFields::all(TableId::LexicalTokens),
        // A target province's state and controller (its country and name).
        TableFields::named(TableId::Provinces, &["state", "controller"])?,
        // A target state's owner and controller, and its stored names.
        TableFields::named(
            TableId::States,
            &["owner", "controller", "custom_name", "owner_name"],
        )?,
        // A target state owner's original tag.
        TableFields::named(
            TableId::CountryAppearance,
            &["original_tag", "original_tag_reference"],
        )?,
        TableFields::all(TableId::ReservedCountryNaming),
    ])
}

fn start(format: FormatVersion) -> Box<dyn Extract> {
    Box::new(BlockExtract {
        // Raids are read every hour: each change is a kept hour.
        kept: Kept::new(TABLES, format, kept::Rebuilt::First),
        countries: kept::Countries::default(),
        parents: Reduced::new(TableId::RaidCountries),
        provinces: Reduced::new(TableId::Provinces),
        states: Reduced::new(TableId::States),
        appearance: Reduced::new(TableId::CountryAppearance),
    })
}

struct BlockFacts {
    hours: Vec<kept::Hour>,
}

/// Kept frames hold only what the scan reads: the countries' keys, the
/// raid countries' slots, countries and eligible types, the original tags,
/// and the provinces and states the targets name, with the fields the
/// target joins read. Each is shared by kept hours where it is equal.
struct BlockExtract {
    kept: Kept,
    countries: kept::Countries,
    parents: Reduced<RaidCountryRow>,
    provinces: Reduced<ProvinceRow>,
    states: Reduced<StateRow>,
    appearance: Reduced<CountryAppearanceRow>,
}

impl BlockExtract {
    fn reduce(&mut self, frame: &DecodedFrame) -> DecodedFrame {
        let mut kept = frame.clone();
        self.countries.reduce(&mut kept);
        let parents = frame.complete::<tables::RaidCountries>().ok().map(|rows| {
            rows.iter()
                .map(|row| RaidCountryRow {
                    canonical_slot: row.canonical_slot,
                    country: row.country,
                    eligible_types: row.eligible_types.clone(),
                    ..Default::default()
                })
                .collect()
        });
        self.parents
            .keep(&mut kept, parents, TableRows::RaidCountries);
        let appearance = frame
            .complete::<tables::CountryAppearance>()
            .ok()
            .map(|rows| {
                rows.iter()
                    .map(|row| CountryAppearanceRow {
                        country: row.country,
                        original_tag: row.original_tag.clone(),
                        original_tag_reference: row.original_tag_reference.clone(),
                        ..Default::default()
                    })
                    .collect()
            });
        self.appearance
            .keep(&mut kept, appearance, TableRows::CountryAppearance);
        // The provinces and states a target can name: its province, leader
        // province, building location and state, and each such province's
        // state. A target's joins read nothing else of these tables.
        let mut province_ids = BTreeSet::new();
        let mut state_ids = BTreeSet::new();
        if let Ok(targets) = frame.complete::<tables::RaidTargets>() {
            for target in targets {
                let scope = &target.scope;
                province_ids.extend([
                    scope.province,
                    scope.leader_province,
                    scope.building_location,
                ]);
                state_ids.extend([scope.state, scope.building_location]);
            }
        }
        province_ids.remove(&0);
        let provinces = frame.complete::<tables::Provinces>().ok().map(|rows| {
            rows.iter()
                .filter(|row| province_ids.contains(&row.id))
                .map(|row| {
                    state_ids.insert(row.state);
                    ProvinceRow {
                        id: row.id,
                        state: row.state,
                        controller: row.controller,
                        ..Default::default()
                    }
                })
                .collect::<Vec<_>>()
        });
        state_ids.remove(&0);
        let states = frame.complete::<tables::States>().ok().map(|rows| {
            rows.iter()
                .filter(|row| state_ids.contains(&row.id))
                .map(|row| StateRow {
                    id: row.id,
                    owner: row.owner,
                    controller: row.controller,
                    custom_name: row.custom_name.clone(),
                    owner_name: row.owner_name.clone(),
                    ..Default::default()
                })
                .collect()
        });
        self.provinces
            .keep(&mut kept, provinces, TableRows::Provinces);
        self.states.keep(&mut kept, states, TableRows::States);
        kept
    }
}

impl Extract for BlockExtract {
    /// What the block's kept hours hold.
    fn bytes(&self) -> u64 {
        self.kept.bytes()
    }

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

/// Fold one block's facts into the carried raids, with the content hourly
/// preparation binds. Returns how it folded and the last hour it reached.
pub fn fold(
    prepared: &mut Prepared,
    content: Option<&Content>,
    facts: Box<dyn Any + Send>,
    replay: &Replay,
    observed: &mut kept::Observed<'_>,
    check: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<(Folded, Option<i32>)> {
    let _cpu = ::replay_frame::cpu::scope("fold driver", None);
    let facts = {
        let _cpu = ::replay_frame::cpu::scope("fold facts", None);
        facts
            .downcast::<BlockFacts>()
            .map_err(|_| io::Error::other("Pre-pass facts of another family"))?
    };
    let format = replay.format;
    let (reached, kept) = kept::fold(
        facts.hours,
        &mut |frame: &DecodedFrame, changes: &FrameChanges| {
            prepared.observe_changed(frame, format, content, changes)
        },
        observed,
        check,
    )?;
    #[cfg(any(test, feature = "test-support"))]
    count_kept(kept);
    #[cfg(not(any(test, feature = "test-support")))]
    let _ = kept;
    Ok((Folded::Fast, reached))
}

/// Observe the block hour by hour with the change facts of the block alone,
/// as hourly preparation does. Returns the last hour observed.
pub fn observe_block(
    prepared: &mut Prepared,
    content: Option<&Content>,
    replay: &Replay,
    index: &BlockIndex,
    observed: &mut kept::Observed<'_>,
    check: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<Option<i32>> {
    let _cpu = ::replay_frame::cpu::scope("fold driver", None);
    let mut tracker = FrameChangeTracker::new();
    let mut reached = None;
    let mut stopped = false;
    let mut failure = None;
    let read = replay.read_block_alone(
        index,
        TABLES,
        |raw, frame| {
            let _cpu = ::replay_frame::cpu::scope("fold driver", None);
            let changes = tracker.observe_updates(frame, std::sync::Arc::new(raw), replay.format);
            if stopped {
                return Ok(());
            }
            reached = Some(frame.hour);
            let result = {
                let _cpu = ::replay_frame::cpu::scope("fold observe", None);
                prepared.observe_changed(frame, replay.format, content, &changes)
            };
            match observed(frame.hour, result) {
                Ok(live) => {
                    stopped = !live;
                    Ok(())
                }
                Err(error) => {
                    failure = Some(error);
                    Err(io::Error::other("Raid observation stopped"))
                }
            }
        },
        || check(),
    );
    if let Some(error) = failure {
        return Err(error);
    }
    read?;
    Ok(reached)
}

#[cfg(any(test, feature = "test-support"))]
fn count_kept(hours: usize) {
    dashboard_base::prepared::kept::note_frames(FAMILY.name, hours);
}

/// Kept hours (frames) the folds on this thread observed.
#[cfg(any(test, feature = "test-support"))]
pub fn kept_hours() -> usize {
    dashboard_base::prepared::kept::counted(FAMILY.name).1
}
