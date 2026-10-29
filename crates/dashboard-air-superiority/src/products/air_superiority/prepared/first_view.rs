//! Prepared daily values followed by cached keyframe samples. Historical side
//! keys and the most-active row selection in the tail are approximate too.
use super::*;
use dashboard_base::prepared::{
    coverage::Coverage,
    prefix::{self, Prefix},
    prepass::{Held, HeldBytes},
    query::keyframes,
};
use dashboard_base::products::country_groups::Scan as Groups;
use replay_frame::frame::FrameDecoder;

#[derive(Clone)]
struct Sample {
    snapshot: Snapshot,
    sides: Vec<AirSide>,
    countries: Arc<BTreeSet<String>>,
    auxiliary: Arc<scan::Auxiliary>,
}
impl Sample {
    fn bytes(&self) -> usize {
        let mut bytes = HeldBytes::default();
        bytes.add(size_of::<Self>());
        bytes.heap(&self.countries).heap(&self.auxiliary);
        bytes.add(self.sides.capacity() * size_of::<AirSide>());
        for side in &self.sides {
            bytes
                .heap(&side.key)
                .heap(&side.faction)
                .heap(&side.colour_faction)
                .heap(&side.tag)
                .heap(&side.name)
                .heap(&side.leader)
                .heap(&side.factions);
            bytes.add(side.reads.capacity() * size_of::<AirSideRead>());
            for (_, name, leader, faction, _, factions, colour) in &side.reads {
                bytes
                    .heap(name)
                    .heap(leader)
                    .heap(faction)
                    .heap(factions)
                    .heap(colour);
            }
        }
        let snapshot = &self.snapshot;
        bytes
            .heap(&snapshot.key_read)
            .heap(&snapshot.cells)
            .heap(&snapshot.fallback)
            .heap(&snapshot.wings)
            .heap(&snapshot.theatres)
            .heap(&snapshot.active);
        bytes.total() as usize
    }
}

impl Held for AirCell {
    fn held(&self, bytes: &mut HeldBytes) {
        match &self.0 {
            None => {}
            Some(AirValue::Reason(value)) => {
                bytes.heap(value);
            }
            Some(AirValue::Viewpoint(value)) => {
                bytes.heap(value);
            }
            Some(AirValue::Scores(value)) => {
                bytes.heap(value);
            }
            Some(AirValue::SideViewpoint { view, leader, .. }) => {
                bytes.heap(view).heap(leader);
            }
            Some(AirValue::Sides(value)) => {
                bytes
                    .heap(&value.scores)
                    .heap(&value.viewpoints)
                    .heap(&value.unknown)
                    .heap(&value.no_air_object);
            }
        }
    }
}

impl Held for scan::KeyRead {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes
            .heap(&self.countries)
            .heap(&self.sides)
            .heap(&self.published);
    }
}

impl Held for scan::Auxiliary {
    fn held(&self, bytes: &mut HeldBytes) {
        bytes
            .heap(&self.wings)
            .heap(&self.theatres)
            .heap(&self.theatre_names);
    }
}

fn relocate(cell: &mut AirCell, indices: &[u32]) {
    let Some(value) = &mut cell.0 else {
        return;
    };
    match value {
        AirValue::Scores(scores) => {
            for (side, _) in scores {
                *side = indices[*side as usize];
            }
        }
        AirValue::SideViewpoint { side, .. } => *side = indices[*side as usize],
        AirValue::Sides(cell) => {
            for (side, _) in cell.scores.iter_mut().chain(&mut cell.viewpoints) {
                *side = indices[*side as usize];
            }
            for side in cell.unknown.iter_mut().chain(&mut cell.no_air_object) {
                *side = indices[*side as usize];
            }
        }
        _ => {}
    }
}

