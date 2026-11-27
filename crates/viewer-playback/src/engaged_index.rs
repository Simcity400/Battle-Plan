
use playhead::Timeline;
use replay::campaign::Replay;
use replay_data::{BranchId, Hour, rows::TableId, rows::tables};
use replay_frame::{branches::BranchReason, branches::BranchStart, frame::DecodedFrame, frame::Tables};
use std::{
    collections::BTreeMap,
    io, mem,
    sync::{Arc, atomic::Ordering},
    time::Instant,
};
use viewer_engagements::engaged::HourSides;
use viewer_engagements::engagement_history::{
    Covered,
    Folded,
    History,
    ICON_KINDS,
    NAVAL,
    NAVAL_ICON,
    SOURCES,
    icon_hour,
    icon_source,
};
use viewer_foundation::fold_tables::available;
#[cfg(test)]
use viewer_engagements::engagement_history::{
    CombatFlags,
    LAND,
    Provenance,
    at,
    combats,
    kept_frames,
};
use viewer_engagements::{
    engaged::{self, Continuity, Engaged},
    icon_frames::{IconFold, Kept},
    naval_check::{self, ByteCheck, Summary},
};
use viewer_foundation::{
    perspective::Country,
    tables::{Available, core},
    visibility::{Relations, ZeroVisibility},
};

const ICON_TABLES: [TableId; 5] = [
    TableId::LandCombats,
    TableId::LandCombatSides,
    TableId::NavalCombats,
    TableId::NavalCombatShips,
    TableId::NavalCombatDetails,
];

const VIEWER_TABLES: [TableId; 12] = [
    TableId::Campaign,
    TableId::CountryLinks,
    TableId::Countries,
    TableId::LocalPlayer,
    TableId::Players,
    TableId::HumanControllers,
    TableId::FactionMembers,
    TableId::WarRelations,
    TableId::Subjects,
    TableId::CountryCounterInputs,
    TableId::ReservedCountryNaming,
    TableId::CombatEngagementRefreshes,
];

enum Witness {
    Unavailable,
    Complete {
        land: BTreeMap<(u64, u8), engaged::SideWitness>,
        naval: BTreeMap<(u64, u8), engaged::SideWitness>,
        controller_targets: Vec<u32>,
        local_viewers: Vec<u32>,
        cache_written_viewers: Vec<u32>,
    },
}

fn read_inputs<F: Tables + ?Sized>(frame: &F) -> io::Result<engaged::Snapshot> {
    let links: BTreeMap<u32, u32> = whole_core::<tables::CountryLinks, F>(frame)?.iter().map(|link| (link.id, link.canonical_slot)).collect();
    let relations = Relations::read_tables(frame, &links);
    let zero_visibility = ZeroVisibility::read_tables(frame, &links);
    let controllers = controllers(frame, &links)?;
    Ok(engaged::Snapshot { links, relations, zero_visibility, controllers })
}

fn push_unique(list: &mut Vec<u32>, value: u32) {
    if !list.contains(&value) {
        list.push(value);
    }
}

