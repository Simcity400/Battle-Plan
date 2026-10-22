#[cfg(test)]
use super::assignment::{historical_anchors, resolve_keys};
#[cfg(test)]
use super::scores::ScoreRead;
use super::scores::ScoreView;
use super::{
    assignment::{AssignedSide, SideAnchors, SideKey, assign_sides_with_anchors, native_side_keys},
    *,
};
use dashboard_base::products::country_groups::Scan;
#[cfg(test)]
use replay_frame::frame::TableUpdate;
use replay_frame::{frame::DecodedFrame, frame::ViewIdentity};
use std::sync::Arc;

#[derive(Clone, Default)]
pub(super) struct DayCache {
    /// Display hours of sampled daily cells; snapshots retain their supplying hour.
    pub sampled_days: BTreeMap<i32, i32>,
    pub segments: Vec<Arc<DaySegment>>,
    pub regions: BTreeSet<u32>,
    pub countries: BTreeSet<String>,
    pub theatres: BTreeMap<String, Vec<u8>>,
    pub sides: Arc<Vec<AirSide>>,
    #[cfg(test)]
    pub side_indices: Arc<HashMap<String, u32>>,
    pub anchors: Arc<SideAnchors>,
    pub warnings: Vec<String>,
    #[cfg(test)]
    pub viewpoint: Option<String>,
    #[cfg(test)]
    pub viewpoint_country: bool,
    pub available: bool,
    #[cfg(test)]
    pub partial: Option<(i32, Arc<Snapshot>)>,
    #[cfg(test)]
    pub theatre_read: Option<i32>,
}
#[derive(Clone, Default)]
pub(super) struct DaySegment {
    pub days: BTreeMap<i32, Arc<Snapshot>>,
    pub auxiliary: Vec<(i32, i32, Arc<Auxiliary>)>,
    #[cfg(test)]
    pub built_hours: Vec<i32>,
}
impl DayCache {
    #[cfg(test)]
    pub fn days(&self) -> BTreeMap<i32, &Arc<Snapshot>> {
        self.segments
            .iter()
            .flat_map(|segment| segment.days.iter().map(|(day, sample)| (*day, sample)))
            .collect()
    }
    #[cfg(test)]
    pub fn day(&self, day: i32) -> Option<&Arc<Snapshot>> {
        self.segments
            .iter()
            .rev()
            .find_map(|segment| segment.days.get(&day))
    }
    pub fn auxiliary(&self) -> impl Iterator<Item = &(i32, i32, Arc<Auxiliary>)> {
        self.segments.iter().flat_map(|segment| &segment.auxiliary)
    }
    #[cfg(test)]
    pub fn built_hours(&self) -> Vec<i32> {
        self.segments
            .iter()
            .flat_map(|segment| &segment.built_hours)
            .copied()
            .collect()
    }
}

#[derive(Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct Auxiliary {
    #[serde(skip)]
    pub wings: BTreeMap<u32, u32>,
    pub wings_known: bool,
    pub theatres: Arc<BTreeMap<String, BTreeSet<u32>>>,
    pub theatre_names: BTreeMap<String, Vec<u8>>,
    pub theatres_known: bool,
    pub theatre_read: Option<i32>,
}

#[derive(Clone)]
pub(super) struct Snapshot {
    pub hour: i32,
    pub(super) key_read: Option<Arc<KeyRead>>,
    pub cells: BTreeMap<u32, AirCell>,
    pub fallback: AirCell,
    pub wings: BTreeMap<u32, u32>,
    pub wings_known: bool,
    pub complete: bool,
    pub theatres: Arc<BTreeMap<String, BTreeSet<u32>>>,
    pub theatres_known: bool,
    pub available: bool,
    pub excluded: usize,
    pub active: BTreeSet<u32>,
    pub theatre_read: Option<i32>,
}

#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct KeyRead {
    pub(super) countries: BTreeSet<String>,
    pub(super) sides: Vec<SideKey>,
    pub(super) published: Vec<(usize, String)>,
}
#[cfg(test)]
impl KeyRead {
    fn changes(&self, anchors: &SideAnchors) -> bool {
        let keys = resolve_keys(&self.sides, &self.countries, anchors);
        self.published
            .iter()
            .any(|(index, key)| &keys[*index] != key)
    }
}

#[cfg(test)]
fn published_keys_change(cache: &DayCache, anchors: &SideAnchors) -> bool {
    let mut checked = HashSet::new();
    cache
        .segments
        .iter()
        .flat_map(|segment| segment.days.values())
        .chain(cache.partial.iter().map(|(_, sample)| sample))
        .filter_map(|sample| sample.key_read.as_ref())
        .any(|read| checked.insert(Arc::as_ptr(read)) && read.changes(anchors))
}

pub(super) fn span(spans: &mut Vec<AirSpan>, hour: i32) {
    match spans.last_mut() {
        Some(last) if last.last_hour.checked_add(1) == Some(hour) => last.last_hour = hour,
        _ => spans.push(AirSpan {
            first_hour: hour,
            last_hour: hour,
        }),
    }
}
pub(super) fn dictionary(
    sides: &mut Vec<AirSide>,
    indices: &mut HashMap<String, u32>,
    side: &AssignedSide,
    hour: i32,
) -> u32 {
    let tag = side
        .faction
        .is_none()
        .then(|| side.members.first())
        .flatten();
    if let Some(&index) = indices.get(&side.key) {
        let known = &mut sides[index as usize];
        let at = known.reads.partition_point(|read| read.0 <= hour);
        if at > 0 {
            let old = &known.reads[at - 1];
            if (&old.1, &old.2, old.3.as_ref(), old.4, &old.5, &old.6)
                == (
                    &side.name,
                    &side.leader,
                    tag,
                    side.co_belligerents,
                    &side.factions,
                    &side.colour_faction,
                )
            {
                return index;
            }
        }
        let read = (
            hour,
            side.name.clone(),
            side.leader.clone(),
            tag.cloned(),
            side.co_belligerents,
            side.factions.clone(),
            side.colour_faction.clone(),
        );
        if at > 0 && known.reads[at - 1].0 == hour {
            known.reads[at - 1] = read;
        } else {
            known.reads.insert(at, read);
        }
        return index;
    }
    let index = sides.len() as u32;
    let read = (
        hour,
        side.name.clone(),
        side.leader.clone(),
        tag.cloned(),
        side.co_belligerents,
        side.factions.clone(),
        side.colour_faction.clone(),
    );
    sides.push(AirSide {
        key: side.key.clone(),
        faction: side.faction.clone(),
        colour_faction: side.colour_faction.clone(),
        tag: tag.cloned(),
        name: side.name.clone(),
        leader: side.leader.clone(),
        co_belligerents: side.co_belligerents,
        factions: side.factions.clone(),
        reads: vec![read],
    });
    indices.insert(side.key.clone(), index);
    index
}

fn reason(reason: &str) -> AirCell {
    AirCell(Some(AirValue::Reason(reason.into())))
}

