//! Special projects prepared on their own, block by block. A pre-pass worker
//! reads a block alone with the projects' tables and, only where the bound
//! content holds a scoped project name ([`bound`]), the trigger kit such a
//! name evaluates (`triggers::RECORDED_TABLES`), only at the hours a field
//! the observer or a scoped name's declared reads read changes
//! ([`FAMILY`]'s fields): every field of the tables the observer reads of a
//! frame ([`WATCHED_WHOLE`]: projects, programs, history, characters,
//! portraits, the dictionary and the country links), the countries' keys,
//! and of the naming and trigger tables the fields the declared reads read
//! (`triggers::inputs::Key::dependence`), the row keys of the others. The
//! block reader also observes every hour that decodes a table watched whole
//! again, so the dictionary's every new allocation is an observed hour, and
//! each project completion date the history holds ([`Extract::deadline`]),
//! at which the observer revisits its completions.
//!
//! The worker keeps each observed hour's frame, and how it holds the hour's
//! rows ([`Held`]): the block reader builds the frame of an hour at which no
//! watched field changed on the last one read, holding the hour's rows only
//! of the tables watched whole and the watched fields of the others; and a
//! frame it reads at a later hour carries each table so built, not decoded
//! again since, at its built allocation, whose rows are not that hour's. So
//! a frame is the hour's whole frame only where the reader read the hour and
//! every table it holds is that read's own. Between them it keeps each quiet
//! hour's projects slot read and the tables its update rebuilt.
//!
//! The family folds each block in order on the carried observer: a whole
//! hour as hourly preparation observes it; a built or quiet hour (the last
//! kept frame at that hour, with the tables its update rebuilt), or a read
//! one carrying built tables, through [`Prepared::observe_kept`], which
//! observes it exactly while every scoped name's inputs read the same
//! without the frame. Where an hour cannot be observed so, the rest of the
//! block is observed hour by hour, on its whole frames, as hourly
//! preparation observes it.

use super::{Prepared, WATCHED_WHOLE};
use dashboard_base::prepared::{
    frame_changes::FrameChanges,
    kept::{Observed, consecutive, kept_changes, observe_hours},
    prepass::{Bound, Extract, Family, Folded},
};
use replay::{
    container::{BlockIndex, QuietHours, Replay, TableFields},
    format::FormatVersion,
    frame::{DecodedFrame, SharedView, TABLE_COUNT},
    rows::{TableId, tables},
};
use std::{
    any::Any,
    collections::BTreeSet,
    io,
    ops::Range,
    sync::{LazyLock, atomic::AtomicBool},
};

/// Each hour the family observes, counted in tests.
pub const OBSERVED: &str = "projects.observed";

/// Each quiet hour its fold observes on the last kept frame, counted in
/// tests.
pub const QUIET: &str = "projects.quiet";

/// Each block its fold takes from an extract that read the trigger kit,
/// counted in tests.
pub const KIT: &str = "projects.kit";

/// The tables the projects' observer reads: its project, program, history,
/// character and dictionary tables, the countries' keys and links and their
/// naming tables (`installed_content::country_names::TABLES`); with `world`, also
/// the shared trigger kit a scoped project name evaluates
/// (`triggers::RECORDED_TABLES`), which only content holding such a name
/// reads ([`Prepared::reads_trigger_world`]). The campaign, which every read
/// checks, is among them.
pub fn tables(world: bool) -> &'static [TableId] {
    fn read(world: bool) -> Vec<TableId> {
        let mut tables: Vec<_> = WATCHED_WHOLE
            .into_iter()
            .chain([TableId::Countries, TableId::Campaign])
            .chain(
                world
                    .then_some(installed_content::country_names::TABLES.as_slice())
                    .into_iter()
                    .flatten()
                    .copied(),
            )
            .chain(
                world
                    .then_some(dashboard_text::triggers::RECORDED_TABLES)
                    .into_iter()
                    .flatten()
                    .copied(),
            )
            .collect();
        tables.sort_unstable();
        tables.dedup();
        tables
    }
    static OWN: LazyLock<Vec<TableId>> = LazyLock::new(|| read(false));
    static WORLD: LazyLock<Vec<TableId>> = LazyLock::new(|| read(true));
    if world { &WORLD } else { &OWN }
}