fn witness<F: Tables + ?Sized>(frame: &F) -> io::Result<Witness> {
    use replay_data::rows::CombatEngagementRefreshRow;
    let Available::Complete(rows) = available::<tables::CombatEngagementRefreshes, F>(frame)? else {
        return Ok(Witness::Unavailable);
    };
    let mut land = BTreeMap::<(u64, u8), engaged::SideWitness>::new();
    let mut naval = BTreeMap::<(u64, u8), engaged::SideWitness>::new();
    let mut controller_targets = Vec::new();
    let mut local_viewers = Vec::new();
    let mut cache_written_viewers = Vec::new();
    for row in rows {
        let recorded = (row.triggers & CombatEngagementRefreshRow::TRIGGER_RECORDED) != 0;
        let units = (row.triggers & CombatEngagementRefreshRow::TRIGGER_UNITS) != 0;
        match row.kind {
            CombatEngagementRefreshRow::LAND_SIDE => {
                if row.target == 0
                    || row.side > 1
                    || row.viewer > i32::MAX as u32
                    || row.triggers == 0
                    || (row.triggers & !3) != 0
                {
                    return Ok(Witness::Unavailable);
                }
                let entry = land.entry((row.target, row.side)).or_default();
                entry.lists.recorded |= recorded;
                entry.lists.units |= units;
                if recorded {
                    push_unique(&mut entry.recorded_viewers, row.viewer);
                }
                if units {
                    push_unique(&mut entry.units_viewers, row.viewer);
                }
            }
            CombatEngagementRefreshRow::NAVAL_SIDE => {
                if row.target == 0
                    || row.side > 1
                    || row.viewer > i32::MAX as u32
                    || row.triggers == 0
                    || (row.triggers & !3) != 0
                {
                    return Ok(Witness::Unavailable);
                }
                let entry = naval.entry((row.target, row.side)).or_default();
                entry.lists.recorded |= recorded;
                entry.lists.units |= units;
                if recorded {
                    push_unique(&mut entry.recorded_viewers, row.viewer);
                }
                if units {
                    push_unique(&mut entry.units_viewers, row.viewer);
                }
            }
            CombatEngagementRefreshRow::CONTROLLER_TARGET => {
                if row.side != 0
                    || row.viewer != 0
                    || row.target == 0
                    || row.target > i32::MAX as u64
                    || row.triggers != CombatEngagementRefreshRow::TRIGGER_RECORDED
                {
                    return Ok(Witness::Unavailable);
                }
                controller_targets.push(row.target as u32);
            }
            CombatEngagementRefreshRow::LOCAL_ALL_SIDES => {
                if row.side != 0
                    || row.viewer != 0
                    || row.target > i32::MAX as u64
                    || (row.triggers != CombatEngagementRefreshRow::TRIGGER_RECORDED
                        && row.triggers
                            != (CombatEngagementRefreshRow::TRIGGER_RECORDED
                                | CombatEngagementRefreshRow::TRIGGER_CACHE_WRITE))
                {
                    return Ok(Witness::Unavailable);
                }
                let viewer = row.target as u32;
                push_unique(&mut local_viewers, viewer);
                if (row.triggers & CombatEngagementRefreshRow::TRIGGER_CACHE_WRITE) != 0 {
                    push_unique(&mut cache_written_viewers, viewer);
                }
            }
            _ => return Ok(Witness::Unavailable),
        }
    }
    Ok(Witness::Complete {
        land,
        naval,
        controller_targets,
        local_viewers,
        cache_written_viewers,
    })
}

struct Viewer {
    slot: Option<u32>,
    raw: Option<u32>,
    alternate_slot: Option<u32>,
    alternate_raw: Option<u32>,
    inputs: Arc<engaged::Snapshot>,
}

fn controllers<F: Tables + ?Sized>(
    frame: &F,
    links: &BTreeMap<u32, u32>,
) -> io::Result<Option<engaged::Controllers>> {
    let (Available::Complete(players), Available::Complete(counts)) = (
        available::<tables::Players, F>(frame)?,
        available::<tables::HumanControllers, F>(frame)?,
    ) else {
        return Ok(None);
    };
    let mut out = engaged::Controllers {
        players: players
            .iter()
            .map(|player| {
                (
                    player.id,
                    (player.country, links.get(&player.country).copied()),
                )
            })
            .collect(),
        counts: BTreeMap::new(),
    };
    for row in counts {
        match links.get(&row.country) {
            Some(&slot) => *out.counts.entry(slot).or_default() += i64::from(row.count),
            None => return Ok(None),
        }
    }
    Ok(Some(out))
}

fn whole_core<T: replay_data::rows::TableSchema, F: Tables + ?Sized>(frame: &F) -> io::Result<&[T::Row]> {
    viewer_foundation::fold_tables::whole::<T, F>(frame).ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, format!("hour {} {:?} is not whole; the core tables carry the political map itself, so this hour cannot be drawn", frame.at_hour(), T::ID))
    })
}

impl Viewer {
    #[cfg(test)]
    fn read<F: Tables + ?Sized>(frame: &F, country: &Country) -> io::Result<Self> {
        Self::with(frame, country, &Arc::new(read_inputs(frame)?))
    }