fn check_country_tags(countries: &[replay_data::rows::CountryRow]) -> io::Result<()> {
    let unsupported = countries
        .iter()
        .filter(|country| country.tag.contains(&0) || std::str::from_utf8(&country.tag).is_err())
        .count();
    if unsupported != 0 {
        return Err(invalid(format!(
            "Air comparison cannot represent {unsupported} recorded country tags: tags must be UTF-8 without NUL bytes"
        )));
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn snapshot(
    frame: &DecodedFrame,
    groups: &Scan,
    viewpoint: Option<&str>,
    sides: &mut Vec<AirSide>,
) -> io::Result<Snapshot> {
    let mut indices = sides
        .iter()
        .enumerate()
        .map(|(index, side)| (side.key.clone(), index as u32))
        .collect();
    snapshot_kind(
        frame,
        groups,
        viewpoint,
        false,
        (sides, &mut indices),
        &historical_anchors(groups),
        false,
    )
}

type SideCatalogue<'a> = (&'a mut Vec<AirSide>, &'a mut HashMap<String, u32>);

pub(super) fn snapshot_kind(
    frame: &DecodedFrame,
    groups: &Scan,
    viewpoint: Option<&str>,
    viewpoint_country: bool,
    catalogue: SideCatalogue<'_>,
    anchors: &SideAnchors,
    keep_keys: bool,
) -> io::Result<Snapshot> {
    let mut sample = snapshot_scores(
        frame,
        groups,
        viewpoint,
        viewpoint_country,
        catalogue,
        anchors,
        keep_keys,
    )?;
    apply_auxiliary(&mut sample, &auxiliary(frame));
    Ok(sample)
}

fn snapshot_scores(
    frame: &DecodedFrame,
    groups: &Scan,
    viewpoint: Option<&str>,
    viewpoint_country: bool,
    catalogue: SideCatalogue<'_>,
    anchors: &SideAnchors,
    keep_keys: bool,
) -> io::Result<Snapshot> {
    snapshot_score_read(
        frame,
        ScoreView {
            rows: dashboard_text::metadata::recorded_rows::<tables::RaidAirScores>(frame)?,
            complete: frame.complete::<tables::RaidAirScores>().is_ok(),
            owners: None,
            active: None,
        },
        SideRead {
            groups,
            anchors,
            keep_keys,
            cached: None,
        },
        (viewpoint, viewpoint_country),
        catalogue,
    )
}

struct SideRead<'a> {
    groups: &'a Scan,
    anchors: &'a SideAnchors,
    keep_keys: bool,
    cached: Option<&'a mut RosterRead>,
}

// A scan fixes its group history and anchors before it walks the frames.
// Only a membership/war/name boundary or changed country identity needs a join.
#[derive(Default)]
pub(super) struct RosterRead {
    boundaries: Option<BTreeSet<i32>>,
    present: BTreeSet<String>,
    first: i32,
    last: i32,
    sides: Option<Vec<AssignedSide>>,
}
impl RosterRead {
    fn read(
        &mut self,
        groups: &Scan,
        anchors: &SideAnchors,
        hour: i32,
        present: &BTreeSet<String>,
    ) -> Option<&[AssignedSide]> {
        let boundaries = self.boundaries.get_or_insert_with(|| {
            let mut boundaries = BTreeSet::new();
            let mut span = |first: i32, last: i32| {
                boundaries.insert(first);
                if let Some(after) = last.checked_add(1) {
                    boundaries.insert(after);
                }
            };
            for faction in groups.factions.values() {
                for run in faction.members.iter().chain(faction.leaders.iter()) {
                    span(run.first_hour, run.last_hour);
                }
                for name in faction.names.iter() {
                    span(name.first_hour, name.last_hour);
                }
                for run in faction.incomplete.iter() {
                    span(run.first_hour, run.last_hour);
                }
            }
            for run in groups.wars.iter() {
                span(run.first_hour, run.last_hour);
            }
            for runs in groups.identities.values() {
                for run in runs.iter() {
                    span(run.first_hour, run.last_hour);
                }
            }
            for run in groups
                .unobserved()
                .factions
                .iter()
                .chain(&groups.unobserved().wars)
            {
                span(run.first_hour, run.last_hour);
            }
            boundaries
        });
        if self.first > hour
            || hour > self.last
            || self.present != *present
            || self.present.is_empty()
        {
            self.sides = assign_sides_with_anchors(groups, hour, present, anchors);
            self.present.clone_from(present);
            self.first = hour;
            self.last = boundaries
                .range((std::ops::Bound::Excluded(hour), std::ops::Bound::Unbounded))
                .next()
                .map_or(i32::MAX, |next| next - 1);
        }
        self.sides.as_deref()
    }
}

#[derive(Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct ScoreOperands {
    pub identity_known: bool,
    pub present: Arc<BTreeSet<String>>,
    pub witnessed: BTreeSet<String>,
    pub invalid: BTreeSet<String>,
    pub complete: bool,
    pub unresolved: bool,
    pub available: bool,
}
#[derive(Default)]
pub(super) struct CountryScores {
    pub operands: ScoreOperands,
    pub pairs: BTreeMap<(String, u32), (i64, i64)>,
    pub active: BTreeSet<u32>,
}
pub(super) fn score_countries(
    frame: &DecodedFrame,
) -> io::Result<(ScoreOperands, HashMap<u32, String>)> {
    let mut operands = ScoreOperands::default();
    let (Ok(countries), Ok(links)) = (
        frame.complete::<tables::Countries>(),
        frame.complete::<tables::CountryLinks>(),
    ) else {
        return Ok((operands, HashMap::new()));
    };
    let mut slots = HashMap::new();
    let present = Arc::make_mut(&mut operands.present);
    check_country_tags(countries)?;
    let links = link_slots(links);
    for (tag, country) in tagged_countries_indexed(countries, &links) {
        let Some((_, slot)) = country.found() else {
            return Ok((operands, HashMap::new()));
        };
        let tag = std::str::from_utf8(tag).expect("tags checked above");
        if tag.is_empty() || slots.insert(slot, tag).is_some_and(|old| old != tag) {
            return Ok((operands, HashMap::new()));
        }
        present.insert(tag.to_owned());
    }
    operands.identity_known = true;
    let owners = links
        .into_iter()
        .filter_map(|(raw, slot)| slots.get(&slot).map(|tag| (raw, (*tag).to_owned())))
        .collect();
    Ok((operands, owners))
}

pub(super) fn country_scores(
    frame: &DecodedFrame,
    scores: ScoreView<'_>,
) -> io::Result<CountryScores> {
    let (operands, owners) = score_countries(frame)?;
    Ok(score_values(scores, operands, &owners))
}