/// The family: its block extract reads every table any content's
/// observation reads, the trigger kit included, unless its run reads as the
/// bound content can ([`bound`]).
pub static FAMILY: LazyLock<Family> = LazyLock::new(|| Family {
    name: "projects",
    extract: "projects.extract",
    sequential: "projects.sequential",
    fallback: "projects.fallback",
    tables: tables(true),
    fields: Some(|| fields(true)),
    start: |format| start(format, true),
});

/// The reads of a run whose bound content holds no scoped project name
/// (`world` false, [`Prepared::reads_trigger_world`]): the observer's tables
/// without the trigger kit, which nothing its observation evaluates reads.
/// None with a scoped name, which reads as [`FAMILY`] declares.
pub fn bound(world: bool) -> Option<Bound> {
    (!world).then(|| Bound {
        tables: tables(false).to_vec(),
        fields: fields(false).ok(),
        start: Box::new(|format| start(format, false)),
    })
}

/// The fields of the naming and trigger tables the declared reads of scoped
/// names read (`triggers/world/dependence.rs`, `scope.rs`,
/// `scripted_text/dependence.rs`, `character_scope::FIELDS`), but the few
/// that move every hour (a party's popularity, a state's manpower, an
/// order's path): a name reading one of those is evaluated on whole frames,
/// as is one reading a table watched only by its keys. This list decides
/// how often hours are quiet, never what is published.
const DECLARED: &[(TableId, &[&str])] = &[
    (
        TableId::CountryAppearance,
        &[
            "cosmetic_tag",
            "name_tag",
            "leader_ideology",
            "overlord",
            "original_tag",
            "original_tag_reference",
            "old_name",
            "old_adjective",
            "old_definite_name",
        ],
    ),
    (
        TableId::Politics,
        &[
            "ruling_party",
            "has_elections",
            "capital",
            "original_capital",
            "last_election",
        ],
    ),
    (
        TableId::PoliticalParties,
        &[
            "is_ruling",
            "token",
            "tag",
            "name_default",
            "name",
            "long_name",
        ],
    ),
    (TableId::Autonomy, &["state"]),
    (
        TableId::ReservedCountryNaming,
        &[
            "tag",
            "name_tag",
            "cosmetic_tag",
            "leader_ideology",
            "group",
            "overlord",
            "autonomy",
            "built",
            "original_tag",
            "original_tag_reference",
            "owned_state_count",
            "faction",
            "land_theatre_count",
        ],
    ),
    (TableId::FactionMembers, &["faction"]),
    (TableId::Factions, &["name", "template_token"]),
    (TableId::WarRelations, &["start_hour", "end_hour"]),
    (TableId::Subjects, &["start_hour", "end_hour"]),
    (TableId::ScriptVariables, &["entries", "seed"]),
    (TableId::ScriptFlags, &["value"]),
    (
        TableId::States,
        &[
            "owner",
            "controller",
            "continent",
            "custom_name",
            "owner_name",
        ],
    ),
    (TableId::Provinces, &["state", "owner", "controller"]),
    (TableId::Focus, &["tree"]),
    (TableId::CompletedFocuses, &["token"]),
    (
        TableId::CountryCounterInputs,
        &[
            "owned_state_count",
            "controlled_state_count",
            "owned_states",
            "controlled_states",
        ],
    ),
    (TableId::CountryCivilWars, &["target"]),
    (TableId::AdvisorTokens, &["token"]),
    (TableId::FactionAdvisors, &["token"]),
    (TableId::HiredAdvisors, &["idea"]),
    (TableId::ActiveIdeas, &["token"]),
    (TableId::PartyLeaders, &["character", "leader_id", "role"]),
    (TableId::CountryCharacters, &["character"]),
    (
        TableId::CountryWars,
        &["can_occupy_non_war", "enemies_read", "enemies", "ordered"],
    ),
    (
        TableId::DynamicModifiers,
        &["token", "scope_country", "scope_state"],
    ),
    (
        TableId::StateDynamicModifiers,
        &["token", "scope_country", "scope_state"],
    ),
    (TableId::StateCores, &["cores", "claims"]),
    (TableId::Nukes, &["amount_raw"]),
    (TableId::RelationOpinions, &["opinion"]),
    (TableId::ResearchTechnologies, &["classifier"]),
    (
        TableId::CountryFactories,
        &["civilian_levels", "naval_levels", "military_levels"],
    ),
    (TableId::DivisionTemplates, &["owner", "name", "name_key"]),
    (TableId::DivisionTemplateIcons, &["country"]),
    (TableId::CharacterRoleTraits, &["tokens", "definitions"]),
    (
        TableId::UnitLeaders,
        &[
            "character_id",
            "category",
            "owner",
            "skill_status",
            "skill_level",
        ],
    ),
    (TableId::UnitLeaderTraits, &["definition_ordinal"]),
    (
        TableId::CharacterOperatives,
        &[
            "resolved",
            "category",
            "owner",
            "skill_level",
            "trait_definitions",
        ],
    ),
    (TableId::CountryRetiredCharacters, &["character"]),
    (TableId::LandTheatres, &["owner", "ordinal"]),
    (
        TableId::OrdersGroups,
        &["is_army_group", "theatre", "theatre_ordinal"],
    ),
    (TableId::OrdersGroupPlans, &["primary_orders"]),
    (TableId::ArmyOrders, &["order_type", "active"]),
    (TableId::ArmyOrderPlans, &["successors"]),
    (
        TableId::StateResourceInputs,
        &[
            "gate",
            "global_factor",
            "origin",
            "infrastructure_level",
            "infrastructure_modifier",
            "supply_route",
        ],
    ),
    (
        TableId::StateResources,
        &["pool", "additive", "factor", "cost"],
    ),
    (TableId::StateBaseResources, &["amount"]),
    (TableId::StateBuildings, &["level", "healthy_level"]),
    (TableId::ProvinceBuildings, &["level"]),
    (
        TableId::CountryBuildingResources,
        &["building", "count", "resource", "amount"],
    ),
    (TableId::ResourceDefinitions, &["token", "active"]),
    (TableId::IntelligenceAgencies, &["created", "name"]),
    (TableId::ProvinceNames, &["name"]),
    (TableId::LocalPlayer, &["country", "observer_country"]),
];