    fn with<F: Tables + ?Sized>(frame: &F, country: &Country, inputs: &Arc<engaged::Snapshot>) -> io::Result<Self> {
        let countries = whole_core::<tables::Countries, F>(frame)?;
        let mut tags = BTreeMap::new();
        if let Some(lowest) = countries.iter().min_by_key(|c| c.canonical_slot) {
            tags.insert(lowest.canonical_slot, String::new());
        }
        if let Country::Tag(tag) = country
            && let Some(found) = countries
                .iter()
                .filter(|c| c.tag.eq_ignore_ascii_case(tag.as_bytes()))
                .min_by_key(|c| c.canonical_slot)
        {
            tags.insert(
                found.canonical_slot,
                String::from_utf8_lossy(&found.tag).into_owned(),
            );
        }
        if let Country::Slot { slot, .. } = country
            && countries.iter().any(|c| c.canonical_slot == *slot)
        {
            tags.insert(*slot, String::new());
        }
        let (slot, _) = viewer_foundation::fold_tables::viewer(frame, &inputs.links, &tags, country)?;
        Ok(Self {
            slot: Some(slot),
            raw: None,
            alternate_slot: None,
            alternate_raw: None,
            inputs: Arc::clone(inputs),
        })
    }

    fn recording<F: Tables + ?Sized>(
        frame: &F,
        inputs: &Arc<engaged::Snapshot>,
    ) -> (Self, Option<(Option<u32>, u32)>) {
        let local = viewer_foundation::fold_tables::whole::<tables::LocalPlayer, F>(frame)
            .and_then(|rows| match rows {
                [player] => {
                    let raw = if (player.country as i32) > 0 {
                        player.country
                    } else if (player.observer_country as i32) > 0 {
                        player.observer_country
                    } else {
                        0
                    };
                    let slot = if raw == 0 {
                        Some(0)
                    } else {
                        inputs.links.get(&raw).copied()
                    };
                    Some((slot, raw))
                }
                _ => None,
            });
        match local {
            Some((slot, raw)) => (
                Self {
                    slot,
                    raw: Some(raw),
                    alternate_slot: None,
                    alternate_raw: None,
                    inputs: Arc::clone(inputs),
                },
                Some((slot, raw)),
            ),
            None => (
                Self {
                    slot: None,
                    raw: None,
                    alternate_slot: None,
                    alternate_raw: None,
                    inputs: Arc::clone(inputs),
                },
                None,
            ),
        }
    }

    fn sample<'a>(&'a self, kind: usize, recording: bool, witness: &'a Witness, sides: HourSides) -> engaged::Sample<'a> {
        let refreshes = match witness {
            Witness::Unavailable => engaged::RefreshWitness::Unavailable,
            Witness::Complete {
                land,
                naval,
                controller_targets,
                local_viewers,
                cache_written_viewers,
            } => engaged::RefreshWitness::Complete {
                sides: if kind == NAVAL { naval } else { land },
                controller_targets: if recording { &[] } else { controller_targets },
                local_viewers: if recording { local_viewers } else { &[] },
                cache_written_viewers: if recording { cache_written_viewers } else { &[] },
            },
        };
        engaged::Sample {
            viewer: self.slot,
            viewer_raw: self.raw,
            alternate_viewer: self.alternate_slot,
            alternate_viewer_raw: self.alternate_raw,
            recording,
            snapshot: &self.inputs,
            refreshes,
            sides: sides.sides,
            complete: sides.complete,
            removal: sides.removal,
        }
    }
}

pub struct EngagedIndex {
    pub shared: Arc<History>,
}

impl Drop for EngagedIndex {
    fn drop(&mut self) {
        self.shared.cancelled.store(true, Ordering::Release);
    }
}

impl EngagedIndex {
    #[cfg(any(test, feature = "test-support"))]
    pub fn folded(frames: &[DecodedFrame], country: Country, load: Option<Hour>) -> Self {
        let shared = Arc::new(History::default());
        let mut pass = Pass::new(Arc::clone(&shared), country);
        for frame in frames {
            pass.fold_hour(frame.branch, frame.hour, Some(frame), load, false);
        }
        Self { shared }
    }
}

#[derive(Default)]
struct Track {
    histories: Vec<engaged::History>,
    changed: Vec<Vec<((u64, u8), Engaged)>>,
    ended: Vec<Vec<(u64, u8)>>,
}

impl Track {
    fn sized(&mut self) {
        self.histories.resize_with(SOURCES.len(), Default::default);
        self.changed.resize_with(SOURCES.len(), Vec::new);
        self.ended.resize_with(SOURCES.len(), Vec::new);
    }

    fn advance(&mut self, viewer: &Viewer, recording: bool, witness: &Witness, sides: Vec<HourSides>, continuity: Continuity) {
        self.sized();
        for (kind, sides) in sides.into_iter().enumerate() {
            self.histories[kind].advance_reporting(
                viewer.sample(kind, recording, witness, sides),
                continuity,
                &mut self.changed[kind],
                &mut self.ended[kind],
            );
        }
    }