pub(super) fn score_values(
    scores: ScoreView<'_>,
    operands: ScoreOperands,
    owners: &HashMap<u32, String>,
) -> CountryScores {
    let mut projection = CountryScores {
        operands,
        ..Default::default()
    };
    projection.operands.complete = scores.complete;
    projection.operands.available = scores.complete || !scores.rows.is_empty();
    if let Some(active) = scores.active {
        projection.active.extend(active.keys());
    } else {
        projection.active.extend(
            scores
                .rows
                .iter()
                .filter(|row| row.friendly > 0 || row.enemy > 0)
                .map(|row| row.region),
        );
    }
    if !projection.operands.identity_known {
        return projection;
    }
    let mut supplied = BTreeMap::new();
    let mut visit = |row: &replay_data::rows::RaidAirScoreRow| {
        let Some(tag) = owners.get(&row.country).map(String::as_str) else {
            projection.operands.unresolved = true;
            return;
        };
        projection.operands.witnessed.insert(tag.to_owned());
        if row.friendly < 0 || row.enemy < 0 {
            projection.operands.invalid.insert(tag.to_owned());
            return;
        }
        let value = (row.country, (row.friendly, row.enemy));
        supplied
            .entry((tag.to_owned(), row.region))
            .and_modify(|old: &mut (u32, (i64, i64))| {
                if row.country > old.0 {
                    *old = value;
                }
            })
            .or_insert(value);
    };
    if let Some(owners) = scores.owners {
        for index in owners.values().flat_map(|regions| regions.values()) {
            visit(&scores.rows[*index]);
        }
    } else {
        for row in scores.rows {
            visit(row);
        }
    }
    projection.pairs = supplied
        .into_iter()
        .map(|(key, (_, value))| (key, value))
        .collect();
    projection
}
fn snapshot_score_read(
    frame: &DecodedFrame,
    scores: ScoreView<'_>,
    side_read: SideRead<'_>,
    view: (Option<&str>, bool),
    catalogue: SideCatalogue<'_>,
) -> io::Result<Snapshot> {
    let projection = country_scores(frame, scores)?;
    snapshot_country_scores(frame.hour, &projection, side_read, view, catalogue)
}
pub(super) fn prepared_snapshot(
    hour: i32,
    projection: &CountryScores,
    groups: &Scan,
    anchors: &SideAnchors,
    view: (Option<&str>, bool),
    catalogue: SideCatalogue<'_>,
    rosters: &mut RosterRead,
) -> io::Result<Snapshot> {
    snapshot_country_scores(
        hour,
        projection,
        SideRead {
            groups,
            anchors,
            keep_keys: false,
            cached: Some(rosters),
        },
        view,
        catalogue,
    )
}
pub(super) fn catalogue_snapshot(
    hour: i32,
    projection: &CountryScores,
    groups: &Scan,
    sides: &mut Vec<AirSide>,
) -> io::Result<Snapshot> {
    // Capture native operands; final historical keys are resolved by the prepared query.
    let anchors = SideAnchors::default();
    snapshot_country_scores(
        hour,
        projection,
        SideRead {
            groups,
            anchors: &anchors,
            keep_keys: true,
            cached: None,
        },
        (None, false),
        (sides, &mut HashMap::new()),
    )
}
fn snapshot_country_scores(
    hour: i32,
    projection: &CountryScores,
    side_read: SideRead<'_>,
    view: (Option<&str>, bool),
    catalogue: SideCatalogue<'_>,
) -> io::Result<Snapshot> {
    let SideRead {
        groups,
        anchors,
        keep_keys,
        cached,
    } = side_read;
    let (viewpoint, viewpoint_country) = view;
    let (sides, indices) = catalogue;
    let mut result = Snapshot {
        hour,
        key_read: None,
        cells: BTreeMap::new(),
        fallback: reason("identity-not-recorded"),
        wings: BTreeMap::new(),
        wings_known: false,
        complete: false,
        theatres: Arc::default(),
        theatres_known: false,
        available: false,
        excluded: 0,
        active: BTreeSet::new(),
        theatre_read: None,
    };
    result.complete = projection.operands.complete;
    result.available = projection.operands.available;
    if !projection.operands.identity_known {
        return Ok(result);
    }
    let present = &projection.operands.present;
    let country_view = viewpoint.is_some() && viewpoint_country;
    let owned_rosters = if cached.is_none() && !country_view {
        assign_sides_with_anchors(groups, hour, present, anchors)
    } else {
        None
    };
    let rosters = if country_view {
        &[][..]
    } else {
        let rosters = if let Some(cached) = cached {
            cached.read(groups, anchors, hour, present)
        } else {
            owned_rosters.as_deref()
        };
        let Some(rosters) = rosters else {
            result.fallback = reason("sides-not-recorded");
            return Ok(result);
        };
        rosters
    };
    let view_roster = viewpoint
        .filter(|_| !viewpoint_country)
        .and_then(|view| rosters.iter().find(|side| side.key == view));
    if viewpoint.is_some() && !viewpoint_country && view_roster.is_none() {
        result.fallback = reason("side-not-present");
        result.active.extend(&projection.active);
        return Ok(result);
    }
    let view_tag = viewpoint
        .filter(|_| viewpoint_country)
        .or_else(|| view_roster.and_then(|side| side.leader.as_deref()));
    let wanted: HashSet<&str> = match (viewpoint, view_roster, view_tag) {
        (Some(_), Some(side), None) => side.members.iter().map(String::as_str).collect(),
        (_, _, Some(tag)) => [tag].into(),
        _ => rosters
            .iter()
            .flat_map(|side| {
                side.leader
                    .as_deref()
                    .map(|tag| vec![tag])
                    .unwrap_or_else(|| side.members.iter().map(String::as_str).collect())
            })
            .collect(),
    };
    result.complete &= !projection.operands.unresolved
        && !wanted
            .iter()
            .any(|tag| projection.operands.invalid.contains(*tag));
    let regions = &projection.active;
    let pairs = &projection.pairs;
    let witnessed = &projection.operands.witnessed;
    let pair = |tag: &str, region: Option<u32>| -> Result<Option<(i64, i64)>, ()> {
        if !present.contains(tag) {
            return Err(());
        }
        if let Some(value) = region.and_then(|region| pairs.get(&(tag.to_owned(), region))) {
            return Ok(Some(*value));
        }
        if !result.complete {
            return Err(());
        }
        Ok(witnessed.contains(tag).then_some((0, 0)))
    };
    let air_rosters: Vec<_> = rosters
        .iter()
        .filter(|side| {
            viewpoint.is_some()
                || side.faction.is_some()
                || !matches!(side_pair(side, None, &pair).1, Ok(None))
        })
        .collect();
    let excluded = rosters.len() - air_rosters.len();
    let mut used_keys = BTreeSet::new();
    let mut side_ids = vec![None; air_rosters.len()];
    let mut viewpoint_side = None;
    let mut build = |region: Option<u32>, sides: &mut Vec<AirSide>| -> AirCell {
        if viewpoint.is_some()
            && let Some(side) = view_roster
        {
            let (leader, value) = side_pair(side, region, &pair);
            if keep_keys {
                used_keys.insert(side.key.as_str());
            }
            let index =
                *viewpoint_side.get_or_insert_with(|| dictionary(sides, indices, side, hour));
            return match value {
                Ok(Some((friendly, enemy))) => AirCell(Some(AirValue::SideViewpoint {
                    view: (friendly.to_string(), enemy.to_string()),
                    side: index,
                    leader: leader.map(str::to_owned),
                })),
                Ok(None) => AirCell(Some(AirValue::Sides(AirSideCell {
                    scores: vec![],
                    viewpoints: vec![],
                    unknown: vec![],
                    unknown_set: None,
                    no_air_object: vec![index],
                    filtered: false,
                    excluded: 0,
                }))),
                Err(()) => AirCell(Some(AirValue::Sides(AirSideCell {
                    scores: vec![],
                    viewpoints: vec![],
                    unknown: vec![index],
                    unknown_set: None,
                    no_air_object: vec![],
                    filtered: false,
                    excluded: 0,
                }))),
            };
        }
        if let Some(tag) = view_tag {
            return match pair(tag, region) {
                Ok(Some((friendly, enemy))) => AirCell(Some(AirValue::Viewpoint((
                    friendly.to_string(),
                    enemy.to_string(),
                )))),
                Ok(None) => reason("no-air-object"),
                Err(()) => reason("score-not-recorded"),
            };
        }
        let mut cell = AirSideCell {
            scores: vec![],
            viewpoints: vec![],
            unknown: vec![],
            unknown_set: None,
            no_air_object: vec![],
            filtered: false,
            excluded: 0,
        };
        for (at, side) in air_rosters.iter().enumerate() {
            let (leader, value) = side_pair(side, region, &pair);
            // Owners without a strategic-air object do not participate. A known
            // zero contributes no part; never ship the neutral country roster.
            if matches!(value, Ok(None)) && side.faction.is_none() {
                continue;
            }
            if matches!(value, Ok(Some((0, _)))) {
                continue;
            }
            if keep_keys {
                used_keys.insert(side.key.as_str());
            }
            let index = *side_ids[at].get_or_insert_with(|| dictionary(sides, indices, side, hour));
            if side.leader.is_none()
                && let Some(leader) = leader
            {
                cell.viewpoints.push((index, leader.to_owned()));
            }
            match value {
                Ok(Some((friendly, _))) if friendly > 0 => {
                    cell.scores.push((index, friendly.to_string()))
                }
                Ok(None) => cell.no_air_object.push(index),
                Err(()) => cell.unknown.push(index),
                _ => {}
            }
        }
        if cell.unknown.is_empty() && cell.no_air_object.is_empty() && cell.viewpoints.is_empty() {
            AirCell(Some(AirValue::Scores(cell.scores)))
        } else {
            AirCell(Some(AirValue::Sides(cell)))
        }
    };
    let fallback = build(None, sides);
    result.active.clone_from(regions);
    for &region in regions {
        result.cells.insert(region, build(Some(region), sides));
    }
    result.fallback = fallback;
    if !used_keys.is_empty() {
        let (countries, sides) =
            native_side_keys(groups, hour, rosters).expect("assigned native side identities");
        result.key_read = Some(Arc::new(KeyRead {
            countries,
            sides,
            published: rosters
                .iter()
                .enumerate()
                .filter(|(_, side)| used_keys.contains(side.key.as_str()))
                .map(|(index, side)| (index, side.key.clone()))
                .collect(),
        }));
    }
    result.excluded = excluded;
    Ok(result)
}