/// What a block is read by: every table watched whole that the observer
/// reads of a frame, the countries' keys, the declared fields of the naming
/// and trigger tables ([`DECLARED`]) and the row keys of every other table
/// read (with `world`, of the trigger kit too) but the campaign, which the
/// reader checks on its own.
fn fields(world: bool) -> io::Result<Vec<TableFields>> {
    let mut watched = Vec::new();
    for &table in tables(world) {
        if table == TableId::Campaign {
            continue;
        }
        let watch = if WATCHED_WHOLE.contains(&table) {
            TableFields::all(table)
        } else if table == TableId::Countries {
            TableFields::named(table, &["canonical_slot", "tag"])?
        } else if let Some((_, names)) = DECLARED.iter().find(|(declared, _)| *declared == table) {
            TableFields::named(table, names)?
        } else {
            TableFields::keys(table)
        };
        watched.push(watch);
    }
    Ok(watched)
}

/// [`fields`], once each.
fn watched(world: bool) -> &'static [TableFields] {
    static OWN: LazyLock<Vec<TableFields>> = LazyLock::new(|| fields(false).unwrap_or_default());
    static WORLD: LazyLock<Vec<TableFields>> = LazyLock::new(|| fields(true).unwrap_or_default());
    if world { &WORLD } else { &OWN }
}

/// An extract of a block read with the trigger kit (`world`) or without it.
fn start(format: FormatVersion, world: bool) -> Box<dyn Extract> {
    Box::new(BlockExtract {
        format,
        world,
        hours: Vec::new(),
        last: None,
        built: Vec::new(),
        deadline: None,
        frames: 0,
        #[cfg(test)]
        history_scans: 0,
    })
}

/// How an observed hour's frame holds the hour's rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Held {
    /// Every table: the block reader read the hour, and no table holds rows
    /// built at an earlier hour.
    Whole,
    /// The block reader read the hour, but some tables hold rows built at an
    /// earlier hour it did not read, which may differ from the hour's in
    /// fields no one watches. Every table the hour's update rebuilt is the
    /// read's own and may have changed in any field; every other table did
    /// not change since the hour before.
    Read,
    /// Built on the last frame at an hour the block reader did not read:
    /// every table holds the last frame's rows, and one the hour's update
    /// rebuilt changed only in fields no one watches.
    Built,
}