    fn forget(&mut self) {
        self.sized();
        for ((history, changed), ended) in self
            .histories
            .iter_mut()
            .zip(&mut self.changed)
            .zip(&mut self.ended)
        {
            let held = mem::take(history);
            changed.clear();
            ended.clear();
            changed.extend(
                held.sides()
                    .filter(|(_, value)| *value != Engaged::Unknown)
                    .map(|(key, _)| (key, Engaged::Unknown)),
            );
        }
    }

    fn record(&self, fold: &mut Folded, hour: Hour) {
        for (kind, (changed, ended)) in self.changed.iter().zip(&self.ended).enumerate() {
            fold.record(kind, hour, changed, ended);
        }
    }
}

#[derive(Default)]
struct Fold {
    perspective: Track,
    last: Option<(Hour, bool)>,
    recording: Track,
    recording_last: Option<(Hour, bool)>,
    client_view: Option<(Option<u32>, u32, Arc<engaged::Snapshot>)>,
    icons: Vec<IconFold>,
    icon_changed: Vec<Vec<(u64, Option<Kept>)>>,
    recording_icons: Vec<IconFold>,
    recording_icon_changed: Vec<Vec<(u64, Option<Kept>)>>,
}

fn same_raw_in(links: &BTreeMap<u32, u32>, a: u32, b: u32) -> bool {
    a == b
        || (a != 0
            && b != 0
            && links
                .get(&a)
                .zip(links.get(&b))
                .is_some_and(|(sa, sb)| sa == sb))
}

fn advance_icons<F: Tables + ?Sized>(
    icons: &mut [IconFold],
    changed: &mut [Vec<(u64, Option<Kept>)>],
    folded: Option<(&F, &[engaged::History])>,
    bytes: Option<&BTreeMap<u64, [u8; 2]>>,
    continuity: Continuity,
) {
    for (kind, (icons, changed)) in icons.iter_mut().zip(changed).enumerate() {
        let Some((frame, histories)) = folded else {
            icons.lose(changed);
            continue;
        };
        let history = &histories[icon_source(kind)];
        let bytes = bytes.filter(|_| kind == NAVAL_ICON);
        let flag = |combat: u64| {
            let flag = history.combat(combat);
            bytes.map_or(flag, |bytes| naval_check::settle(flag, bytes.get(&combat)))
        };
        match icon_hour(kind, frame, &flag) {
            Ok(hour) => icons.advance(&hour, continuity, changed),
            Err(_) => icons.lose(changed),
        }
    }
}

fn continuity(
    last: Option<(Hour, bool)>,
    hour: Hour,
    load: Option<Hour>,
    restart: bool,
) -> Continuity {
    match last {
        _ if load == Some(hour) => Continuity::Load,
        _ if restart => Continuity::Gap,
        None => Continuity::Gap,
        Some((last, true)) if last.checked_add(1) == Some(hour) => Continuity::Next,
        Some(_) => Continuity::Gap,
    }
}

fn native_load(start: &BranchStart) -> Option<Hour> {
    (start.reason == BranchReason::LoadSignal && start.loaded_hour == Some(start.first_hour))
        .then_some(start.first_hour)
}

struct Pass {
    shared: Arc<History>,
    country: Country,
    folds: BTreeMap<BranchId, Fold>,
    check: ByteCheck,
    inputs: Option<Arc<engaged::Snapshot>>,
    witness: Option<Witness>,
    viewer: Option<Viewer>,
}

fn pass_tables() -> Vec<TableId> {
    let mut tables: Vec<TableId> = VIEWER_TABLES.to_vec();
    for source in SOURCES {
        tables.extend_from_slice(source.tables());
    }
    tables.extend_from_slice(&ICON_TABLES);
    tables.sort_by_key(|table| table.index());
    tables.dedup();
    tables
}

pub fn check_recorded_bytes(replay: &Replay) -> Summary {
    let mut pass = Pass::new(Arc::new(History::default()), Country::Recorded);
    pass.fold(replay, &pass_tables());
    pass.check.summary.complete = true;
    mem::take(&mut pass.check.summary)
}