type ScorePair = Result<Option<(i64, i64)>, ()>;
type SidePair<'a> = (Option<&'a str>, ScorePair);

fn side_pair<'a>(
    side: &'a AssignedSide,
    region: Option<u32>,
    pair: &impl Fn(&str, Option<u32>) -> ScorePair,
) -> SidePair<'a> {
    if let Some(tag) = &side.leader {
        return (Some(tag.as_str()), pair(tag, region));
    }
    if side.faction.is_some() {
        return (None, Err(()));
    }
    let mut best: Option<(&String, (i64, i64))> = None;
    let mut unknown = false;
    for tag in &side.members {
        match pair(tag, region) {
            Ok(Some(value))
                if best.is_none_or(|(old, score)| {
                    value.0 > score.0 || value.0 == score.0 && tag < old
                }) =>
            {
                best = Some((tag, value))
            }
            Err(()) => unknown = true,
            _ => {}
        }
    }
    (
        best.map(|(tag, _)| tag.as_str()),
        if unknown {
            Err(())
        } else {
            Ok(best.map(|(_, value)| value))
        },
    )
}

#[cfg(test)]
pub(super) const DAY_TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::RaidAirScores,
];
#[cfg(test)]
pub(super) const MOMENT_TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::RaidAirScores,
    TableId::AirWings,
    TableId::AirTheatres,
    TableId::AirTheatreGroups,
    TableId::AirTheatreMembers,
];

#[cfg(test)]
pub(super) fn day_decoder(tables: &[TableId]) -> FrameDecoder {
    let tables: Vec<_> = tables
        .iter()
        .copied()
        .filter(|table| *table != TableId::RaidAirScores)
        .collect();
    FrameDecoder::for_tables(&tables)
}

pub(super) fn auxiliary(frame: &DecodedFrame) -> Auxiliary {
    let mut result = Auxiliary::default();
    let Ok(wings) = frame.complete::<tables::AirWings>() else {
        return result;
    };
    result.wings_known = true;
    for wing in wings {
        if let Some(region) = wing.mission_region {
            *result.wings.entry(region).or_default() += 1;
        }
    }
    if let (Ok(theatres), Ok(groups), Ok(members)) = (
        frame.complete::<tables::AirTheatres>(),
        frame.complete::<tables::AirTheatreGroups>(),
        frame.complete::<tables::AirTheatreMembers>(),
    ) {
        let wing_regions: HashMap<_, _> = wings
            .iter()
            .filter_map(|wing| Some((wing.id, wing.mission_region?)))
            .collect();
        let group_theatres: HashMap<_, _> = groups
            .iter()
            .map(|group| (group.id, group.theatre))
            .collect();
        for theatre in theatres {
            let id = theatre.id.to_string();
            Arc::make_mut(&mut result.theatres)
                .entry(id.clone())
                .or_default();
            result.theatre_names.insert(id, theatre.name.clone());
        }
        result.theatres_known = true;
        for member in members {
            if let Some(region) = wing_regions.get(&member.wing) {
                if let Some(theatre) = group_theatres
                    .get(&member.group)
                    .and_then(|id| Arc::make_mut(&mut result.theatres).get_mut(&id.to_string()))
                {
                    theatre.insert(*region);
                } else {
                    result.theatres_known = false;
                }
            }
        }
    }
    result
}