/// One hour of a block's facts, in hour order.
enum Hour {
    /// An hour the block reader observed, with its frame and change facts,
    /// and how the frame holds the hour's rows.
    Observed {
        frame: DecodedFrame,
        changes: FrameChanges,
        held: Held,
    },
    /// Consecutive recorded hours from `first`, after the hour before them,
    /// at which no watched field changed: whether each read the projects,
    /// and the tables its update rebuilt.
    Quiet {
        first: i32,
        hours: Vec<(bool, Vec<TableId>)>,
    },
}

/// A block's hours as the extract kept them.
struct BlockFacts {
    hours: Vec<Hour>,
    /// Whether the block was read with the trigger kit, by its fields
    /// ([`watched`]).
    world: bool,
    /// Frames the extract was given or built.
    #[cfg_attr(not(test), allow(dead_code))]
    frames: usize,
}

struct BlockExtract {
    format: FormatVersion,
    /// Whether the block is read with the trigger kit.
    world: bool,
    hours: Vec<Hour>,
    /// The last frame observed.
    last: Option<DecodedFrame>,
    /// The tables whose rows in the last frame may not be those of the
    /// latest hour given: built at an hour the block reader did not read, or
    /// rebuilt by a quiet hour's update since.
    built: Vec<TableId>,
    /// The first completion date after the last hour observed.
    deadline: Option<i32>,
    frames: usize,
    #[cfg(test)]
    history_scans: usize,
}

impl BlockExtract {
    /// How `frame` holds its hour's rows, and the tables it holds rows of an
    /// earlier hour in. The block reader read the hour when it is the
    /// block's first, or a watched table holds other rows than the last
    /// frame did (it reads the hours a watched field changes); a frame
    /// built on the last one holds that one's rows in every table, each one
    /// its hour's update rebuilt at a new allocation. A frame read carries a
    /// table built earlier, and not decoded again since, at the allocation
    /// the last frame held it at; every other table at a new allocation is
    /// the read's own.
    fn held(&self, frame: &DecodedFrame) -> (Held, Vec<TableId>) {
        // Without a watch list every hour is read whole.
        let watched = watched(self.world);
        let Some(last) = self.last.as_ref().filter(|_| !watched.is_empty()) else {
            return (Held::Whole, Vec::new());
        };
        let read = watched.iter().any(|watch| {
            let index = watch.table.index();
            !SharedView::ptr_eq(&last.tables[index], &frame.tables[index])
                && last.tables[index] != frame.tables[index]
        });
        let built: Vec<TableId> = tables(self.world)
            .iter()
            .copied()
            .filter(|&table| {
                let index = table.index();
                if SharedView::ptr_eq(&last.tables[index], &frame.tables[index]) {
                    self.built.contains(&table)
                } else {
                    !read
                }
            })
            .collect();
        let held = match (read, built.is_empty()) {
            (false, _) => Held::Built,
            (true, true) => Held::Whole,
            (true, false) => Held::Read,
        };
        (held, built)
    }
}

impl Extract for BlockExtract {
    fn bytes(&self) -> u64 {
        let mut held = dashboard_base::prepared::prepass::HeldBytes::default();
        held.add(self.hours.capacity() * size_of::<Hour>());
        held.add(self.built.capacity() * size_of::<TableId>());
        for hour in &self.hours {
            match hour {
                Hour::Observed { frame, changes, .. } => {
                    held.frame(frame).changes(changes);
                }
                Hour::Quiet { hours, .. } => {
                    held.add(hours.capacity() * size_of::<(bool, Vec<TableId>)>());
                    for (_, rebuilt) in hours {
                        held.add(rebuilt.capacity() * size_of::<TableId>());
                    }
                }
            }
        }
        held.total()
    }

    fn observe(
        &mut self,
        _: &Replay,
        frame: &DecodedFrame,
        changes: &FrameChanges,
    ) -> io::Result<()> {
        self.frames += 1;
        let (held, built) = self.held(frame);
        self.built = built;
        // The history's completion dates after this hour, as the observer
        // finds its next one.
        if self.last.is_none()
            || changes.rows_changed[TableId::ProjectHistory.index()]
            || self.deadline.is_some_and(|date| date <= frame.hour)
        {
            #[cfg(test)]
            {
                self.history_scans += 1;
            }
            self.deadline = frame
                .complete::<tables::ProjectHistory>()
                .ok()
                .and_then(|rows| {
                    rows.iter()
                        .map(super::super::history_operand)
                        .filter(|&(_, completion, date)| completion && date > frame.hour)
                        .map(|(_, _, date)| date)
                        .min()
                });
        }
        self.last = Some(frame.clone());
        self.hours.push(Hour::Observed {
            frame: frame.clone(),
            changes: kept_changes(changes),
            held,
        });
        Ok(())
    }

