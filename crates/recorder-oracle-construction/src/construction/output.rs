use super::*;

pub(super) const OUTPUT_TABLES: [TableId; 8] = [
    TableId::ConstructionLineOutput,
    TableId::BuildingConstruction,
    TableId::RailwayConstruction,
    TableId::RailEdges,
    TableId::StateBuildings,
    TableId::ProvinceBuildings,
    TableId::States,
    TableId::CountryLinks,
];

pub fn compare_output(
    replay: &Replay,
    frame: &DecodedFrame,
    report: &mut Report,
) -> io::Result<()> {
    report.limitation("the save keeps no per-line construction spending; branch self-checks validate progress and repair cadence, not the exact repair rate", "ConstructionLineOutput");
    report.limitation("the save does not retain deleted lines or their daily completion events; completion timestamps require the native-hook smoke comparison", "ConstructionLineOutput.completion");
    let rows = super::complete::<tables::ConstructionLineOutput>(frame)?;
    let since = match rows.first() {
        Some(row) => Some(row.since_hour),
        None => observation_start(replay, frame)?,
    };
    let Some(since) = since else {
        report.limitation(
            "no construction output was published on this branch up to this hour",
            "ConstructionLineOutput",
        );
        return Ok(());
    };
    if replay
        .gaps
        .iter()
        .any(|g| g.branch == frame.branch && g.first_hour <= frame.hour && g.last_hour >= since)
    {
        report.limitation(
            "a gap since construction observation began prevents its branch self-check",
            "ConstructionLineOutput",
        );
        return Ok(());
    }
    let mut blocks: Vec<_> = replay
        .index
        .iter()
        .filter(|b| b.branch == frame.branch && b.first_hour <= frame.hour && b.last_hour >= since)
        .collect();
    blocks.sort_by_key(|b| b.first_hour);
    let mut check = OutputCheck::default();
    for block in blocks {
        replay.visit_blocks(
            &[block],
            &OUTPUT_TABLES,
            |_, _, hour_frame| {
            if hour_frame.hour > frame.hour {
                return Ok(false);
            }
            let decoded = Ok::<&replay_frame::frame::DecodedFrame, std::io::Error>(hour_frame)?;
            if decoded.hour >= since {
                check.observe(decoded, report)?;
            }
                Ok(true)
            },
            || Ok(()),
        )?;
    }
    check.finish(report, frame.hour);
    Ok(())
}

fn observation_start(replay: &Replay, frame: &DecodedFrame) -> io::Result<Option<Hour>> {
    let mut blocks: Vec<_> = replay
        .index
        .iter()
        .filter(|b| b.branch == frame.branch && b.first_hour <= frame.hour)
        .collect();
    blocks.sort_by_key(|b| std::cmp::Reverse(b.first_hour));
    for block in blocks {
        let mut since = None;
        replay.visit_blocks(
            &[block],
            &OUTPUT_TABLES,
            |_, _, hour_frame| {
            if hour_frame.hour > frame.hour {
                return Ok(false);
            }
            let decoded = Ok::<&replay_frame::frame::DecodedFrame, std::io::Error>(hour_frame)?;
            if let Some(row) = super::complete::<tables::ConstructionLineOutput>(decoded)
                .ok()
                .and_then(|rows| rows.first())
            {
                since = Some(row.since_hour);
            }
                Ok(true)
            },
            || Ok(()),
        )?;
        if since.is_some() {
            return Ok(since);
        }
    }
    Ok(None)
}

pub(super) const ENDED_ADDITION: &str = "a line with a completion beside an unwitnessed count addition ended before the compared hour; the save keeps no deleted line to certify its count";

#[derive(Clone)]
struct RailwayEnd {
    edge: (u32, u32),
    level: Option<i32>,
    selected: bool,
}

#[derive(Clone)]
struct Published {
    output: ConstructionLineOutputRow,
    produced: i64,
    cost: i64,
    amount: i32,
    shares_segment: bool,
    end: Option<RailwayEnd>,
    target: Option<(u32, u32, u32, u32, u32)>,
    country: Option<u32>,
    state_owner: Option<u32>,
    repair_levels: i32,
    building: Option<replay_data::rows::BuildingRow>,
}
#[derive(Default)]
pub(super) struct OutputCheck {
    last: Option<(Hour, BTreeMap<u64, Published>)>,
    buildings: usize,
    railways: usize,
    repairs: usize,
    natural_repairs: usize,
    excluded: usize,
    missing_buildings: usize,
    ownership_resets: usize,
    bounded_carries: usize,
    terminal_spills: usize,
    missing_terminal_levels: usize,
    external_levels: usize,
    count_additions: usize,
    uncertified_additions: BTreeMap<u64, (Hour, Hour)>,
    ended: BTreeSet<u64>,
    shared_railways: usize,
    unpaid_railways: usize,
    seen: BTreeSet<u64>,
    output_handles: BTreeSet<u64>,
    completions: usize,
    cancellations: usize,
}

