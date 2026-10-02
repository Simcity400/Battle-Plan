use super::*;

/// The tables the construction output self-check decodes.
pub(super) const OUTPUT_TABLES: [TableId; 5] = [
    TableId::ConstructionLineOutput,
    TableId::BuildingConstruction,
    TableId::RailwayConstruction,
    TableId::StateBuildings,
    TableId::ProvinceBuildings,
];

/// No save holds cumulative spending. Check it against successive queue reads
/// and hourly building levels; repairs have a necessary health/cadence check.
pub(in crate::oracle) fn compare_output(
    replay: &Replay,
    frame: &DecodedFrame,
    report: &mut Report,
) -> io::Result<()> {
    report.limitation("the save keeps no per-line construction spending; branch self-checks validate progress and repair cadence, not the exact repair rate", "ConstructionLineOutput");
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
    shares_segment: bool,
    /// Building target identity; changing one cannot continue a progress check.
    target: Option<(u32, u32, u32, u32)>,
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
    bounded_carries: usize,
    shared_railways: usize,
    seen: BTreeSet<u64>,
    output_handles: BTreeSet<u64>,
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
        let state = super::complete::<tables::StateBuildings>(frame).ok();
        let province = super::complete::<tables::ProvinceBuildings>(frame).ok();
        // 1419EEEA0 looks progress up by province pair; 140E7DF25/3D and
        // 140E7DF4B/64 write both directions. 1419F0DF0 and 1419F0ED3/E7
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
                        shares_segment: shared.contains(&r.id),
                        target: None,
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
                        shares_segment: false,
                        target: Some((
                            r.building_type,
                            r.target_flags,
                            r.target_location,
                            r.target_state,
                        )),
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
                        // AdvanceRepair@1410C5C90: 10^7 partial health per level.
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
                if !b.railway {
                    let (Some(old), Some(new)) = (&was.building, &now.building) else {
                        self.missing_buildings += 1;
                        continue;
                    };
                    let finished = i128::from(new.level) - i128::from(old.level);
                    if finished < 0 {
                        self.excluded += 1;
                        continue;
                    }
                    expected += finished * i128::from(now.cost);
                }
                let spent = i128::from(b.construction_raw) - i128::from(a.construction_raw)
                    + i128::from(b.bank_raw)
                    - i128::from(a.bank_raw);
                let carry = i128::from(b.carry_raw) - i128::from(a.carry_raw);
                // 1419EF5C1 forwards the overdraw to another segment. Its
                // destination can consume some or all of it; args alone do not
                // tell the discarded tail. Bound that tail, never call the
                // whole carry new spending or pretend it was all discarded.
                let valid = if b.railway {
                    carry >= 0 && spent >= expected && spent - expected <= carry
                } else {
                    carry == 0 && spent == expected
                };
                if valid {
                    if b.railway {
                        self.railways += 1;
                        self.bounded_carries += usize::from(carry > 0);
                    } else {
                        self.buildings += 1;
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
        report.compared("ConstructionLineOutput", self.buildings + self.railways);
        report.compared("ConstructionLineOutput new buildings", self.buildings);
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
                "linked, instant, changed-cost or changed-target construction stretches are not progress comparators",
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
    }
}
