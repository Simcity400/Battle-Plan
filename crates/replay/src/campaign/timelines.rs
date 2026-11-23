
use super::{BlockIndex, Replay, SeekResult};
use replay_data::{BranchId, Hour, codec::invalid_static, rows::{ClockRow, TableId, TableRows}};
use replay_frame::{branches::{BranchReason, BranchStart, Gap, Parentage}, frame::TableView};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Evidence {
    Counters,
    Only,
    Process,
    Unrecorded,
    Unconfirmed,
    Chosen,
}

impl Evidence {
    pub fn open(self) -> bool {
        matches!(self, Self::Unconfirmed | Self::Unrecorded | Self::Chosen)
    }
}

pub type Choices = BTreeMap<BranchId, BranchId>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub segment: BranchId,
    pub parent: Option<BranchId>,
    pub evidence: Evidence,
    pub candidates: Vec<BranchId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Piece {
    pub segment: BranchId,
    pub first_hour: Hour,
    pub last_hour: Hour,
    pub declared: Hour,
}

pub fn command_hours(pieces: &[Piece]) -> Vec<(Hour, Hour)> {
    pieces
        .iter()
        .enumerate()
        .map(|(at, piece)| {
            let last = pieces
                .get(at + 1)
                .map_or(piece.last_hour, |next| piece.last_hour.min(next.declared - 1));
            (piece.declared.min(piece.first_hour), last)
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Join {
    pub hour: Hour,
    pub segment: BranchId,
    pub parent: BranchId,
    pub evidence: Evidence,
    pub candidates: Vec<BranchId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Timeline {
    pub id: BranchId,
    pub pieces: Vec<Piece>,
    pub joins: Vec<Join>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Parting {
    Main,
    LeftBehind(Hour),
    Separate,
}

impl Timeline {
    pub fn first_hour(&self) -> Hour {
        self.pieces[0].first_hour
    }

    pub fn last_hour(&self) -> Hour {
        self.pieces[self.pieces.len() - 1].last_hour
    }

    pub fn piece_at(&self, hour: Hour) -> Option<&Piece> {
        let at = self.pieces.partition_point(|piece| piece.last_hour < hour);
        self.pieces.get(at).filter(|piece| piece.first_hour <= hour)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Timelines {
    pub list: Vec<Timeline>,
    pub main: BranchId,
    pub links: Vec<Link>,
    homes: BTreeMap<BranchId, BranchId>,
}

impl Timelines {
    pub fn get(&self, id: BranchId) -> Option<&Timeline> {
        self.list
            .binary_search_by_key(&id, |timeline| timeline.id)
            .ok()
            .map(|at| &self.list[at])
    }

    pub fn parting(&self, id: BranchId) -> Option<Parting> {
        let timeline = self.get(id)?;
        if id == self.main {
            return Some(Parting::Main);
        }
        let main = self.get(self.main)?;
        for (at, (own, held)) in timeline.pieces.iter().zip(&main.pieces).enumerate() {
            if own.segment != held.segment || own.first_hour != held.first_hour {
                return Some(if at == 0 {
                    Parting::Separate
                } else {
                    Parting::LeftBehind(own.first_hour.min(held.first_hour))
                });
            }
            if own.last_hour != held.last_hour {
                return Some(Parting::LeftBehind(own.last_hour.min(held.last_hour) + 1));
            }
        }
        let shared = timeline.pieces.len().min(main.pieces.len());
        Some(Parting::LeftBehind(timeline.pieces[shared - 1].last_hour + 1))
    }

    pub fn segment_at(&self, id: BranchId, hour: Hour) -> Option<BranchId> {
        self.get(id)?.piece_at(hour).map(|piece| piece.segment)
    }

    pub fn home(&self, segment: BranchId) -> Option<BranchId> {
        self.homes.get(&segment).copied()
    }
}

pub fn choices_path(campaign: &[u8]) -> PathBuf {
    let digest = blake3::hash(campaign);
    replay_settings::recorder_settings::histories_folder()
        .join(format!("{}.json", &digest.to_hex()[..32]))
}

pub fn read_choices(campaign: &[u8]) -> Choices {
    read_choices_at(&choices_path(campaign))
}

fn read_choices_at(path: &Path) -> Choices {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<BTreeMap<String, String>>(&bytes).ok())
        .map(|held| {
            held.into_iter()
                .filter_map(|(segment, parent)| Some((segment.parse().ok()?, parent.parse().ok()?)))
                .collect()
        })
        .unwrap_or_default()
}

pub fn write_choice(campaign: &[u8], segment: BranchId, parent: Option<BranchId>) -> io::Result<()> {
    let path = choices_path(campaign);
    let mut choices = read_choices_at(&path);
    match parent {
        Some(parent) => choices.insert(segment, parent),
        None => choices.remove(&segment),
    };
    let held: BTreeMap<String, String> = choices
        .into_iter()
        .map(|(segment, parent)| (segment.to_string(), parent.to_string()))
        .collect();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let staged = path.with_extension("json.new");
    std::fs::write(&staged, serde_json::to_vec(&held).map_err(io::Error::other)?)?;
    std::fs::rename(staged, path)
}

type Counters = [Option<u32>; 13];

fn counters(clock: &ClockRow) -> Counters {
    let signed = |value: Option<i32>| value.map(|value| value as u32);
    [
        clock.debug_current_ref_id,
        signed(clock.unit),
        signed(clock.order_index),
        signed(clock.front_index),
        signed(clock.theatre_index),
        signed(clock.theater_group_index),
        signed(clock.military_deployment_line_index),
        signed(clock.military_deployment_conveyor_index),
        signed(clock.equipment_variant_index),
        signed(clock.country_leader_index),
        signed(clock.navy_id),
        signed(clock.land_combat_id),
        signed(clock.multiplayer_random_count),
    ]
}

fn advance(from: u32, to: u32) -> i64 {
    i64::from(to.wrapping_sub(from) as i32)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Verdict {
    Unknown,
    Contradicts,
    Allows,
}

fn verdict(before: &Counters, seen: &Counters) -> Verdict {
    let mut compared = false;
    for k in 0..before.len() {
        let (Some(from), Some(now)) = (before[k], seen[k]) else {
            continue;
        };
        compared = true;
        if advance(from, now) < 0 {
            return Verdict::Contradicts;
        }
    }
    if compared {
        Verdict::Allows
    } else {
        Verdict::Unknown
    }
}

struct Extent {
    start: BranchStart,
    first: Hour,
    last: Hour,
    recorded: Vec<(Hour, Hour)>,
}

impl Extent {
    fn records(&self, hour: Hour) -> bool {
        let at = self.recorded.partition_point(|(_, last)| *last < hour);
        self.recorded.get(at).is_some_and(|(first, _)| *first <= hour)
    }

    fn recorded_before(&self, hour: Hour) -> Option<Hour> {
        let at = self.recorded.partition_point(|(first, _)| *first < hour);
        at.checked_sub(1)
            .map(|at| self.recorded[at].1.min(hour - 1))
    }

    fn first_recorded(&self) -> Option<Hour> {
        self.recorded.first().map(|(first, _)| *first)
    }
}

fn extents(replay: &Replay) -> io::Result<(Vec<Extent>, Vec<(BranchId, Hour, Hour)>)> {
    let mut held: BTreeMap<BranchId, Extent> = BTreeMap::new();
    for start in &replay.branches {
        held.insert(
            start.id,
            Extent {
                start: start.clone(),
                first: Hour::MAX,
                last: Hour::MIN,
                recorded: Vec::new(),
            },
        );
    }
    for block in &replay.index {
        let Some(extent) = held.get_mut(&block.branch) else {
            continue;
        };
        engine_allocation::reserve(&mut extent.recorded, 1)?;
        extent.recorded.push((block.first_hour, block.last_hour));
        extent.first = extent.first.min(block.first_hour);
        extent.last = extent.last.max(block.last_hour);
    }
    for gap in &replay.gaps {
        if let Some(extent) = held.get_mut(&gap.branch) {
            extent.first = extent.first.min(gap.first_hour);
            extent.last = extent.last.max(gap.last_hour);
        }
    }
    let mut out = Vec::with_capacity(held.len());
    let mut unseen = Vec::new();
    for mut extent in held.into_values() {
        if extent.recorded.is_empty() {
            if extent.first <= extent.last {
                engine_allocation::reserve(&mut unseen, 1)?;
                unseen.push((extent.start.id, extent.first, extent.last));
            }
            continue;
        }
        extent.recorded.sort_unstable();
        out.push(extent);
    }
    Ok((out, unseen))
}

struct Clocks<'a> {
    replay: &'a Replay,
    read: HashMap<(BranchId, Hour), Option<Counters>>,
}

impl Clocks<'_> {
    fn at(&mut self, segment: BranchId, hour: Hour) -> Option<Counters> {
        let replay = self.replay;
        *self.read.entry((segment, hour)).or_insert_with(|| {
            match replay.seek_selected(segment, hour, Some(&[TableId::Clock])).ok()? {
                SeekResult::Frame(frame) => match &*frame.tables[TableId::Clock.index()] {
                    TableView::Complete(TableRows::Clock(rows)) => rows.first().map(counters),
                    _ => None,
                },
                SeekResult::Gap { .. } => None,
            }
        })
    }
}

fn link(
    replay: &Replay,
    extents: &[Extent],
    unseen: &[(BranchId, Hour, Hour)],
    choices: &Choices,
) -> Vec<Link> {
    let mut clocks = Clocks {
        replay,
        read: HashMap::new(),
    };
    let starts: Vec<BranchId> = replay.branches.iter().map(|start| start.id).collect();
    let mut links = Vec::with_capacity(extents.len());
    for (at, extent) in extents.iter().enumerate() {
        let id = extent.start.id;
        let earlier = &extents[..at];
        let mut found = found_link(&mut clocks, earlier, unseen, &starts, extent);
        if let Some(parent) = choices.get(&id)
            && found.candidates.contains(parent)
        {
            found.parent = Some(*parent);
            found.evidence = Evidence::Chosen;
        }
        links.push(found);
    }
    links
}

fn found_link(
    clocks: &mut Clocks<'_>,
    earlier: &[Extent],
    unseen: &[(BranchId, Hour, Hour)],
    starts: &[BranchId],
    extent: &Extent,
) -> Link {
    let id = extent.start.id;
    let cut = extent.first;
    let none = Link {
        segment: id,
        parent: None,
        evidence: Evidence::Unconfirmed,
        candidates: Vec::new(),
    };
    let Some(seen_hour) = extent.first_recorded() else {
        return none;
    };
    let saved = extent
        .start
        .loaded_hour
        .filter(|hour| *hour < seen_hour)
        .unwrap_or(seen_hour.saturating_sub(1));
    let seen = clocks.at(id, seen_hour);

    if extent.start.reason == BranchReason::HourDiscontinuity
        && let Some(before) = earlier.last()
        && before.start.session == extent.start.session
        && before.last < cut
        && !starts
            .iter()
            .any(|&held| before.start.id < held && held < id)
    {
        return Link {
            segment: id,
            parent: Some(before.start.id),
            evidence: Evidence::Process,
            candidates: Vec::new(),
        };
    }

    let judge = |clocks: &mut Clocks<'_>, held: &Extent, before: Hour| -> Verdict {
        if !held.records(before) {
            return Verdict::Unknown;
        }
        let Some(seen) = seen else {
            return Verdict::Unknown;
        };
        let Some(from) = clocks.at(held.start.id, before) else {
            return Verdict::Unknown;
        };
        verdict(&from, &seen)
    };

    let played: Vec<(&Extent, Verdict)> = earlier
        .iter()
        .filter(|held| held.first <= saved && saved <= held.last)
        .map(|held| (held, judge(clocks, held, saved)))
        .collect();
    if !played.is_empty() {
        let allowed: Vec<BranchId> = played
            .iter()
            .filter(|(_, verdict)| *verdict == Verdict::Allows)
            .map(|(held, _)| held.start.id)
            .collect();
        let open: Vec<BranchId> = played
            .iter()
            .filter(|(_, verdict)| *verdict != Verdict::Contradicts)
            .map(|(held, _)| held.start.id)
            .collect();
        let gapped = unseen
            .iter()
            .any(|&(held, first, last)| held < id && first <= saved && saved <= last);
        if allowed.len() == 1 && open.len() == 1 && !gapped {
            return Link {
                segment: id,
                parent: Some(allowed[0]),
                evidence: if played.len() == 1 {
                    Evidence::Only
                } else {
                    Evidence::Counters
                },
                candidates: Vec::new(),
            };
        }
        let candidates = if open.is_empty() {
            played.iter().map(|(held, _)| held.start.id).collect()
        } else {
            open
        };
        return Link {
            segment: id,
            parent: candidates.last().copied(),
            evidence: Evidence::Unconfirmed,
            candidates,
        };
    }

    let before: Vec<(&Extent, Verdict)> = earlier
        .iter()
        .filter_map(|held| {
            let last = held.recorded_before(cut)?;
            Some((held, judge(clocks, held, last)))
        })
        .collect();
    let allowed: Vec<BranchId> = before
        .iter()
        .filter(|(_, verdict)| *verdict != Verdict::Contradicts)
        .map(|(held, _)| held.start.id)
        .collect();
    let candidates = if allowed.is_empty() {
        before.iter().map(|(held, _)| held.start.id).collect()
    } else {
        allowed
    };
    match candidates.last() {
        Some(parent) => Link {
            segment: id,
            parent: Some(*parent),
            evidence: Evidence::Unrecorded,
            candidates,
        },
        None => none,
    }
}

fn join(extents: &[Extent], links: Vec<Link>) -> io::Result<Timelines> {
    let mut list: Vec<Timeline> = Vec::new();
    let mut home: HashMap<BranchId, usize> = HashMap::new();
    for (extent, link) in extents.iter().zip(&links) {
        let own = Piece {
            segment: extent.start.id,
            first_hour: extent.first,
            last_hour: extent.last,
            declared: extent.start.first_hour.min(extent.first),
        };
        let parent = link.parent.and_then(|parent| {
            let at = *home.get(&parent)?;
            let through = list[at]
                .pieces
                .iter()
                .rposition(|piece| piece.segment == parent && piece.first_hour < own.first_hour)?;
            Some((at, through))
        });
        let Some((at, through)) = parent else {
            home.insert(own.segment, list.len());
            engine_allocation::reserve(&mut list, 1)?;
            list.push(Timeline {
                id: own.segment,
                pieces: vec![own],
                joins: Vec::new(),
            });
            continue;
        };
        let join = Join {
            hour: own.first_hour,
            segment: own.segment,
            parent: link.parent.unwrap_or_default(),
            evidence: link.evidence,
            candidates: link.candidates.clone(),
        };
        if through + 1 == list[at].pieces.len() && list[at].last_hour() < own.first_hour {
            list[at].pieces.push(own);
            list[at].joins.push(join);
            home.insert(own.segment, at);
            continue;
        }
        let from = &list[at];
        let mut pieces = Vec::with_capacity(through + 2);
        for piece in &from.pieces[..=through] {
            pieces.push(Piece {
                last_hour: piece.last_hour.min(own.first_hour - 1),
                ..*piece
            });
        }
        pieces.push(own);
        let mut joins = Vec::with_capacity(through + 1);
        joins.extend_from_slice(&from.joins[..through]);
        joins.push(join);
        home.insert(own.segment, list.len());
        engine_allocation::reserve(&mut list, 1)?;
        list.push(Timeline {
            id: own.segment,
            pieces,
            joins,
        });
    }
    let main = extents
        .last()
        .and_then(|extent| home.get(&extent.start.id))
        .map(|at| list[*at].id)
        .ok_or_else(|| invalid_static("the recording holds no recorded hour"))?;
    let homes = home
        .into_iter()
        .map(|(segment, at)| (segment, list[at].id))
        .collect();
    list.sort_unstable_by_key(|timeline| timeline.id);
    Ok(Timelines {
        list,
        main,
        links,
        homes,
    })
}

#[derive(Debug)]
pub(crate) struct Physical {
    pub(crate) index: Vec<BlockIndex>,
    pub(crate) segments: Vec<u32>,
    pub(crate) branches: Vec<BranchStart>,
    pub(crate) timelines: Timelines,
}

fn clip<T>(
    pieces: &[Piece],
    items: &[T],
    of: impl Fn(&T) -> (BranchId, Hour, Hour),
    mut keep: impl FnMut(&T, Hour, Hour) -> io::Result<()>,
) -> io::Result<()> {
    for piece in pieces {
        for item in items {
            let (segment, first, last) = of(item);
            if segment != piece.segment || last < piece.first_hour || first > piece.last_hour {
                continue;
            }
            keep(item, first.max(piece.first_hour), last.min(piece.last_hour))?;
        }
    }
    Ok(())
}

const UNRECORDED: &str = "Not recorded: this play happened while the recorder wasn't watching";

impl Replay {
    pub fn open_timelines(path: &Path) -> io::Result<Self> {
        let replay = Self::open_path(path)?;
        let choices = read_choices(&replay.campaign_id);
        replay.into_timelines(&choices)
    }

    pub fn into_timelines(mut self, choices: &Choices) -> io::Result<Self> {
        if self.physical.is_some() {
            return Err(invalid_static("the recording is already read as timelines"));
        }
        let (extents, unseen) = extents(&self)?;
        if extents.is_empty() {
            return Ok(self);
        }
        let links = link(&self, &extents, &unseen, choices);
        let timelines = join(&extents, links)?;

        let mut branches = Vec::with_capacity(timelines.list.len());
        let mut index = Vec::new();
        let mut gaps = Vec::new();
        let mut stops = Vec::new();
        let mut losses = Vec::new();
        for timeline in &timelines.list {
            let first = &timeline.pieces[0];
            let start = self
                .branches
                .iter()
                .find(|start| start.id == first.segment)
                .ok_or_else(|| invalid_static("a timeline starts with no branch"))?;
            branches.push(BranchStart {
                id: timeline.id,
                session: start.session,
                loaded_hour: start.loaded_hour,
                first_hour: first.declared.min(first.first_hour),
                parent: Parentage::Unknown,
                reason: start.reason,
            });
            clip(
                &timeline.pieces,
                &self.index,
                |block| (block.branch, block.first_hour, block.last_hour),
                |block, first_hour, last_hour| {
                    engine_allocation::reserve(&mut index, 1)?;
                    index.push(BlockIndex {
                        branch: timeline.id,
                        first_hour,
                        last_hour,
                        ..*block
                    });
                    Ok(())
                },
            )?;
            clip(
                &timeline.pieces,
                &self.gaps,
                |gap| (gap.branch, gap.first_hour, gap.last_hour),
                |gap, first_hour, last_hour| {
                    engine_allocation::reserve(&mut gaps, 1)?;
                    gaps.push(Gap {
                        branch: timeline.id,
                        first_hour,
                        last_hour,
                        reason: engine_allocation::string(&gap.reason)?,
                    });
                    Ok(())
                },
            )?;
            for pair in timeline.pieces.windows(2) {
                if pair[1].first_hour > pair[0].last_hour + 1 {
                    engine_allocation::reserve(&mut gaps, 1)?;
                    gaps.push(Gap {
                        branch: timeline.id,
                        first_hour: pair[0].last_hour + 1,
                        last_hour: pair[1].first_hour - 1,
                        reason: UNRECORDED.into(),
                    });
                }
            }
            let last = timeline.pieces[timeline.pieces.len() - 1].segment;
            for stop in self.stops.iter().filter(|stop| stop.branch == last) {
                engine_allocation::reserve(&mut stops, 1)?;
                stops.push(super::CaptureStop {
                    branch: timeline.id,
                    ..stop.clone()
                });
            }
            for control in &self.controls {
                if let replay_recording::control::ControlRecord::EventLoss {
                    session,
                    branch,
                    after_hour,
                    hour,
                } = *control
                    && timeline
                        .piece_at(hour)
                        .is_some_and(|piece| piece.segment == branch)
                {
                    engine_allocation::reserve(&mut losses, 1)?;
                    losses.push(replay_recording::control::ControlRecord::EventLoss {
                        session,
                        branch: timeline.id,
                        after_hour,
                        hour,
                    });
                }
            }
        }
        self.controls
            .retain(|control| !matches!(control, replay_recording::control::ControlRecord::EventLoss { .. }));
        engine_allocation::reserve(&mut self.controls, losses.len())?;
        self.controls.extend(losses);
        self.partial_control = self
            .controls
            .iter()
            .position(|control| matches!(control, replay_recording::control::ControlRecord::Partial { .. }));

        self.gaps = gaps;
        let physical = Physical {
            index: std::mem::replace(&mut self.index, index),
            segments: std::mem::take(&mut self.segments),
            branches: std::mem::replace(&mut self.branches, branches),
            timelines,
        };
        self.stops = stops;
        self.physical = Some(Box::new(physical));
        Ok(self)
    }

    pub fn timelines(&self) -> Option<&Timelines> {
        self.physical.as_ref().map(|physical| &physical.timelines)
    }

    pub fn segments_recorded(&self) -> &[BranchStart] {
        self.physical
            .as_ref()
            .map_or(&self.branches, |physical| &physical.branches)
    }

    pub fn segment_at(&self, branch: BranchId, hour: Hour) -> Option<BranchId> {
        match &self.physical {
            Some(physical) => physical.timelines.segment_at(branch, hour),
            None => Some(branch),
        }
    }

    pub fn pieces(&self, branch: BranchId) -> Vec<Piece> {
        match &self.physical {
            Some(physical) => physical
                .timelines
                .get(branch)
                .map(|timeline| timeline.pieces.clone())
                .unwrap_or_default(),
            None => self
                .branches
                .iter()
                .find(|start| start.id == branch)
                .map(|start| {
                    let mut first = start.first_hour;
                    let mut last = start.first_hour;
                    for block in self.index.iter().filter(|block| block.branch == branch) {
                        first = first.min(block.first_hour);
                        last = last.max(block.last_hour);
                    }
                    vec![Piece {
                        segment: branch,
                        first_hour: first,
                        last_hour: last,
                        declared: start.first_hour,
                    }]
                })
                .unwrap_or_default(),
        }
    }

    pub fn pages(&self) -> &[BlockIndex] {
        self.physical
            .as_ref()
            .map_or(&self.index, |physical| &physical.index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clock(count: u32, random: i32) -> Counters {
        let mut held = [Some(count); 13];
        held[12] = Some(random as u32);
        held
    }

    #[test]
    fn counters_only_ever_rule_a_history_out() {
        let before = clock(100, -5);
        assert_eq!(verdict(&before, &clock(110, 61)), Verdict::Allows);
        assert_eq!(verdict(&before, &clock(99, 61)), Verdict::Contradicts);
        assert_eq!(
            verdict(&clock(100, i32::MAX - 3), &clock(101, i32::MIN + 4)),
            Verdict::Allows
        );
        let mut unread = [None; 13];
        assert_eq!(verdict(&unread, &clock(1, 1)), Verdict::Unknown);
        unread[3] = Some(5);
        assert_eq!(verdict(&unread, &clock(4, 1)), Verdict::Contradicts);
    }

    fn extent(id: BranchId, session: u64, reason: BranchReason, hours: (Hour, Hour)) -> Extent {
        Extent {
            start: BranchStart {
                id,
                session,
                loaded_hour: None,
                first_hour: hours.0,
                parent: Parentage::Unknown,
                reason,
            },
            first: hours.0,
            last: hours.1,
            recorded: vec![hours],
        }
    }

    fn linked(segment: BranchId, parent: Option<BranchId>, evidence: Evidence) -> Link {
        Link {
            segment,
            parent,
            evidence,
            candidates: Vec::new(),
        }
    }

    #[test]
    fn a_reload_into_the_past_forks_and_a_continuation_extends() {
        let extents = [
            extent(1, 1, BranchReason::SessionStart, (0, 100)),
            extent(2, 1, BranchReason::LoadSignal, (51, 120)),
            extent(3, 2, BranchReason::SessionStart, (121, 200)),
            extent(4, 2, BranchReason::LoadSignal, (81, 90)),
        ];
        let links = vec![
            linked(1, None, Evidence::Unconfirmed),
            linked(2, Some(1), Evidence::Counters),
            linked(3, Some(2), Evidence::Only),
            linked(4, Some(2), Evidence::Counters),
        ];
        let timelines = join(&extents, links).unwrap();
        assert_eq!(timelines.main, 4);
        let ids: Vec<_> = timelines.list.iter().map(|timeline| timeline.id).collect();
        assert_eq!(ids, [1, 2, 4]);
        let pieces = |id| {
            timelines
                .get(id)
                .unwrap()
                .pieces
                .iter()
                .map(|piece| (piece.segment, piece.first_hour, piece.last_hour))
                .collect::<Vec<_>>()
        };
        assert_eq!(pieces(1), [(1, 0, 100)]);
        assert_eq!(pieces(2), [(1, 0, 50), (2, 51, 120), (3, 121, 200)]);
        assert_eq!(pieces(4), [(1, 0, 50), (2, 51, 80), (4, 81, 90)]);
        assert_eq!(timelines.parting(4), Some(Parting::Main));
        assert_eq!(timelines.parting(2), Some(Parting::LeftBehind(81)));
        assert_eq!(timelines.parting(1), Some(Parting::LeftBehind(51)));
        assert_eq!(timelines.segment_at(4, 60), Some(2));
        assert_eq!(timelines.segment_at(4, 95), None);
        assert_eq!(timelines.home(3), Some(2));
        let joins: Vec<_> = timelines.get(4).unwrap().joins.iter().map(|join| join.hour).collect();
        assert_eq!(joins, [51, 81]);
    }

    #[test]
    fn going_on_from_play_a_history_already_went_on_from_forks_it() {
        let extents = [
            extent(1, 1, BranchReason::SessionStart, (0, 100)),
            extent(2, 2, BranchReason::SessionStart, (101, 200)),
            extent(3, 3, BranchReason::SessionStart, (250, 260)),
        ];
        let links = vec![
            linked(1, None, Evidence::Unconfirmed),
            linked(2, Some(1), Evidence::Only),
            linked(3, Some(1), Evidence::Chosen),
        ];
        let timelines = join(&extents, links).unwrap();
        assert_eq!(timelines.main, 3);
        let segments = |id| {
            timelines
                .get(id)
                .unwrap()
                .pieces
                .iter()
                .map(|piece| (piece.segment, piece.first_hour, piece.last_hour))
                .collect::<Vec<_>>()
        };
        assert_eq!(segments(1), [(1, 0, 100), (2, 101, 200)]);
        assert_eq!(segments(3), [(1, 0, 100), (3, 250, 260)]);
        assert_eq!(timelines.parting(1), Some(Parting::LeftBehind(101)));
    }

    #[test]
    fn commands_before_a_first_frame_count_for_their_piece() {
        let piece = |segment, first_hour, last_hour, declared| Piece {
            segment,
            first_hour,
            last_hour,
            declared,
        };
        let pieces = [piece(1, 101, 150, 100), piece(2, 151, 200, 150)];
        assert_eq!(command_hours(&pieces), [(100, 149), (150, 200)]);
    }

    #[test]
    fn missed_play_extends_with_its_gap_and_a_separate_history_stands_alone() {
        let extents = [
            extent(1, 1, BranchReason::SessionStart, (0, 10)),
            extent(2, 2, BranchReason::SessionStart, (20, 30)),
            extent(3, 3, BranchReason::SessionStart, (5, 8)),
        ];
        let links = vec![
            linked(1, None, Evidence::Unconfirmed),
            linked(2, Some(1), Evidence::Unrecorded),
            linked(3, None, Evidence::Chosen),
        ];
        let timelines = join(&extents, links).unwrap();
        assert_eq!(timelines.main, 3);
        assert_eq!(timelines.get(1).unwrap().pieces.len(), 2);
        assert_eq!(timelines.parting(1), Some(Parting::Separate));
        assert_eq!(timelines.get(3).unwrap().first_hour(), 5);
    }
}