pub(super) fn moment(
    source: &Source,
    params: &AirParams,
    context: &LoadContext,
    coverage: Option<Coverage>,
    cancelled: &AtomicBool,
) -> io::Result<serde_json::Value> {
    let branch = params.branch_id.parse().expect("validated branch");
    let (store, key) = dashboard_base::prepared::files(source, context.content.as_ref())?;
    let prefix = Prefix::new(
        source,
        branch,
        prefix::extent(source, branch, coverage.clone())?,
        params.last_hour,
    );
    let groups = if coverage.is_some() {
        read_groups(&store, &key, branch)?
    } else {
        Groups::new(source.replay_ref(), branch)
    };
    let anchors = assignment::historical_anchors(&groups);
    let mut sides = Vec::new();
    let mut reached = params.clone();
    reached.last_hour = prefix.through(params.last_hour);
    let (first, _) = hours(params)?;
    let samples = if coverage.is_some() && first <= reached.last_hour.unwrap_or(first) {
        Prepared::read_query(&store, &key, branch)?.samples(
            &store,
            &key,
            branch,
            &reached,
            &std::sync::OnceLock::new(),
            &groups,
            &anchors,
            &mut sides,
            cancelled,
        )?
    } else {
        Vec::new()
    };
    let mut reply = AirReply::Moment(moment_reply(samples, sides, &groups, params, &anchors)?);
    resolve_names(
        source,
        &groups,
        context,
        params.last_hour.unwrap_or(first),
        None,
        &mut reply,
    );
    check_reply_size(&reply)?;
    let mut data = serde_json::to_value(reply)?;
    prefix.mark_revisable(source, branch, &mut data)?;
    Ok(data)
}

