use super::*;

/// The tables the construction output self-check decodes.
pub(super) const OUTPUT_TABLES: [TableId; 7] = [
    TableId::ConstructionLineOutput,
    TableId::BuildingConstruction,
    TableId::RailwayConstruction,
    TableId::StateBuildings,
    TableId::ProvinceBuildings,
    TableId::States,
    TableId::CountryLinks,
];

/// No save holds cumulative spending. Check it against successive queue reads
/// and target growth; repairs have a necessary health/cadence check.
pub(in crate::oracle) fn compare_output(
    replay: &Replay,
    frame: &DecodedFrame,
    report: &mut Report,
) -> io::Result<()> {
    report.limitation("the save keeps no per-line construction spending; branch self-checks validate progress and repair cadence, not the exact repair rate", "ConstructionLineOutput");
    report.limitation("the save does not retain deleted lines or their daily completion events; completion timestamps require the native-hook smoke comparison", "ConstructionLineOutput.completion");
    let rows = super::complete::<tables::ConstructionLineOutput>(frame)?;
    let Some(since) = rows.first().map(|row| row.since_hour) else {
        report.limitation(
            "no construction output rows at the compared hour",
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
        let mut decoder = FrameDecoder::for_tables(&OUTPUT_TABLES);
        for encoded in replay.read_block_selected(block, &OUTPUT_TABLES)? {
            if encoded.hour > frame.hour {
                break;
            }
            let decoded = decoder.advance(&encoded)?;
            if decoded.hour >= since {
                check.observe(decoded, report)?;
            }
        }
    }
    check.finish(report);
    Ok(())
}

#[derive(Clone)]
struct Published {
    output: ConstructionLineOutputRow,
    produced: i64,
    cost: i64,
    amount: i32,
    shares_segment: bool,
    /// Building target identity; changing one cannot continue a progress check.
    target: Option<(u32, u32, u32, u32, u32)>,
    country: Option<u32>,
    state_owner: Option<u32>,
    repair_levels: i32,
    building: Option<replay::rows::BuildingRow>,
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
    external_levels: usize,
    shared_railways: usize,
    seen: BTreeSet<u64>,
    output_handles: BTreeSet<u64>,
    completions: usize,
    cancellations: usize,
}

impl OutputCheck {
    pub(super) fn observe(&mut self, frame: &DecodedFrame, report: &mut Report) -> io::Result<()> {
        if !super::daily_read(frame.hour, &frame.metrics, TableId::ConstructionLineOutput) {
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
        for row in rows {
            if let replay::rows::Recorded::Value(Some(event)) = row.completion {
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
        // Progress is looked up by province pair; updates
        // write both directions. Transfers and completion
        // recompute produced from that shared progress, not this line's spend.
        let mut segments: BTreeMap<(u32, u32), BTreeSet<u64>> = BTreeMap::new();
        for line in railways {
            for pair in line.route.windows(2) {
                segments
                    .entry((pair[0].min(pair[1]), pair[0].max(pair[1])))
                    .or_default()
                    .insert(line.id);
            }
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
                        target: None,
                        country: canonical(r.country),
                        state_owner: None,
                        repair_levels: 0,
                        building: None,
                    })
            } else {
                buildings.iter().find(|r| r.id == row.line).map(|r| {
                    // CBuildingReference prefers province, then offsite country,
                    // then state (1163510). Offsite health is not recorded.
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
        if let Some((hour, before)) = self.last.take() {
            // Secondary linked lines return before H1 and mirror the primary's
            // progress. A witnessed primary makes its target a shared comparator.
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
                        // Repair advancement: 10^7 partial health per level.
                        // The game also adds BASE_FACTORY_REPAIR at zero IC;
                        // zero-IC repair advances are therefore retained as ticks.
                        let gain = (i128::from(new.healthy_level) - i128::from(old.healthy_level))
                            * i128::from(engine::cbuilding::PARTIAL_HEALTH_PER_LEVEL)
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
                if !b.railway {
                    // The game consumes this line's amount
                    // for each completed cost subtraction from its progress. A target's
                    // level can also rise through another line or an effect.
                    let mut finished = i128::from(was.amount) - i128::from(now.amount);
                    if finished < 0 {
                        self.excluded += 1;
                        continue;
                    }
                    let growth = was
                        .building
                        .as_ref()
                        .zip(now.building.as_ref())
                        .map(|(old, new)| i128::from(new.level) - i128::from(old.level));
                    // Script effects can also update levels without spending
                    // this line's IC or consuming its remaining count.
                    let reset = match (&a.count_reset, &b.count_reset) {
                        (replay::rows::Recorded::Value(a), replay::rows::Recorded::Value(b)) => {
                            Some(i128::from(*b) - i128::from(*a))
                        }
                        (
                            replay::rows::Recorded::NotRecorded,
                            replay::rows::Recorded::NotRecorded,
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
                    if finished > 0 {
                        // Ownership transfer resets the old owner's lines for this state.
                        // The reset clears count and repair levels without
                        // touching progress, spending or the building level.
                        // The game compares canonical identity, not raw tags.
                        ownership_reset = now.amount == 0
                            && was.country.is_some()
                            && was.country == now.country
                            && was.state_owner == was.country
                            && now.state_owner.is_some()
                            && was.state_owner != now.state_owner
                            && growth.is_some();
                        if let Some(reset) = reset {
                            // The native count reset observation
                            // separates cleared quantity from paid completions,
                            // even when the game lowers their target afterwards.
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
                            // An old prefix has no reset quantity. Compare the
                            // exact native cost debits: any completed quantity
                            // must be integral and within the cleared count.
                            // No debit is attributed to the reset itself.
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
                // The game forwards the overdraw to another segment. Its
                // destination can consume some or all of it; args alone do not
                // tell the discarded tail. Bound that tail, never call the
                // whole carry new spending or pretend it was all discarded.
                let valid = if b.railway {
                    carry >= 0 && spent >= expected && spent - expected <= carry
                } else {
                    // Progress addition and completed-level
                    // subtraction store signed qwords. Compare
                    // their bit patterns after widening all aggregate arithmetic.
                    carry == 0 && spent as u64 == expected as u64
                };
                if valid {
                    if b.railway {
                        self.railways += 1;
                        self.bounded_carries += usize::from(carry > 0);
                    } else {
                        self.buildings += 1;
                        self.external_levels += usize::from(external_growth);
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
    pub(super) fn finish(self, report: &mut Report) {
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
                self.shared_railways,
                "shares a segment with another railway line",
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