pub(super) fn apply_auxiliary(sample: &mut Snapshot, aux: &Auxiliary) {
    sample.wings.clone_from(&aux.wings);
    sample.wings_known = aux.wings_known;
    sample.theatres.clone_from(&aux.theatres);
    sample.theatres_known = aux.theatres_known;
    sample.theatre_read = aux.theatre_read;
    if !matches!(&sample.fallback.0, Some(AirValue::Reason(reason)) if matches!(reason.as_str(), "identity-not-recorded" | "sides-not-recorded" | "side-not-present"))
    {
        sample.active.extend(aux.wings.keys());
    }
}

fn date_theatres(read: bool, hour: i32, known: bool, last_read: &mut Option<i32>) {
    if !known {
        *last_read = None;
    } else if read {
        *last_read = Some(hour);
    }
}

#[cfg(test)]
fn dated_auxiliary(frame: &DecodedFrame, last_read: &mut Option<i32>) -> Auxiliary {
    let mut result = auxiliary(frame);
    date_theatres(
        dashboard_text::metadata::slot_read(
            replay_data::format::FormatVersion::CURRENT,
            &frame.metrics,
            TableId::AirTheatres,
            frame.hour,
        ),
        frame.hour,
        result.theatres_known,
        last_read,
    );
    result.theatre_read = *last_read;
    result
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
pub(super) struct AuxiliaryRead {
    #[serde(skip)]
    views: Vec<ViewIdentity>,
    value: Arc<Auxiliary>,
}
impl AuxiliaryRead {
    pub(super) fn read(
        &mut self,
        read: bool,
        frame: &DecodedFrame,
        last_read: &mut Option<i32>,
    ) -> Arc<Auxiliary> {
        let tables = [
            TableId::AirWings,
            TableId::AirTheatres,
            TableId::AirTheatreGroups,
            TableId::AirTheatreMembers,
        ];
        if self.views.len() != tables.len()
            || self
                .views
                .iter()
                .zip(tables)
                .any(|(old, table)| !old.is(&frame.tables[table.index()]))
        {
            let mut value = auxiliary(frame);
            value.theatre_read = self.value.theatre_read;
            if value.theatres == self.value.theatres {
                value.theatres = Arc::clone(&self.value.theatres);
            }
            if *self.value != value {
                self.value = Arc::new(value);
            }
            // Weak identities do not keep decoder baselines alive or force COW.
            self.views = tables
                .iter()
                .map(|table| frame.tables[table.index()].identity())
                .collect();
        }
        self.date(read, frame.hour, last_read);
        Arc::clone(&self.value)
    }

    /// Date the held theatres at `hour`, read there or not (`read`), as
    /// [`Self::read`] dates them after taking the hour's tables: at an hour
    /// whose auxiliary rows are the hour before's, this is all it does.
    pub(super) fn date(&mut self, read: bool, hour: i32, last_read: &mut Option<i32>) {
        date_theatres(read, hour, self.value.theatres_known, last_read);
        if self.value.theatre_read != *last_read {
            Arc::make_mut(&mut self.value).theatre_read = *last_read;
        }
    }
}

#[cfg(test)]
#[derive(Default)]
pub(super) struct DayReads {
    /// The recording's format, which dates its daily tables
    /// ([`dashboard_text::metadata::slot_read`]).
    format: replay_data::format::FormatVersion,
    scores: ScoreRead,
    regions: HashSet<u32>,
    countries: BTreeSet<String>,
    theatres: BTreeMap<String, Vec<u8>>,
    available: bool,
    theatre_read: Option<i32>,
    previous_hour: Option<i32>,
    auxiliary: AuxiliaryRead,
    country_view: Option<ViewIdentity>,
    country_tags: Vec<Vec<u8>>,
    rosters: RosterRead,
}
#[cfg(test)]
impl DayReads {
    pub(super) fn snapshot(
        &mut self,
        frame: &DecodedFrame,
        groups: &Scan,
        view: (Option<&str>, bool),
        catalogue: SideCatalogue<'_>,
        anchors: &SideAnchors,
        base: Option<&DayCache>,
    ) -> io::Result<Snapshot> {
        let mut sample = snapshot_score_read(
            frame,
            self.scores.view(),
            SideRead {
                groups,
                anchors,
                keep_keys: true,
                cached: Some(&mut self.rosters),
            },
            view,
            catalogue,
        )?;
        if let Some(base) = base {
            attach_auxiliary(base, &mut sample);
        } else {
            apply_auxiliary(&mut sample, &self.auxiliary.value);
        }
        Ok(sample)
    }

    pub(super) fn observe(
        &mut self,
        frame: &(impl replay_frame::frame::FrameSource + ?Sized),
        decoded: &DecodedFrame,
        include_auxiliary: bool,
    ) -> io::Result<Option<Arc<Auxiliary>>> {
        let aux = self.observe_tables(frame, decoded, include_auxiliary);
        let view = &decoded.tables[TableId::Countries.index()];
        if !self.country_view.as_ref().is_some_and(|old| old.is(view)) {
            let known = decoded.complete::<tables::Countries>().unwrap_or(&[]);
            if known.len() != self.country_tags.len()
                || known
                    .iter()
                    .zip(&self.country_tags)
                    .any(|(country, tag)| &country.tag != tag)
            {
                check_country_tags(known)?;
                self.country_tags = known.iter().map(|country| country.tag.clone()).collect();
                self.countries.extend(known.iter().map(|country| {
                    std::str::from_utf8(&country.tag)
                        .expect("checked tags")
                        .to_owned()
                }));
            }
            self.country_view = Some(view.identity());
        }
        Ok(aux)
    }

    // Window baseline hours need score/theatre state, but their country labels
    // are not part of the requested view. Validate labels only when sampled.
    fn observe_tables(
        &mut self,
        frame: &(impl replay_frame::frame::FrameSource + ?Sized),
        decoded: &DecodedFrame,
        include_auxiliary: bool,
    ) -> Option<Arc<Auxiliary>> {
        self.scores.observe(frame, &mut self.regions);
        let scores = self.scores.view();
        self.available |= scores.complete || !scores.rows.is_empty();
        if self
            .previous_hour
            .is_some_and(|hour| hour.checked_add(1) != Some(frame.hour()))
        {
            self.theatre_read = None;
        }
        self.previous_hour = Some(frame.hour());
        if !include_auxiliary {
            return None;
        }
        let previous = Arc::clone(&self.auxiliary.value);
        let aux = self.auxiliary.read(
            dashboard_text::metadata::slot_read(
                self.format,
                &decoded.metrics,
                TableId::AirTheatres,
                decoded.hour,
            ),
            decoded,
            &mut self.theatre_read,
        );
        if !Arc::ptr_eq(&previous, &aux) {
            self.regions.extend(aux.wings.keys());
            self.theatres.extend(aux.theatre_names.clone());
        }
        Some(aux)
    }
}

#[cfg(test)]
pub(super) fn attach_auxiliary(cache: &DayCache, sample: &mut Snapshot) {
    let segment = cache.segments.iter().rev().find(|segment| {
        segment
            .auxiliary
            .first()
            .is_some_and(|(first, _, _)| *first <= sample.hour)
    });
    let aux = segment.and_then(|segment| {
        let index = segment
            .auxiliary
            .partition_point(|(first, _, _)| *first <= sample.hour);
        index
            .checked_sub(1)
            .and_then(|index| segment.auxiliary.get(index))
            .filter(|(_, last, _)| sample.hour <= *last)
    });
    if let Some((_, _, aux)) = aux {
        apply_auxiliary(sample, aux);
    }
}

/// A seek within the last column reads at most one day's intersecting blocks.
/// It never borrows the cached end-of-day value from after the requested hour.
#[cfg(test)]
pub(super) fn partial_day(
    replay: &Replay,
    branch: u64,
    groups: &Scan,
    cache: &mut DayCache,
    last: i32,
    cancelled: &AtomicBool,
) -> io::Result<Option<Snapshot>> {
    let first = last
        .div_euclid(DAY_HOURS)
        .checked_mul(DAY_HOURS)
        .ok_or_else(|| invalid("Air timeline start is outside the calendar range"))?;
    let mut sample = None;
    // Index bounds select the final supplying block. Apply preceding deltas,
    // then materialise only the final supplying sample from borrowed rows.
    if let Some(block) = replay.index.iter().rfind(|block| {
        block.branch == branch && block.first_hour <= last && block.last_hour >= first
    }) {
        let frames = replay.read_block_selected(block, DAY_TABLES)?;
        let mut decoder = day_decoder(DAY_TABLES);
        let mut reads = DayReads {
            format: replay.format,
            ..DayReads::default()
        };
        let final_hour = frames
            .iter()
            .rfind(|frame| frame.hour <= last && frame.hour >= first)
            .map(|frame| frame.hour);
        for frame in frames.into_iter().take_while(|frame| frame.hour <= last) {
            check_cancel(cancelled)?;
            let decoded = decoder.advance(&frame)?;
            validate_campaign(decoded, replay)?;
            reads.observe_tables(&frame, decoded, false);
            if Some(frame.hour) == final_hour {
                sample = Some(snapshot_score_read(
                    decoded,
                    reads.scores.view(),
                    SideRead {
                        groups,
                        anchors: &cache.anchors,
                        keep_keys: true,
                        cached: Some(&mut reads.rosters),
                    },
                    (cache.viewpoint.as_deref(), cache.viewpoint_country),
                    (
                        Arc::make_mut(&mut cache.sides),
                        Arc::make_mut(&mut cache.side_indices),
                    ),
                )?);
            }
        }
    }
    if let Some(sample) = &mut sample {
        attach_auxiliary(cache, sample);
    }
    Ok(sample)
}

#[cfg(test)]
pub(super) fn scan_days(
    replay: &Replay,
    branch: u64,
    groups: &Scan,
    viewpoint: Option<&str>,
    cancelled: &AtomicBool,
) -> io::Result<DayCache> {
    scan_days_kind(replay, branch, groups, viewpoint, false, cancelled)
}

#[cfg(test)]
pub(super) fn scan_days_kind(
    replay: &Replay,
    branch: u64,
    groups: &Scan,
    viewpoint: Option<&str>,
    viewpoint_country: bool,
    cancelled: &AtomicBool,
) -> io::Result<DayCache> {
    let mut cache = DayCache {
        viewpoint: viewpoint.map(str::to_owned),
        viewpoint_country,
        ..Default::default()
    };
    extend_days(&mut cache, replay, branch, groups, cancelled)?;
    Ok(cache)
}
#[cfg(test)]
pub(super) fn extend_days(
    cache: &mut DayCache,
    replay: &Replay,
    branch: u64,
    groups: &Scan,
    cancelled: &AtomicBool,
) -> io::Result<()> {
    extend_days_shared(cache, replay, branch, groups, None, cancelled)
}

#[cfg(test)]
fn merge_sides(sides: &mut Vec<AirSide>, indices: &mut HashMap<String, u32>, base: &[AirSide]) {
    for side in base {
        if let Some(&index) = indices.get(&side.key) {
            let known = &mut sides[index as usize];
            for read in &side.reads {
                match known.reads.binary_search_by_key(&read.0, |read| read.0) {
                    Ok(at) => known.reads[at] = read.clone(),
                    Err(at) => known.reads.insert(at, read.clone()),
                }
            }
        } else {
            indices.insert(side.key.clone(), sides.len() as u32);
            sides.push(side.clone());
        }
    }
}

#[cfg(test)]
pub(super) fn extend_days_shared(
    cache: &mut DayCache,
    replay: &Replay,
    branch: u64,
    groups: &Scan,
    base: Option<&DayCache>,
    cancelled: &AtomicBool,
) -> io::Result<()> {
    let anchors = base.map_or_else(
        || Arc::new(historical_anchors(groups)),
        |base| Arc::clone(&base.anchors),
    );
    extend_days_anchored(cache, replay, branch, groups, base, anchors, cancelled)
}

#[cfg(test)]
pub(super) fn extend_days_anchored(
    cache: &mut DayCache,
    replay: &Replay,
    branch: u64,
    groups: &Scan,
    base: Option<&DayCache>,
    anchors: Arc<SideAnchors>,
    cancelled: &AtomicBool,
) -> io::Result<()> {
    let blocks: Vec<_> = replay
        .index
        .iter()
        .filter(|block| block.branch == branch)
        .collect();
    if cache.segments.len() == blocks.len() && cache.anchors == anchors {
        return Ok(());
    }
    if !cache.segments.is_empty()
        && cache.anchors != anchors
        && published_keys_change(cache, &anchors)
    {
        // New historical alliance evidence can relabel earlier components. Build
        // the whole branch only if a published key changes, using cold/window anchors,
        // publishing only after success; never leave old and new keys mixed.
        let mut rebuilt = DayCache {
            viewpoint: cache.viewpoint.clone(),
            viewpoint_country: cache.viewpoint_country,
            ..Default::default()
        };
        extend_days_anchored(
            &mut rebuilt,
            replay,
            branch,
            groups,
            base,
            Arc::clone(&anchors),
            cancelled,
        )?;
        *cache = rebuilt;
        return Ok(());
    }
    let mut side_values = Arc::clone(&cache.sides);
    let mut side_indices = Arc::clone(&cache.side_indices);
    let sides = Arc::make_mut(&mut side_values);
    let indices = Arc::make_mut(&mut side_indices);
    if let Some(base) = base {
        merge_sides(sides, indices, &base.sides);
    }
    let mut segments = Vec::new();
    let mut reads = DayReads {
        format: replay.format,
        theatre_read: cache.theatre_read,
        previous_hour: cache
            .segments
            .iter()
            .rev()
            .flat_map(|segment| segment.days.values().rev())
            .next()
            .map(|sample| sample.hour),
        ..Default::default()
    };
    let mut last_key_read = cache
        .segments
        .iter()
        .rev()
        .flat_map(|segment| segment.days.values().rev())
        .find_map(|sample| sample.key_read.clone());
    let tables = if base.is_some() {
        DAY_TABLES
    } else {
        MOMENT_TABLES
    };
    let first = cache.segments.len();
    dashboard_core::blocks::scan_blocks(
        replay,
        blocks[first..].iter().copied(),
        tables,
        cancelled,
        |pending| {
            for (index, (_, frames)) in (first..).zip(pending) {
                check_cancel(cancelled)?;
                let mut segment = DaySegment::default();
                let mut frames = frames?.into_iter().peekable();
                let mut decoder = day_decoder(tables);
                // Consume each payload after its update; large blocks do not retain all
                // delta allocations while indexing later hours.
                while let Some(frame) = frames.next() {
                    check_cancel(cancelled)?;
                    let decoded = decoder.advance(&frame)?;
                    validate_campaign(decoded, replay)?;
                    if let Some(aux) = reads.observe(&frame, decoded, base.is_none())? {
                        match segment.auxiliary.last_mut() {
                            Some((_, end, old))
                                if end.checked_add(1) == Some(frame.hour)
                                    && Arc::ptr_eq(old, &aux) =>
                            {
                                *end = frame.hour
                            }
                            _ => segment.auxiliary.push((frame.hour, frame.hour, aux)),
                        }
                    }
                    let next = frames
                        .peek()
                        .map(|next| next.hour)
                        .or_else(|| blocks.get(index + 1).map(|next| next.first_hour));
                    if next.is_none_or(|next| {
                        next.div_euclid(DAY_HOURS) != frame.hour.div_euclid(DAY_HOURS)
                    }) {
                        let mut sample = reads.snapshot(
                            decoded,
                            groups,
                            (cache.viewpoint.as_deref(), cache.viewpoint_country),
                            (&mut *sides, &mut *indices),
                            &anchors,
                            base,
                        )?;
                        if let Some(read) = sample.key_read.as_mut() {
                            if let Some(previous) = last_key_read
                                .as_ref()
                                .filter(|previous| previous.as_ref() == read.as_ref())
                            {
                                *read = Arc::clone(previous);
                            }
                            last_key_read = Some(Arc::clone(read));
                        }
                        segment
                            .days
                            .insert(frame.hour.div_euclid(DAY_HOURS), Arc::new(sample));
                    }
                    #[cfg(test)]
                    segment.built_hours.push(frame.hour);
                    drop(frame);
                }
                if let Some(base) = base {
                    segment.auxiliary = base.segments[index].auxiliary.clone();
                    reads.regions.extend(&base.regions);
                    reads.theatres.clone_from(&base.theatres);
                }
                segments.push(Arc::new(segment));
            }
            Ok(())
        },
    )?;
    // All fallible work is local. A failed extension drops its side/read changes
    // and segments without changing published indices or truncating old reads.
    for segment in segments {
        let day = blocks[cache.segments.len()]
            .first_hour
            .div_euclid(DAY_HOURS);
        if let Some(previous) = cache
            .segments
            .last_mut()
            .filter(|previous| previous.days.contains_key(&day))
        {
            Arc::make_mut(previous).days.remove(&day);
        }
        cache.segments.push(segment);
    }
    cache.anchors = anchors;
    if let Some(base) = base.filter(|base| base.sides.as_ref() == side_values.as_ref()) {
        cache.sides = Arc::clone(&base.sides);
        cache.side_indices = Arc::clone(&base.side_indices);
    } else {
        cache.sides = side_values;
        cache.side_indices = side_indices;
    }
    cache.regions.extend(reads.regions);
    cache.countries.extend(reads.countries);
    cache.theatres.extend(reads.theatres);
    cache.available |= reads.available;
    cache.theatre_read = reads.theatre_read;
    cache.partial = None;
    cache.warnings.clone_from(&groups.warnings);
    Ok(())
}

#[cfg(test)]
pub(super) fn scan_window(
    replay: &Replay,
    branch: u64,
    groups: &Scan,
    params: &AirParams,
    anchors: &SideAnchors,
    cancelled: &AtomicBool,
) -> io::Result<(Vec<Snapshot>, Vec<AirSide>)> {
    let (first, last) = hours(params)?;
    let mut samples = Vec::new();
    let mut sides = Vec::new();
    let mut indices = HashMap::new();
    let mut reads = DayReads {
        format: replay.format,
        ..DayReads::default()
    };
    // Include the preceding daily slot to date carried theatre membership.
    let read_first = first.saturating_sub(DAY_HOURS);
    dashboard_core::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|block| {
            block.branch == branch && block.first_hour <= last && block.last_hour >= read_first
        }),
        MOMENT_TABLES,
        cancelled,
        |blocks| {
            for (_, frames) in blocks {
                let frames = frames?;
                let mut decoder = day_decoder(MOMENT_TABLES);
                for frame in frames {
                    check_cancel(cancelled)?;
                    if frame.hour > last {
                        break;
                    }
                    let decoded = decoder.advance(&frame)?;
                    validate_campaign(decoded, replay)?;
                    reads.observe_tables(&frame, decoded, true);
                    if frame.hour < first {
                        continue;
                    }
                    let mut sample = snapshot_score_read(
                        decoded,
                        reads.scores.view(),
                        SideRead {
                            groups,
                            anchors,
                            keep_keys: false,
                            cached: Some(&mut reads.rosters),
                        },
                        (
                            (params.mode == Some(AirMode::Viewpoint))
                                .then_some(params.viewpoint.as_deref())
                                .flatten(),
                            params.viewpoint_country == Some(true),
                        ),
                        (&mut sides, &mut indices),
                    )?;
                    apply_auxiliary(&mut sample, &reads.auxiliary.value);
                    samples.push(sample);
                }
            }
            Ok(())
        },
    )?;
    Ok((samples, sides))
}