pub(super) fn query(
    source: &Source,
    params: &AirParams,
    context: &LoadContext,
    coverage: Option<Coverage>,
    cancelled: &AtomicBool,
) -> io::Result<serde_json::Value> {
    let branch = params.branch_id.parse().expect("validated branch");
    let replay = source.replay_ref();
    let (store, key) = dashboard_base::prepared::files(source, context.content.as_ref())?;
    let after = coverage
        .as_ref()
        .map_or(i32::MIN, |coverage| coverage.last_hour);
    let state = coverage
        .as_ref()
        .map(|_| Prepared::read_query(&store, &key, branch))
        .transpose()?;
    let groups = if state.is_some() {
        read_groups(&store, &key, branch)?
    } else {
        Groups::new(replay, branch)
    };
    let mut cache = state
        .as_ref()
        .map_or_else(DayCache::default, |state| state.catalogue(&groups));
    // The prepared prefix's theatre membership, before the sampled tail's.
    if state.is_some()
        && params.view == AirView::Choices
        && params
            .theatres
            .as_ref()
            .is_some_and(|theatres| !theatres.is_empty())
    {
        let first = params.first_hour.unwrap_or(i32::MIN);
        let last = params.last_hour.unwrap_or(i32::MAX).min(after);
        if first <= last {
            cache.segments.push(Arc::new(super::theatre_membership(
                &store, &key, branch, first, last, cancelled,
            )?));
        }
    }
    let mut sides = (*cache.sides).clone();
    let mut segment = scan::DaySegment::default();
    let mut sample_hours = BTreeMap::new();
    if let Some(state) = state.as_ref().filter(|_| params.view == AirView::Timeline) {
        let mut reached = params.clone();
        reached.last_hour = Some(params.last_hour.unwrap_or(after).min(after));
        if reached
            .first_hour
            .is_some_and(|first| first <= reached.last_hour.unwrap())
        {
            for sample in state.samples(
                &store,
                &key,
                branch,
                &reached,
                &std::sync::OnceLock::new(),
                &groups,
                &cache.anchors,
                &mut sides,
                cancelled,
            )? {
                segment
                    .days
                    .insert(sample.hour.div_euclid(DAY_HOURS), Arc::new(sample));
            }
        }
    }
    let viewpoint = (params.mode == Some(AirMode::Viewpoint))
        .then_some(params.viewpoint.clone())
        .flatten()
        .unwrap_or_default();
    let country = params.viewpoint_country == Some(true);
    let selected = [
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::Factions,
        TableId::FactionMembers,
        TableId::WarRelations,
        TableId::LexicalTokens,
        TableId::Capitulation,
        TableId::HumanControllers,
        TableId::RaidAirScores,
        TableId::AirWings,
        TableId::AirTheatres,
        TableId::AirTheatreGroups,
        TableId::AirTheatreMembers,
    ];
    let approximate = keyframes::blocks_after(
        source,
        "air-superiority",
        branch,
        after,
        params.last_hour,
        &selected,
        &(&viewpoint, country),
        Some(&key),
        cancelled,
        &Sample::bytes,
        &|block| {
            let keyframe = replay.read_keyframe_selected(block, &selected)?;
            let mut decoder = FrameDecoder::for_tables(&selected);
            let frame = decoder.advance(&keyframe)?;
            let mut groups = Groups::new(replay, branch);
            groups.observe(frame)?;
            let mut local_sides = Vec::new();
            let mut indices = HashMap::new();
            let snapshot = scan::snapshot_kind(
                frame,
                &groups,
                (!viewpoint.is_empty()).then_some(viewpoint.as_str()),
                country,
                (&mut local_sides, &mut indices),
                &assignment::historical_anchors(&groups),
                false,
            )?;
            Ok(Some(Sample {
                snapshot,
                sides: local_sides,
                countries: scan::score_countries(frame)?.0.present,
                auxiliary: Arc::new(scan::auxiliary(frame)),
            }))
        },
        |spans, sample| {
            let mut snapshot = sample.snapshot.clone();
            let mut relocated = Vec::new();
            for side in &sample.sides {
                let at = if let Some(at) = sides.iter().position(|old| old.key == side.key) {
                    for read in &side.reads {
                        match sides[at].reads.binary_search_by_key(&read.0, |read| read.0) {
                            Ok(_) => {}
                            Err(index) => sides[at].reads.insert(index, read.clone()),
                        }
                    }
                    at
                } else {
                    sides.push(side.clone());
                    sides.len() - 1
                };
                relocated.push(at as u32);
            }
            for cell in snapshot
                .cells
                .values_mut()
                .chain(std::iter::once(&mut snapshot.fallback))
            {
                relocate(cell, &relocated);
            }
            cache.available |= snapshot.available;
            cache.regions.extend(
                snapshot
                    .active
                    .iter()
                    .copied()
                    .chain(snapshot.cells.keys().copied()),
            );
            cache.countries.extend(sample.countries.iter().cloned());
            cache.theatres.extend(
                sample
                    .auxiliary
                    .theatre_names
                    .iter()
                    .map(|(id, name)| (id.clone(), name.clone())),
            );
            let snapshot = Arc::new(snapshot);
            for span in spans {
                segment.auxiliary.push((
                    span.first_hour,
                    span.last_hour,
                    Arc::clone(&sample.auxiliary),
                ));
                for day in
                    span.first_hour.div_euclid(DAY_HOURS)..=span.last_hour.div_euclid(DAY_HOURS)
                {
                    let recorded = ((i64::from(day) + 1) * i64::from(DAY_HOURS) - 1)
                        .min(i64::from(span.last_hour)) as i32;
                    sample_hours.insert(day, snapshot.hour);
                    cache.sampled_days.insert(day, recorded);
                    segment.days.insert(day, Arc::clone(&snapshot));
                }
            }
            Ok(true)
        },
    )?;
    cache.sides = Arc::new(sides);
    cache.segments.push(Arc::new(segment));
    let end = params
        .last_hour
        .or_else(|| {
            source
                .block_windows(branch)
                .last()
                .map(|window| window.last_hour)
        })
        .unwrap_or(after);
    let mut data = serde_json::to_value(cache_reply(
        source, replay, &groups, &cache, params, context, end, cancelled,
    )?)?;
    let mut prefix = Prefix::new(
        source,
        branch,
        prefix::extent(source, branch, coverage)?,
        params.last_hour,
    );
    prefix.sampled(&approximate);
    prefix.mark(&mut data)?;
    if params.view == AirView::Timeline {
        let first = data["firstHour"].as_i64().expect("timeline first hour") as i32;
        let count = data["days"].as_array().expect("timeline days").len();
        data["sampleHours"] = serde_json::to_value(
            (0..count)
                .map(|day| {
                    sample_hours
                        .get(&(first.div_euclid(DAY_HOURS) + day as i32))
                        .copied()
                })
                .collect::<Vec<_>>(),
        )?;
    }
    let mut approximate = approximate;
    // A daily cell crossing the prefix boundary uses one sample for the whole
    // cell; mark its prepared part too rather than claim two supplying reads.
    if let Some(first) = approximate.first_mut()
        && after.div_euclid(DAY_HOURS) == first.first_hour.div_euclid(DAY_HOURS)
    {
        first.first_hour =
            source
                .block_windows(branch)
                .first()
                .map_or(first.first_hour, |window| {
                    window.first_hour.max(
                        first
                            .first_hour
                            .div_euclid(DAY_HOURS)
                            .saturating_mul(DAY_HOURS),
                    )
                });
    }
    data["approximate"] = serde_json::to_value(approximate)?;
    data["rowsApproximate"] = serde_json::Value::Bool(true);
    dashboard_base::products::check_reply_budget(&data)?;
    Ok(data)
}