impl Pass {
    fn new(shared: Arc<History>, country: Country) -> Self {
        Self {
            shared,
            country,
            folds: BTreeMap::new(),
            check: ByteCheck::default(),
            inputs: None,
            witness: None,
            viewer: None,
        }
    }

    fn cancelled(&self) -> bool {
        self.shared.cancelled.load(Ordering::Acquire)
    }

    fn publish(&self) {
        self.shared
            .check
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone_from(&self.check.summary);
    }

    fn fold(&mut self, replay: &Replay, tables: &[TableId]) -> usize {
        let _cost = replay_frame::cpu::scope("viewer", Some("engagement/strikes/text"));
        let mut blocks: Vec<_> = replay.index.iter().collect();
        blocks.sort_by_key(|block| (block.branch, block.first_hour));
        let mut starts: BTreeMap<(BranchId, Hour), bool> = BTreeMap::new();
        for history in &replay.branches {
            for (at, piece) in replay.pieces(history.id).iter().enumerate() {
                let load = replay
                    .segments_recorded()
                    .iter()
                    .find(|start| start.id == piece.segment)
                    .and_then(native_load)
                    .filter(|hour| (piece.first_hour..=piece.last_hour).contains(hour));
                match load {
                    Some(hour) => starts.insert((history.id, hour), true),
                    None if at > 0 => starts.insert((history.id, piece.first_hour), false),
                    None => None,
                };
            }
        }
        let mut hours = 0;
        for block in blocks {
            if self.cancelled() {
                return hours;
            }
            let folded = self
                .folds
                .get(&block.branch)
                .and_then(|fold| fold.last.map(|(hour, _)| hour));
            if folded.is_some_and(|last| block.last_hour <= last) {
                continue;
            }
            let mut stopped = false;
            let read = replay.read_block_alone(
                block,
                tables,
                |_, decoded| {
                    if stopped || self.cancelled() {
                        stopped = true;
                        return Ok(());
                    }
                    if folded.is_some_and(|last| decoded.hour <= last) {
                        return Ok(());
                    }
                    let campaign = core::<tables::Campaign>(decoded)
                        .is_ok_and(|rows| rows.len() == 1 && rows[0].id == replay.campaign_id);
                    let start = starts.get(&(block.branch, decoded.hour)).copied();
                    let load = (start == Some(true)).then_some(decoded.hour);
                    self.fold_hour(
                        block.branch,
                        decoded.hour,
                        campaign.then_some(decoded),
                        load,
                        start == Some(false),
                    );
                    hours += 1;
                    Ok(())
                },
                || Ok(()),
            );
            if stopped {
                return hours;
            }
            if read.is_err() {
                self.failed_block(block.branch, block.last_hour);
            }
        }
        hours
    }

    fn failed_block(&mut self, branch: BranchId, last_hour: Hour) {
        let fold = self.folds.entry(branch).or_default();
        fold.last = Some((last_hour, false));
        fold.recording_last = Some((last_hour, false));
        self.check.summary.failed_blocks += 1;
        self.publish();
    }

    fn fold_hour(
        &mut self,
        branch: BranchId,
        hour: Hour,
        frame: Option<&DecodedFrame>,
        load: Option<Hour>,
        restart: bool,
    ) {
        self.fold_source(branch, hour, frame, load, restart, false, false);
    }