#[cfg(test)]
mod index_tests {
    use super::*;
    use replay_data::{rows::RaidAirScoreRow, rows::TableRows};
    use replay_frame::{frame::FrameEncoder, frame::TableView};

    #[test]
    fn air_superiority_side_join_reuses_rosters_until_a_recorded_boundary() {
        let frames = (0..4)
            .map(|hour| {
                let mut frame = dashboard_core::test_fixtures::ownership_frame(1, hour, 44);
                *frame.tables[TableId::Factions.index()] =
                    TableView::Complete(TableRows::Factions(vec![replay_data::rows::FactionRow {
                        id: 7,
                        name: if hour < 2 {
                            b"First".to_vec()
                        } else {
                            b"Renamed".to_vec()
                        },
                        ..Default::default()
                    }]));
                *frame.tables[TableId::FactionMembers.index()] =
                    TableView::Complete(TableRows::FactionMembers(vec![
                        replay_data::rows::FactionMemberRow {
                            country: 9001,
                            faction: 7,
                            ordinal: 0,
                        },
                    ]));
                *frame.tables[TableId::WarRelations.index()] =
                    TableView::Complete(TableRows::WarRelations(vec![]));
                frame
            })
            .collect();
        let replay = Replay::from_records(
            dashboard_core::test_fixtures::CAMPAIGN.into(),
            &[dashboard_core::test_fixtures::records(1, frames)],
        )
        .unwrap();
        let groups = dashboard_base::products::country_groups::scan_branch(
            &replay,
            1,
            &AtomicBool::new(false),
        )
        .unwrap();
        let anchors = historical_anchors(&groups);
        let mut cached = RosterRead::default();
        let present: BTreeSet<_> = ["GER".to_owned(), "FRA".to_owned()].into();
        assert_eq!(
            cached.read(&groups, &anchors, 0, &present).unwrap()[0].name,
            "First"
        );
        assert_eq!(
            cached.read(&groups, &anchors, 1, &present).unwrap()[0].name,
            "First"
        );
        assert_eq!(
            (cached.first, cached.last),
            (0, 1),
            "unchanged hour did not rejoin the group history"
        );
        assert_eq!(
            cached.read(&groups, &anchors, 2, &present).unwrap()[0].name,
            "Renamed"
        );
        assert_eq!(cached.first, 2);
        let changed: BTreeSet<_> = ["FRA".to_owned()].into();
        assert!(
            cached
                .read(&groups, &anchors, 3, &changed)
                .unwrap()
                .iter()
                .all(|side| side.faction.is_none())
        );
        assert_eq!(
            cached.first, 3,
            "country disappearance invalidates the roster before its next group boundary"
        );
    }