    fn deadline(&self) -> Option<i32> {
        self.deadline
    }

    /// Each quiet hour's projects slot read, from its read evidence, and the
    /// tables its update rebuilt (fields no one watches), whose rows in the
    /// last frame are then not the latest hour's.
    fn quiet(&mut self, _: &Replay, hours: &QuietHours, span: Range<usize>) -> io::Result<()> {
        for &table in tables(self.world) {
            if !self.built.contains(&table) && span.clone().any(|at| hours.rebuilt(at, table)) {
                self.built.push(table);
            }
        }
        let Some((first, _)) = consecutive(hours, span.clone())? else {
            return Ok(());
        };
        let format = self.format;
        let quiet = span.map(|position| {
            let read = dashboard_text::metadata::slot_read(
                format,
                &hours.metrics()[position],
                TableId::Projects,
                hours.hours()[position],
            );
            let rebuilt = tables(self.world)
                .iter()
                .copied()
                .filter(|&table| hours.rebuilt(position, table))
                .collect();
            (read, rebuilt)
        });
        match self.hours.last_mut() {
            Some(Hour::Quiet {
                first: held,
                hours: kept,
            }) if i64::from(*held) + kept.len() as i64 == i64::from(first) => kept.extend(quiet),
            _ => self.hours.push(Hour::Quiet {
                first,
                hours: quiet.collect(),
            }),
        }
        Ok(())
    }

    fn finish(self: Box<Self>) -> io::Result<Box<dyn Any + Send>> {
        if self.hours.is_empty() {
            return Err(io::Error::other("Special projects block has no hours"));
        }
        Ok(Box::new(BlockFacts {
            hours: self.hours,
            world: self.world,
            frames: self.frames,
        }))
    }
}

/// The change facts of a quiet hour on the last kept frame: the tables its
/// update rebuilt, and its projects slot read.
fn quiet_changes(read: bool, rebuilt: &[TableId]) -> FrameChanges {
    let mut rows_changed = [false; TABLE_COUNT];
    for table in rebuilt {
        rows_changed[table.index()] = true;
    }
    let mut slot_read = [false; TABLE_COUNT];
    slot_read[TableId::Projects.index()] = read;
    FrameChanges {
        rows_changed,
        table_read: [false; TABLE_COUNT],
        slot_read,
        updates: None,
    }
}

/// The owners whose project rows differ between two kept frames, by key:
/// those a recorded delta of the table between their hours names (its
/// changed rows and deleted keys, keyed by owner, then project). None
/// unless both frames hold the whole table.
fn changed_project_owners(before: &DecodedFrame, after: &DecodedFrame) -> Option<BTreeSet<u32>> {
    let before = before.complete::<tables::Projects>().ok()?;
    let after = after.complete::<tables::Projects>().ok()?;
    let owner = |key: u128| (key >> 32) as u32;
    let mut owners = BTreeSet::new();
    let (mut old, mut new) = (before.iter().peekable(), after.iter().peekable());
    loop {
        match (old.peek(), new.peek()) {
            (None, None) => break,
            (Some(row), None) => {
                owners.insert(owner(row.key()));
                old.next();
            }
            (None, Some(row)) => {
                owners.insert(owner(row.key()));
                new.next();
            }
            (Some(was), Some(is)) => match was.key().cmp(&is.key()) {
                std::cmp::Ordering::Less => {
                    owners.insert(owner(was.key()));
                    old.next();
                }
                std::cmp::Ordering::Greater => {
                    owners.insert(owner(is.key()));
                    new.next();
                }
                std::cmp::Ordering::Equal => {
                    if was != is {
                        owners.insert(owner(is.key()));
                    }
                    old.next();
                    new.next();
                }
            },
        }
    }
    Some(owners)
}