    pub fn fold_source<F: Tables + ?Sized>(
        &mut self,
        branch: BranchId,
        hour: Hour,
        frame: Option<&F>,
        load: Option<Hour>,
        restart: bool,
        inputs_kept: bool,
        witness_kept: bool,
    ) {
        let fold = self.folds.entry(branch).or_default();
        fold.icons.resize_with(ICON_KINDS, Default::default);
        fold.icon_changed.resize_with(ICON_KINDS, Vec::new);
        fold.recording_icons
            .resize_with(ICON_KINDS, Default::default);
        fold.recording_icon_changed
            .resize_with(ICON_KINDS, Vec::new);
        let perspective_continuity = continuity(fold.last, hour, load, restart);
        let recording_continuity = continuity(fold.recording_last, hour, load, restart);
        let summary = &mut self.check.summary;
        summary.hours += 1;
        let sides = frame.zip(frame.and_then(|frame| {
            (0..SOURCES.len())
                .map(|kind| sides_of(kind, frame))
                .collect::<io::Result<Vec<_>>>()
                .ok()
        }));
        let held = self
            .inputs
            .take()
            .filter(|inputs| inputs_kept && !inputs.zero_visibility.reverse_stale_at(hour));
        let inputs = sides
            .as_ref()
            .and_then(|(frame, _)| held.or_else(|| read_inputs(*frame).ok().map(Arc::new)));
        self.inputs.clone_from(&inputs);
        let held_witness = self.witness.take().filter(|_| witness_kept);
        let witness = sides
            .as_ref()
            .filter(|_| inputs.is_some())
            .and_then(|(frame, _)| held_witness.or_else(|| witness(*frame).ok()));
        let held_viewer = self.viewer.take().filter(|viewer| {
            inputs_kept && inputs.as_ref().is_some_and(|inputs| Arc::ptr_eq(&viewer.inputs, inputs))
        });
        let viewer = held_viewer.or_else(|| {
            sides
                .as_ref()
                .zip(inputs.as_ref())
                .and_then(|((frame, _), inputs)| Viewer::with(*frame, &self.country, inputs).ok())
        });
        self.viewer = viewer.as_ref().map(|viewer| Viewer {
            slot: viewer.slot,
            raw: None,
            alternate_slot: None,
            alternate_raw: None,
            inputs: Arc::clone(&viewer.inputs),
        });
        let player = sides
            .as_ref()
            .zip(inputs.as_ref())
            .map(|((frame, _), inputs)| Viewer::recording(*frame, inputs));
        let previous_view = fold
            .client_view
            .as_ref()
            .filter(|_| recording_continuity == Continuity::Next);
        let player = player.map(|(mut viewer, view)| {
            if let Some((view_slot, view_raw)) = view
                && recording_continuity == Continuity::Next
            {
                if let Some((prev_slot, prev_raw, prev_inputs)) = previous_view {
                    let prev_slot_now = if *prev_raw == 0 {
                        Some(0)
                    } else {
                        viewer.inputs.links.get(prev_raw).copied()
                    };
                    viewer.slot = prev_slot_now.or(*prev_slot);
                    viewer.raw = Some(*prev_raw);
                    let same_view = same_raw_in(&prev_inputs.links, *prev_raw, view_raw)
                        && (Arc::ptr_eq(prev_inputs, &viewer.inputs)
                            || same_raw_in(&viewer.inputs.links, *prev_raw, view_raw));
                    if !same_view {
                        viewer.alternate_slot = view_slot;
                        viewer.alternate_raw = Some(view_raw);
                    }
                } else {
                    viewer.slot = None;
                    viewer.raw = None;
                    viewer.alternate_slot = view_slot;
                    viewer.alternate_raw = Some(view_raw);
                }
            }
            (viewer, view)
        });
        let identified = witness.is_some()
            && player
                .as_ref()
                .is_some_and(|(_, view)| view.is_some_and(|(slot, _)| slot.is_some()));
        match &sides {
            None => summary.unreadable_hours += 1,
            Some((_, sides)) => {
                let naval = &sides[NAVAL];
                if !naval.complete {
                    summary.incomplete_hours += 1;
                }
                if !naval.sides.is_empty() {
                    summary.naval_hours += 1;
                    if !identified {
                        summary.unidentified_hours += 1;
                    }
                }
            }
        }
        match (&sides, &player, &witness) {
            (Some((frame, sides)), Some((viewer, view)), Some(witness)) => {
                fold.recording
                    .advance(viewer, true, witness, sides.clone(), recording_continuity);
                fold.recording_last = Some((hour, true));
                fold.client_view = view.map(|(slot, raw)| (slot, raw, Arc::clone(&viewer.inputs)));
                let bytes = naval_check::recorded_bytes(frame).ok();
                advance_icons(
                    &mut fold.recording_icons,
                    &mut fold.recording_icon_changed,
                    Some((frame, &fold.recording.histories)),
                    bytes.as_ref(),
                    recording_continuity,
                );
                if identified
                    && self
                        .check
                        .compare(
                            branch,
                            hour,
                            frame,
                            &sides[NAVAL].sides,
                            &fold.recording.histories[NAVAL],
                        )
                        .is_err()
                {
                    self.check.summary.unreadable_hours += 1;
                }
            }
            _ => {
                fold.recording.forget();
                fold.recording_last = Some((hour, false));
                fold.client_view = None;
                advance_icons(
                    &mut fold.recording_icons,
                    &mut fold.recording_icon_changed,
                    None::<(&F, &[engaged::History])>,
                    None,
                    recording_continuity,
                );
            }
        }
        let slot = witness.as_ref().and_then(|_| viewer.as_ref()?.slot);
        match (sides, viewer, &witness) {
            (Some((frame, sides)), Some(viewer), Some(witness)) => {
                fold.perspective
                    .advance(&viewer, false, witness, sides, perspective_continuity);
                fold.last = Some((hour, true));
                advance_icons(
                    &mut fold.icons,
                    &mut fold.icon_changed,
                    Some((frame, &fold.perspective.histories)),
                    None,
                    perspective_continuity,
                );
            }
            _ => {
                fold.perspective.forget();
                fold.last = Some((hour, false));
                advance_icons(
                    &mut fold.icons,
                    &mut fold.icon_changed,
                    None::<(&F, &[engaged::History])>,
                    None,
                    perspective_continuity,
                );
            }
        }
        let player_slot = witness
            .as_ref()
            .and_then(|_| player.as_ref()?.1)
            .and_then(|(slot, raw)| (raw != 0).then_some(slot).flatten());
        self.witness = witness;
        {
            let mut branches = self
                .shared
                .branches
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            let covered = branches.entry(branch).or_insert_with(|| Covered::new(hour));
            covered.last = hour;
            if let Some(slot) = slot
                && covered.viewers.last().is_none_or(|(_, last)| *last != slot)
            {
                covered.viewers.push((hour, slot));
            }
            if covered
                .players
                .last()
                .is_none_or(|(_, last)| *last != player_slot)
            {
                covered.players.push((hour, player_slot));
            }
            fold.perspective.record(covered.fold(false), hour);
            fold.recording.record(covered.fold(true), hour);
            for (recording, changed) in [(false, &fold.icon_changed), (true, &fold.recording_icon_changed)] {
                let track = covered.fold(recording);
                for (kind, changed) in changed.iter().enumerate() {
                    track.record_icons(kind, hour, changed);
                }
            }
        }
        self.shared.folded();
        self.publish();
    }
}