    #[test]
    fn air_superiority_region_catalogue_follows_deltas_partial_reads_and_recovery() {
        for countries in [1500, 5000] {
            let mut frame = dashboard_core::test_fixtures::scale_frame(1, 0, countries);
            *frame.tables[TableId::RaidAirScores.index()] =
                TableView::Complete(TableRows::RaidAirScores(
                    (0..countries)
                        .map(|country| RaidAirScoreRow {
                            country: 100_000 + country,
                            region: 7000 + country,
                            friendly: i64::from(country == 0),
                            enemy: 0,
                            impact: Default::default(),
                            friendly_missions: 0,
                            enemy_missions: 0,
                            air_spotting_scale: Some(0),
                        })
                        .collect(),
                ));
            let mut encoder = FrameEncoder::new();
            let mut decoder = FrameDecoder::for_tables(DAY_TABLES);
            let mut reads = DayReads::default();
            let mut expected = HashSet::new();
            let mut catalogue = HashSet::new();
            let mut score_read = ScoreRead::default();
            for hour in 0..5 {
                frame.hour = hour;
                if hour == 2 {
                    if let TableView::Complete(TableRows::RaidAirScores(rows)) =
                        &mut *frame.tables[TableId::RaidAirScores.index()]
                    {
                        rows[1].friendly = 2;
                    }
                } else if hour == 3 {
                    *frame.tables[TableId::RaidAirScores.index()] = TableView::Truncated {
                        missing: 1,
                        rows: TableRows::RaidAirScores(vec![RaidAirScoreRow {
                            country: 100_000,
                            region: 9900,
                            friendly: 3,
                            enemy: 0,
                            impact: Default::default(),
                            friendly_missions: 0,
                            enemy_missions: 0,
                            air_spotting_scale: Some(0),
                        }]),
                    };
                } else if hour == 4 {
                    *frame.tables[TableId::RaidAirScores.index()] =
                        TableView::Complete(TableRows::RaidAirScores(vec![RaidAirScoreRow {
                            country: 100_000,
                            region: 9999,
                            friendly: 4,
                            enemy: 0,
                            impact: Default::default(),
                            friendly_missions: 0,
                            enemy_missions: 0,
                            air_spotting_scale: Some(0),
                        }]));
                }
                let encoded = encoder.encode(frame.clone(), hour == 0).unwrap();
                if hour == 1 {
                    assert!(matches!(
                        encoded.tables[TableId::RaidAirScores.index()],
                        TableUpdate::Unchanged { .. }
                    ));
                }
                if hour == 2 {
                    let TableUpdate::Delta { changed, .. } =
                        &encoded.tables[TableId::RaidAirScores.index()]
                    else {
                        panic!("expected one score delta");
                    };
                    assert_eq!(changed.len(), 1);
                }
                let decoded = decoder.advance(&encoded).unwrap();
                let examined = score_read.observe(&encoded, &mut catalogue);
                assert_eq!(
                    examined,
                    if hour == 0 {
                        countries as usize
                    } else {
                        usize::from(hour != 1)
                    }
                );
                reads.observe(&encoded, decoded, false).unwrap();
                for row in recorded_rows::<tables::RaidAirScores>(decoded).unwrap() {
                    if row.friendly != 0 || row.enemy != 0 {
                        expected.insert(row.region);
                    }
                }
                assert_eq!(reads.regions, expected);
                assert_eq!(catalogue, expected);
                assert_eq!(reads.countries.len(), countries as usize);
            }
        }
    }