/// Fold one block's facts into the carried observer, whose content the
/// caller bound for the block; `hour` is told of each hour observed.
/// Returns how it folded and the last hour it reached.
pub fn fold(
    prepared: &mut Prepared,
    facts: Box<dyn Any + Send>,
    replay: &Replay,
    index: &BlockIndex,
    observed: &mut Observed<'_>,
    check: &mut dyn FnMut() -> io::Result<()>,
    hour: &mut dyn FnMut(),
) -> io::Result<(Folded, Option<i32>)> {
    let facts = facts
        .downcast::<BlockFacts>()
        .map_err(|_| io::Error::other("Pre-pass facts of another family"))?;
    #[cfg(any(test, feature = "test-support"))]
    dashboard_base::prepared::kept::note_frames(FAMILY.name, facts.frames);
    if facts.world {
        dashboard_base::prepared::prepass::note(KIT);
    }
    // The fields the block was read by.
    let read = watched(facts.world);
    let running = AtomicBool::new(false);
    let mut reached = None;
    let mut last: Option<DecodedFrame> = None;
    // Observe one kept hour; None when it must be observed on its whole
    // frame, from where the rest of the block is.
    let kept = |prepared: &mut Prepared,
                frame: &DecodedFrame,
                changes: &FrameChanges,
                held: Held,
                owners: Option<BTreeSet<u32>>,
                observed: &mut Observed<'_>,
                hour: &mut dyn FnMut()|
     -> io::Result<Option<bool>> {
        // A built hour's rebuilt tables changed only in fields no one
        // watches; a read one's may have changed in any: it is told of no
        // watched field.
        let watched: &[TableFields] = match held {
            Held::Read => &[],
            Held::Whole | Held::Built => read,
        };
        let result = prepared.observe_kept(
            frame,
            changes,
            held == Held::Whole,
            watched,
            owners,
            &running,
        );
        if matches!(result, Ok(false)) {
            return Ok(None);
        }
        hour();
        observed(frame.hour, result.map(drop)).map(Some)
    };
    for kept_hour in facts.hours {
        check()?;
        match kept_hour {
            Hour::Observed {
                frame,
                changes,
                held,
            } => {
                let owners = last
                    .as_ref()
                    .and_then(|last| changed_project_owners(last, &frame));
                match kept(prepared, &frame, &changes, held, owners, observed, hour)? {
                    None => {
                        let at = frame.hour;
                        let reached = observe_block(
                            prepared,
                            replay,
                            index,
                            Some(at),
                            observed,
                            check,
                            hour,
                        )?;
                        return Ok((Folded::Fallback, reached));
                    }
                    Some(live) => {
                        reached = Some(frame.hour);
                        if !live {
                            return Ok((Folded::Fast, reached));
                        }
                    }
                }
                last = Some(frame);
            }
            Hour::Quiet { first, hours } => {
                let base = last.as_ref().ok_or_else(|| {
                    io::Error::other("A quiet hour came before the block's first")
                })?;
                for (offset, (read, rebuilt)) in hours.into_iter().enumerate() {
                    check()?;
                    // The span's hours are consecutive i32 hours.
                    let at = first + offset as i32;
                    let mut frame = base.clone();
                    frame.hour = at;
                    let changes = quiet_changes(read, &rebuilt);
                    match kept(
                        prepared,
                        &frame,
                        &changes,
                        Held::Built,
                        None,
                        observed,
                        hour,
                    )? {
                        None => {
                            let reached = observe_block(
                                prepared,
                                replay,
                                index,
                                Some(at),
                                observed,
                                check,
                                hour,
                            )?;
                            return Ok((Folded::Fallback, reached));
                        }
                        Some(live) => {
                            dashboard_base::prepared::prepass::note(QUIET);
                            reached = Some(at);
                            if !live {
                                return Ok((Folded::Fast, reached));
                            }
                        }
                    }
                }
            }
        }
    }
    Ok((Folded::Fast, reached))
}

/// Observe the block hour by hour from `from` (every hour without it), on
/// the block read alone with the tables its content reads and the change
/// facts of the block alone, as hourly preparation does; `hour` is told of
/// each hour observed. Returns the last hour observed.
pub fn observe_block(
    prepared: &mut Prepared,
    replay: &Replay,
    index: &BlockIndex,
    from: Option<i32>,
    observed: &mut Observed<'_>,
    check: &mut dyn FnMut() -> io::Result<()>,
    hour: &mut dyn FnMut(),
) -> io::Result<Option<i32>> {
    let running = AtomicBool::new(false);
    let world = prepared.reads_trigger_world();
    observe_hours(
        replay,
        index,
        tables(world),
        from,
        observed,
        check,
        |frame, changes| {
            hour();
            prepared.observe_changed(frame, changes, &running)
        },
    )
}

#[cfg(test)]
#[path = "block_tests.rs"]
mod tests;