fn sides_of<F: Tables + ?Sized>(kind: usize, frame: &F) -> io::Result<HourSides> {
    if kind == NAVAL {
        viewer_engagements::naval_sides::naval_sides(frame)
    } else {
        viewer_engagements::engagement_progress::land_sides(frame)
    }
}

fn piece_blocks(index: &[replay::campaign::BlockIndex], run: usize, from: Hour, to: Hour) -> Vec<&replay::campaign::BlockIndex> {
    let mut blocks: Vec<&replay::campaign::BlockIndex> = index.iter().filter(|block| block.run as usize == run && block.last_hour >= from && block.first_hour <= to).collect();
    blocks.sort_by_key(|block| (block.first_hour, block.page));
    blocks.dedup_by_key(|block| block.page);
    blocks
}

pub fn fold_range(replay: &Replay, country: Country, timeline: &Timeline, branch: BranchId, first: Hour, last: Hour) -> (Arc<History>, u64, u64, u64) {
    let shared = Arc::new(History::default());
    let mut pass = Pass::new(Arc::clone(&shared), country);
    let tables = pass_tables();
    let (mut total_ns, mut fold_ns, mut failed) = (0u64, 0u64, 0u64);
    for piece in timeline.pieces() {
        let (from, to) = (first.max(piece.first), last.min(piece.last));
        if from > to {
            continue;
        }
        for block in piece_blocks(&replay.index, piece.run, from, to) {
            let began = Instant::now();
            let read = replay.read_block_alone(
                block,
                &tables,
                |_, decoded| {
                    if !(from..=to).contains(&decoded.hour) {
                        return Ok(());
                    }
                    let folding = Instant::now();
                    let campaign = core::<tables::Campaign>(decoded).is_ok_and(|rows| rows.len() == 1 && rows[0].id == replay.campaign_id);
                    pass.fold_hour(branch, decoded.hour, campaign.then_some(decoded), None, false);
                    fold_ns += folding.elapsed().as_nanos() as u64;
                    Ok(())
                },
                || Ok(()),
            );
            if read.is_err() {
                failed += 1;
                pass.failed_block(branch, block.last_hour);
            }
            total_ns += began.elapsed().as_nanos() as u64;
        }
    }
    (shared, total_ns.saturating_sub(fold_ns), fold_ns, failed)
}

pub mod cursor;
pub mod lanes;

#[cfg(test)]
mod tests;