impl OutputCheck {
    pub(super) fn observe(&mut self, frame: &DecodedFrame, report: &mut Report) -> io::Result<()> {
        if !super::daily_read(&frame.metrics, TableId::ConstructionLineOutput) {
            return Ok(());
        }
        let (Ok(rows), Ok(buildings), Ok(railways)) = (
            super::complete::<tables::ConstructionLineOutput>(frame),
            super::complete::<tables::BuildingConstruction>(frame),
            super::complete::<tables::RailwayConstruction>(frame),
        ) else {
            self.last = None;
            return Ok(());
        };
        self.seen.extend(
            buildings
                .iter()
                .map(|r| r.id)
                .chain(railways.iter().map(|r| r.id)),
        );
        self.output_handles.extend(rows.iter().map(|r| r.line));
        self.ended
            .extend(rows.iter().filter(|r| r.ended).map(|r| r.line));
        for row in rows {
            if let replay_data::rows::Recorded::Value(Some(event)) = row.completion {
                let absent = !buildings.iter().any(|r| r.id == row.line)
                    && !railways.iter().any(|r| r.id == row.line);
                if !row.ended
                    || !absent
                    || event.hour < row.since_hour
                    || event.hour > frame.hour
                    || (event.cancelled && !row.railway)
                {
                    report.difference(
                        format!(
                            "ConstructionLineOutput {} native completion consistency",
                            row.line
                        ),
                        format!(
                            "ended={}, absent={absent}, hour={}, cancelled={}",
                            row.ended, event.hour, event.cancelled
                        ),
                        "deleted line with an observed outcome between observation and publication",
                    );
                } else if event.cancelled {
                    self.cancellations += 1;
                } else {
                    self.completions += 1;
                }
            }
        }
        let state = super::complete::<tables::StateBuildings>(frame).ok();
        let province = super::complete::<tables::ProvinceBuildings>(frame).ok();
        let mut links = BTreeMap::new();
        if let Ok(rows) = super::complete::<tables::CountryLinks>(frame) {
            for row in rows {
                if links.insert(row.id, row.canonical_slot).is_some() {
                    return Err(invalid(
                        "duplicate country link in construction ownership inputs",
                    ));
                }
            }
        }
        let canonical = |raw: u32| {
            ((raw as i32) > 0)
                .then(|| links.get(&raw).copied())
                .flatten()
        };
        let mut owners = BTreeMap::new();
        if let Ok(rows) = super::complete::<tables::States>(frame) {
            for row in rows {
                if owners.insert(row.id, canonical(row.owner)).is_some() {
                    return Err(invalid("duplicate state in construction ownership inputs"));
                }
            }
        }
        let mut end_levels = BTreeMap::new();
        let mut segments: BTreeMap<(u32, u32), BTreeSet<u64>> = BTreeMap::new();
        for line in railways {
            if let [.., from, to] = *line.route {
                end_levels.insert((from, to), 0);
            }
            for pair in line.route.windows(2) {
                segments
                    .entry((pair[0].min(pair[1]), pair[0].max(pair[1])))
                    .or_default()
                    .insert(line.id);
            }
        }
        if let Ok(edges) = super::complete::<tables::RailEdges>(frame) {
            for edge in edges {
                if let Some(level) = end_levels.get_mut(&(edge.from, edge.to)) {
                    *level = edge.level;
                }
            }
        } else {
            end_levels.clear();
        }
        let shared: BTreeSet<u64> = segments
            .values()
            .filter(|lines| lines.len() > 1)
            .flat_map(|lines| lines.iter().copied())
            .collect();
        let mut published = BTreeMap::new();
        for row in rows.iter().filter(|r| !r.ended) {
            let p = if row.railway {
                railways
                    .iter()
                    .find(|r| r.id == row.line)
                    .map(|r| Published {
                        output: row.clone(),
                        produced: r.produced,
                        cost: r.cost,
                        amount: r.amount,
                        shares_segment: shared.contains(&r.id),
                        end: r.route.last_chunk::<2>().map(|&[from, to]| {
                            let edge = (from, to);
                            RailwayEnd {
                                edge,
                                level: end_levels.get(&edge).copied(),
                                selected: usize::try_from(r.current_segment).ok() == Some(r.route.len() - 2),
                            }
                        }),
                        target: None,
                        country: canonical(r.country),
                        state_owner: None,
                        repair_levels: 0,
                        building: None,
                    })
            } else {
                buildings.iter().find(|r| r.id == row.line).map(|r| {
                    let (values, location) = if r.target_flags & 1 != 0 {
                        (province, r.target_location)
                    } else if (r.target_country as i32) > 0 {
                        (None, 0)
                    } else {
                        (state, r.target_state)
                    };
                    Published {
                        output: row.clone(),
                        produced: r.produced,
                        cost: r.cost,
                        amount: r.amount,
                        shares_segment: false,
                        end: None,
                        target: Some((
                            r.building_type,
                            r.target_flags,
                            r.target_location,
                            r.target_state,
                            r.target_country,
                        )),
                        country: canonical(r.country),
                        state_owner: owners.get(&r.target_state).copied().flatten(),
                        repair_levels: r.repair_levels,
                        building: values
                            .and_then(|v| {
                                v.iter().find(|b| {
                                    b.location == location && b.building_type == r.building_type
                                })
                            })
                            .cloned(),
                    }
                })
            };
            if let Some(p) = p {
                published.insert(row.line, p);
            } else {
                report.difference(
                    format!(
                        "ConstructionLineOutput {} is live but absent from its queue",
                        row.line
                    ),
                    "live",
                    "absent",
                );
            }
        }
        let live: BTreeSet<u64> = rows.iter().filter(|r| !r.ended).map(|r| r.line).collect();
        for handle in buildings
            .iter()
            .map(|r| r.id)
            .chain(railways.iter().map(|r| r.id))
            .filter(|handle| !live.contains(handle))
        {
            report.difference(
                format!("ConstructionLineOutput {handle} is queued but has no live output row"),
                "absent",
                "queued",
            );
        }
        if let Some((hour, before)) = self.last.take() {
            let building_key = |p: &Published| {
                p.building
                    .as_ref()
                    .zip(p.target)
                    .map(|(b, t)| (t.1 & 1 != 0, b.location, b.building_type))
            };
            let linked_targets: BTreeSet<_> = before
                .values()
                .chain(published.values())
                .filter(|p| p.output.linked)
                .filter_map(building_key)
                .collect();
            for (&handle, now) in &published {
                let Some(was) = before.get(&handle) else {
                    continue;
                };
                let (a, b) = (&was.output, &now.output);
                if was.shares_segment || now.shares_segment {
                    self.shared_railways += 1;
                    continue;
                }
                if b.since_hour != a.since_hour
                    || !a.exact
                    || !b.exact
                    || b.linked
                    || building_key(now).is_some_and(|t| linked_targets.contains(&t))
                    || a.railway != b.railway
                    || was.target != now.target
                    || was.cost != now.cost
                {
                    self.excluded += 1;
                    continue;
                }
                if !b.railway
                    && (was.repair_levels > 0
                        || now.repair_levels > 0
                        || b.repair_ticks != a.repair_ticks)
                {
                    if let (Some(old), Some(new)) = (&was.building, &now.building) {
                        let gain = (i128::from(new.healthy_level) - i128::from(old.healthy_level))
                            * i128::from(engine_native_parts::cbuilding::PARTIAL_HEALTH_PER_LEVEL)
                            + i128::from(new.partial_health)
                            - i128::from(old.partial_health);
                        if gain > 0 && new.level == old.level {
                            if b.repair_ticks <= a.repair_ticks {
                                report.difference(format!("ConstructionLineOutput {handle} repair necessary-check {hour}..{}", frame.hour), "health rose without a repair advance", "a repair advance (factory or natural)");
                            } else if b.repair_raw == a.repair_raw && b.bank_raw == a.bank_raw {
                                self.natural_repairs += 1;
                            } else {
                                self.repairs += 1;
                            }
                        }
                    } else {
                        self.missing_buildings += 1;
                    }
                    continue;
                }
                let mut expected = i128::from(now.produced) - i128::from(was.produced);
                let spent = i128::from(b.construction_raw) - i128::from(a.construction_raw)
                    + i128::from(b.bank_raw)
                    - i128::from(a.bank_raw);
                let mut ownership_reset = false;
                let mut external_growth = false;
                let mut count_added = false;
                if !b.railway {
                    let mut finished = i128::from(was.amount) - i128::from(now.amount);
                    let growth = was
                        .building
                        .as_ref()
                        .zip(now.building.as_ref())
                        .map(|(old, new)| i128::from(new.level) - i128::from(old.level));
                    let reset = match (&a.count_reset, &b.count_reset) {
                        (
                            replay_data::rows::Recorded::Value(a),
                            replay_data::rows::Recorded::Value(b),
                        ) => Some(i128::from(*b) - i128::from(*a)),
                        (
                            replay_data::rows::Recorded::NotRecorded,
                            replay_data::rows::Recorded::NotRecorded,
                        ) => None,
                        _ => {
                            report.difference(
                                format!("ConstructionLineOutput {handle}.count_reset"),
                                "inconsistent witness presence",
                                "both publications recorded or both historical",
                            );
                            continue;
                        }
                    };
                    if finished == 0 && reset.is_some_and(|reset| reset != 0) {
                        report.difference(
                            format!("ConstructionLineOutput {handle}.count_reset"),
                            reset,
                            "zero with no remaining-count decrease",
                        );
                        continue;
                    }
                    if finished <= 0
                        && reset.is_none_or(|reset| reset == 0)
                        && was.state_owner == now.state_owner
                    {
                        let debit = spent - expected;
                        let cost = i128::from(now.cost);
                        if cost > 0 && debit > 0 && debit % cost == 0 {
                            let completed = debit / cost;
                            if growth.is_some_and(|growth| completed <= growth) {
                                finished = completed;
                                count_added = true;
                            }
                        }
                    }
                    if finished < 0 {
                        self.excluded += 1;
                        continue;
                    }
                    if finished > 0 && !count_added {
                        ownership_reset = now.amount == 0
                            && was.country.is_some()
                            && was.country == now.country
                            && was.state_owner == was.country
                            && now.state_owner.is_some()
                            && was.state_owner != now.state_owner
                            && growth.is_some();
                        if let Some(reset) = reset {
                            finished -= reset;
                            ownership_reset = reset > 0;
                            if reset < 0 || finished < 0 {
                                report.difference(
                                    format!("ConstructionLineOutput {handle}.count_reset"),
                                    reset,
                                    "reset quantity within the remaining-count decrease",
                                );
                                continue;
                            }
                        } else if ownership_reset {
                            let debit = (spent - expected) as u64;
                            let cost = now.cost as u64;
                            if debit != 0
                                && (now.cost <= 0
                                    || !debit.is_multiple_of(cost)
                                    || i128::from(debit / cost) > finished)
                            {
                                report.difference(format!("ConstructionLineOutput {handle}.construction_raw ownership reset"), spent,
                                    "exact progress plus integral completed cost debits within the cleared quantity");
                                continue;
                            }
                            finished = if debit == 0 {
                                0
                            } else {
                                i128::from(debit / cost)
                            };
                        }
                        if !ownership_reset && growth.is_none_or(|growth| growth < finished) {
                            report.difference(
                                format!("ConstructionLineOutput {handle} unexplained remaining-count decrease {hour}..{}", frame.hour),
                                format!("{} -> {}; target growth {growth:?}; canonical owners {:?} -> {:?}, line country {:?}", was.amount, now.amount, was.state_owner, now.state_owner, was.country),
                                "completed target levels or the witnessed old-owner count reset on a positive ownership transfer",
                            );
                            continue;
                        }
                    }
                    expected += finished * i128::from(now.cost);
                    external_growth = growth.is_some_and(|growth| growth > finished);
                }
                let carry = i128::from(b.carry_raw) - i128::from(a.carry_raw);
                if b.railway && b.ticks == a.ticks && spent == 0 && carry == 0 && expected > 0 {
                    self.unpaid_railways += 1;
                    continue;
                }
                let valid = if b.railway {
                    let mut within_bound = spent - expected <= carry;
                    if !within_bound && carry >= 0 && expected > 0 && b.ticks > a.ticks
                        && now.amount < was.amount
                        && let (Some(old), Some(new)) = (&was.end, &now.end)
                        && old.edge == new.edge && new.selected
                    {
                        let Some((old, new)) = old.level.zip(new.level) else {
                            self.missing_terminal_levels += 1;
                            continue;
                        };
                        within_bound = new > old;
                        self.terminal_spills += usize::from(within_bound);
                    }
                    carry >= 0 && spent >= expected && within_bound
                } else {
                    carry == 0 && spent as u64 == expected as u64
                };
                if valid {
                    if b.railway {
                        self.railways += 1;
                        self.bounded_carries += usize::from(carry > 0);
                    } else {
                        self.buildings += 1;
                        self.external_levels += usize::from(external_growth);
                        if count_added {
                            self.count_additions += 1;
                            self.uncertified_additions
                                .entry(handle)
                                .or_insert((hour, frame.hour));
                        }
                        self.ownership_resets += usize::from(ownership_reset);
                    }
                } else {
                    report.difference(format!("ConstructionLineOutput {handle}.construction_raw added from hour {hour} to {}", frame.hour), spent,
                        format!("{expected} progress including completed levels; railway discarded carry bound {carry}"));
                }
            }
        }
        self.last = Some((frame.hour, published));
        Ok(())
    }
    pub(super) fn finish(self, report: &mut Report, compared: Hour) {
        let live = match &self.last {
            Some((hour, published)) if *hour == compared => Some(published),
            _ => None,
        };
        for (&handle, &(from, to)) in &self.uncertified_additions {
            let subject = format!("ConstructionLineOutput {handle} completion beside an unwitnessed count addition {from}..{to}");
            if live.is_some_and(|published| published.contains_key(&handle)) {
                continue;
            }
            if self.ended.contains(&handle) {
                report.limitation(ENDED_ADDITION, subject);
                continue;
            }
            report.difference(
                subject,
                "line not published by a read at the compared hour",
                "the line live at the compared hour, its remaining count compared with the save",
            );
        }
        report.compared(
            "ConstructionLineOutput native completion consistency",
            self.completions,
        );
        report.compared(
            "ConstructionLineOutput native cancellation consistency",
            self.cancellations,
        );
        report.compared("ConstructionLineOutput", self.buildings + self.railways);
        report.compared("ConstructionLineOutput new buildings", self.buildings);
        report.compared(
            "ConstructionLineOutput external target growth bounds",
            self.external_levels,
        );
        report.compared(
            "ConstructionLineOutput ownership transfer count resets",
            self.ownership_resets,
        );
        report.compared(
            "ConstructionLineOutput completions beside count additions",
            self.count_additions,
        );
        report.compared("ConstructionLineOutput railways", self.railways);
        report.compared(
            "ConstructionLineOutput repair necessary-check",
            self.repairs,
        );
        report.compared(
            "ConstructionLineOutput natural repair necessary-check",
            self.natural_repairs,
        );
        for (count, reason) in [
            (
                self.terminal_spills,
                "a level completed on the selected final railway segment: spending must cover the gain, but the game discards excess there without a carry to another segment",
            ),
            (
                self.missing_terminal_levels,
                "the selected final railway segment may have completed a level, but its recorded edge level is unavailable to verify the discarded excess",
            ),
            (
                self.shared_railways,
                "shares a segment with another railway line",
            ),
            (
                self.unpaid_railways,
                "a railway line that made no spend advance still gained progress: the game recomputes it from its route's segment progress, raised by something other than this line's spending, so the gain is not its output",
            ),
            (
                self.excluded,
                "linked, instant, changed-cost, increased-count or changed-target construction stretches are not progress comparators",
            ),
            (
                self.missing_buildings,
                "building health/level operands missing (offsite health included) prevent the construction self-check",
            ),
            (
                self.bounded_carries,
                "railway carry arguments bound the discarded tail; its destination's consumed amount is not observed, so these are necessary bounds, not equality checks",
            ),
            (
                self.output_handles.difference(&self.seen).count(),
                "construction output handles absent from every branch census have unknown ownership",
            ),
        ] {
            if count > 0 {
                report.limitation(reason, format!("ConstructionLineOutput: {count}"));
            }
        }
        report.limitation("natural repair rate includes BASE_FACTORY_REPAIR and runtime country/state/template multipliers; the repair check certifies observed cadence, not the rate", "ConstructionLineOutput repairs");
        report.limitation("building target levels also change through native script effects and owner changes without this line consuming IC; progress accounting uses this line's remaining count and requires target growth or a witnessed ownership reset for decreases, without attributing every gained level to this line", "ConstructionLineOutput buildings");
    }
}
