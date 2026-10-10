use super::*;
use replay::rows::Recorded;

#[derive(Clone)]
struct Published {
    line: ProductionLineRow,
    output: BTreeMap<u32, ProductionLineOutputRow>,
}

/// Spending is not saved. Consecutive complete reads can check its effect on
/// surviving lines; the save comparison separately certifies the last operands.
pub(super) struct SpendingCheck {
    since: Hour,
    branch: Option<replay::BranchId>,
    last: Option<(Hour, BTreeMap<u64, Published>)>,
    compared: usize,
    unavailable: usize,
    changed: usize,
    costs: usize,
    naval: usize,
    unsupported: usize,
    inexact: BTreeSet<(u64, u32)>,
    not_recorded: BTreeSet<(u64, u32, &'static str)>,
    refused: BTreeSet<(u64, u32, &'static str)>,
    ended: BTreeSet<u64>,
    unpaired: BTreeSet<u64>,
    partial_life: BTreeSet<u64>,
}

impl SpendingCheck {
    pub(super) fn new(since: Hour) -> Self {
        Self {
            since,
            branch: None,
            last: None,
            compared: 0,
            unavailable: 0,
            changed: 0,
            costs: 0,
            naval: 0,
            unsupported: 0,
            inexact: BTreeSet::new(),
            not_recorded: BTreeSet::new(),
            refused: BTreeSet::new(),
            ended: BTreeSet::new(),
            unpaired: BTreeSet::new(),
            partial_life: BTreeSet::new(),
        }
    }

    pub(super) fn observe(&mut self, frame: &DecodedFrame, report: &mut Report) -> io::Result<()> {
        if !daily_read(frame.hour, &frame.metrics, TableId::ProductionLineOutput) {
            return Ok(());
        }
        if self.branch.is_some_and(|branch| branch != frame.branch) {
            return Err(invalid("production spending self-check crossed a branch"));
        }
        self.branch = Some(frame.branch);
        let (Ok(rows), Ok(lines)) = (
            complete::<tables::ProductionLineOutput>(frame),
            complete::<tables::ProductionLines>(frame),
        ) else {
            self.unavailable += 1;
            self.last = None;
            return Ok(());
        };
        let mut output = BTreeMap::<u64, BTreeMap<u32, ProductionLineOutputRow>>::new();
        for row in rows {
            if row.since_hour != self.since {
                return Err(invalid(
                    "production spending restarted its observation within the branch",
                ));
            }
            if !row.observed_since_creation {
                self.partial_life.insert(row.line);
            }
            if !row.exact {
                self.inexact.insert((row.line, row.ordinal));
            }
            for (field, value) in [
                ("spent_raw", row.spent_raw),
                ("instant_raw", row.instant_raw),
            ] {
                if matches!(value, Recorded::NotRecorded) {
                    self.not_recorded.insert((row.line, row.ordinal, field));
                }
                if matches!(value, Recorded::Value(None)) {
                    self.refused.insert((row.line, row.ordinal, field));
                }
            }
            if row.ended {
                self.ended.insert(row.line);
                continue;
            }
            if output
                .entry(row.line)
                .or_default()
                .insert(row.ordinal, row.clone())
                .is_some()
            {
                return Err(invalid("duplicate production output line ordinal"));
            }
        }
        let mut published = BTreeMap::new();
        for line in lines {
            let Some(output) = output.remove(&line.handle) else {
                // ProductionLineOutputRow's live-line contract and spool
                // Totals::rows seed every census line before publication.
                report.difference(
                    format!(
                        "ProductionLineOutput {} at hour {}",
                        line.handle, frame.hour
                    ),
                    "absent",
                    "live in the same publication's ProductionLines census",
                );
                continue;
            };
            if published
                .insert(
                    line.handle,
                    Published {
                        line: line.clone(),
                        output,
                    },
                )
                .is_some()
            {
                return Err(invalid("duplicate production spending census handle"));
            }
        }
        for handle in output.keys() {
            report.difference(
                format!("ProductionLineOutput {handle} at hour {}", frame.hour),
                "live",
                "absent from the same publication's ProductionLines census",
            );
        }
        if let Some((hour, previous)) = &self.last {
            for (&handle, now) in &published {
                let Some(was) = previous.get(&handle) else {
                    self.unpaired.insert(handle);
                    continue;
                };
                if was
                    .output
                    .values()
                    .chain(now.output.values())
                    .any(|row| !row.exact)
                {
                    continue;
                }
                let (Some(before), Some(after)) = (totals(&was.output)?, totals(&now.output)?)
                else {
                    continue;
                };
                let (before, units_before, credits_before) = before;
                let (after, units_after, credits_after) = after;
                let units = units_after
                    .checked_sub(units_before)
                    .ok_or_else(|| invalid("production spending unit difference overflow"))?;
                let credits = credits_after
                    .checked_sub(credits_before)
                    .ok_or_else(|| invalid("production spending credit difference overflow"))?;
                if was.line.kind != now.line.kind
                    || was.line.country != now.line.country
                    || was.line.variant != now.line.variant
                    || was.line.mio != now.line.mio
                    || was.output.len() != now.output.len()
                    || was.output.iter().any(|(ordinal, row)| {
                        now.output.get(ordinal).is_none_or(|next| {
                            (row.country, row.variant, row.mio)
                                != (next.country, next.variant, next.mio)
                                || ((next.country, next.variant, next.mio)
                                    != (now.line.country, now.line.variant, now.line.mio)
                                    && (row.spent_raw, row.instant_raw, row.units)
                                        != (next.spent_raw, next.instant_raw, next.units))
                        })
                    })
                {
                    self.changed += 1;
                    continue;
                }
                if was.line.cost_raw != now.line.cost_raw || now.line.cost_raw <= 0 {
                    self.costs += 1;
                    continue;
                }
                // The daily update calls one production pass per country
                // at the hourly boundary. Over one day this surviving line has one tick.
                // A blocked naval deployment credits n!=0
                // without delivery and clamps progress; equality
                // of credit and delivery deltas excludes that path for one
                // tick, including signed counts. Multiple ticks can cancel.
                if now.line.kind == engine::cequipment_production_line::KIND_NAVAL
                    && (frame.hour.checked_sub(*hour) != Some(replay::cadence::STAGGER_PERIOD)
                        || credits != units)
                {
                    self.naval += 1;
                    continue;
                }
                if !matches!(
                    now.line.kind,
                    engine::cequipment_production_line::KIND_MILITARY
                        | engine::cequipment_production_line::KIND_NAVAL
                        | engine::cequipment_production_line::KIND_RAILWAY_GUN
                        | engine::cequipment_production_line::KIND_SHIP_REFIT
                ) {
                    self.unsupported += 1;
                    continue;
                }
                let spent = after
                    .checked_sub(before)
                    .ok_or_else(|| invalid("production spending input difference overflow"))?;
                let delivered = units
                    .checked_mul(i128::from(now.line.cost_raw))
                    .ok_or_else(|| invalid("production spending delivery-cost product overflow"))?;
                // Signed qword addition and subtraction wrap.
                // Reporting totals are wide;
                // only their applied low bits participate in native progress.
                let expected = was
                    .line
                    .produced_raw
                    .wrapping_add(spent as i64)
                    .wrapping_sub(delivered as i64);
                if now.line.produced_raw == expected {
                    self.compared += 1;
                } else {
                    report.difference(
                        format!(
                            "ProductionLineOutput {handle}.spent_raw + instant_raw progress from hour {hour} to {}",
                            frame.hour
                        ),
                        now.line.produced_raw,
                        format!("{expected} native wrapping progress after applied IC and delivered units at the unchanged read cost"),
                    );
                }
            }
        } else {
            self.unpaired.extend(published.keys());
        }
        self.last = Some((frame.hour, published));
        Ok(())
    }

    pub(super) fn finish(self, report: &mut Report) {
        report.compared(
            "ProductionLineOutput spending progress self-check",
            self.compared,
        );
        for (line, ordinal, field) in self.not_recorded {
            report.limitation(
                &format!("ProductionLineOutput.{field} not recorded; spending progress cannot compare intervals touching that publication"),
                format!("ProductionLineOutput {line}:{ordinal}"),
            );
        }
        for (line, ordinal, field) in self.refused {
            report.limitation(
                &format!("ProductionLineOutput.{field} unavailable after native classification or cumulative overflow; spending progress cannot compare this publication"),
                format!("ProductionLineOutput {line}:{ordinal}"),
            );
        }
        for (count, reason) in [
            (
                self.unavailable,
                "production spending operands unavailable at a read prevent a progress comparison across that read",
            ),
            (
                self.changed,
                "changed production country, design, organisation or output tuple prevents a progress comparison across that stretch",
            ),
            (
                self.costs,
                "changed or nonpositive native production cost prevents a delivered-unit progress comparison",
            ),
            (
                self.naval,
                "naval no-deploy can clamp progress to the cost: only consecutive reads one daily tick apart with equal credit and delivered-unit deltas exclude the clamp; other naval stretches lack that witness",
            ),
            (
                self.unsupported,
                "unsupported native production line kind has no proven progress comparator",
            ),
            (
                self.inexact.len(),
                "inexact delivery/MIO counts prevent the progress self-check, independently of recorded spending availability",
            ),
            (
                self.ended.len(),
                "ended production lines have no final census progress or save operand for their spending self-check",
            ),
            (
                self.unpaired.len(),
                "production lines without a previous complete progress read cannot certify their initial spending interval",
            ),
            (
                self.partial_life.len(),
                "pre-observation production spending is unknown for lines not observed since creation",
            ),
        ] {
            if count != 0 {
                report.limitation(reason, format!("ProductionLineOutput spending: {count}"));
            }
        }
        report.limitation(
            "production costs and progress are read daily, not at each tick: matching endpoint operands cannot certify an intervening cost or externally changed progress; the progress equality is a recording self-check, not an independent save certificate of cumulative spending",
            "ProductionLineOutput spending",
        );
        report.limitation(
            "native qword progress wraps; endpoint equality checks the applied low bits and cannot independently certify an unwrapped historical spending total",
            "ProductionLineOutput spending",
        );
        report.limitation(
            "progress holds factory and instant advances together; its equality checks spent_raw + instant_raw and cannot independently certify the instant-path classification or the split between those fields",
            "ProductionLineOutput instant classification",
        );
    }
}

fn totals(
    output: &BTreeMap<u32, ProductionLineOutputRow>,
) -> io::Result<Option<(i128, i128, i128)>> {
    let mut totals = (0i128, 0i128, 0i128);
    for row in output.values() {
        let (Recorded::Value(Some(spent)), Recorded::Value(Some(instant))) =
            (row.spent_raw, row.instant_raw)
        else {
            return Ok(None);
        };
        for (total, value) in [
            (&mut totals.0, i128::from(spent) + i128::from(instant)),
            (&mut totals.1, i128::from(row.units)),
            (&mut totals.2, i128::from(row.mio_credit)),
        ] {
            *total = total
                .checked_add(value)
                .ok_or_else(|| invalid("production spending self-check total overflow"))?;
        }
    }
    Ok(Some(totals))
}

#[cfg(test)]
mod tests;