    #[test]
    fn air_superiority_auxiliary_reuses_unchanged_reads_and_observes_changed_wings() {
        let first = (0..DAY_HOURS)
            .find(|hour| !replay_data::cadence::is_refresh(TableId::AirTheatres, hour + 1))
            .unwrap();
        let mut frame = dashboard_core::test_fixtures::source(1, first, Some(1));
        *frame.tables[TableId::AirWings.index()] = TableView::Complete(TableRows::AirWings(vec![]));
        *frame.tables[TableId::AirTheatres.index()] =
            TableView::Complete(TableRows::AirTheatres(vec![]));
        *frame.tables[TableId::AirTheatreGroups.index()] =
            TableView::Complete(TableRows::AirTheatreGroups(vec![]));
        *frame.tables[TableId::AirTheatreMembers.index()] =
            TableView::Complete(TableRows::AirTheatreMembers(vec![]));
        let mut encoder = FrameEncoder::new();
        let mut decoder = FrameDecoder::for_tables(MOMENT_TABLES);
        let mut reads = AuxiliaryRead::default();
        let mut last_read = None;
        let mut previous = None;
        for offset in 0..3 {
            frame.hour = first + offset;
            if offset == 2 {
                *frame.tables[TableId::AirWings.index()] =
                    TableView::Complete(TableRows::AirWings(vec![replay_data::rows::AirWingRow {
                        id: 55,
                        owner: 9001,
                        mission_region: Some(7001),
                        ..Default::default()
                    }]));
            }
            let encoded = encoder.encode(frame.clone(), offset == 0).unwrap();
            let decoded = decoder.advance(&encoded).unwrap();
            let value = reads.read(
                dashboard_text::metadata::slot_read(
                    replay_data::format::FormatVersion::CURRENT,
                    &decoded.metrics,
                    TableId::AirTheatres,
                    decoded.hour,
                ),
                decoded,
                &mut last_read,
            );
            assert!(*value == dated_auxiliary(decoded, &mut last_read));
            if let Some(previous) = previous {
                assert_eq!(Arc::ptr_eq(&previous, &value), offset == 1);
            }
            previous = Some(value);
        }
        assert_eq!(reads.value.wings[&7001], 1);
    }
}
