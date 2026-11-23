use super::*;
use replay_data::rows::OutputPurpose;

pub(super) struct CensusCheck {
    since: Hour,
    branch: Option<replay_data::BranchId>,
    compared: usize,
    unavailable: usize,
    unclassified: usize,
}

fn produces(kind: u8, purpose: OutputPurpose) -> bool {
    use engine_native::cproduction_status::equipment_line::{
        KIND_MILITARY,
        KIND_SHIP_REFIT,
    };
    match purpose {
        OutputPurpose::Refit => kind == KIND_SHIP_REFIT,
        OutputPurpose::Converted => kind == KIND_MILITARY,
        OutputPurpose::New => kind != KIND_SHIP_REFIT,
        OutputPurpose::Unknown => true,
    }
}

impl CensusCheck {
    pub(super) fn new(since: Hour) -> Self {
        Self {
            since,
            branch: None,
            compared: 0,
            unavailable: 0,
            unclassified: 0,
        }
    }

    pub(super) fn observe(&mut self, frame: &DecodedFrame, report: &mut Report) -> io::Result<()> {
        if !daily_read(&frame.metrics, TableId::ProductionLineOutput) {
            return Ok(());
        }
        if self.branch.is_some_and(|branch| branch != frame.branch) {
            return Err(invalid(
                "production line output self-check crossed a branch",
            ));
        }
        self.branch = Some(frame.branch);
        let (Ok(rows), Ok(lines)) = (
            complete::<tables::ProductionLineOutput>(frame),
            complete::<tables::ProductionLines>(frame),
        ) else {
            self.unavailable += 1;
            return Ok(());
        };
        let mut output = BTreeMap::<u64, BTreeMap<u32, OutputPurpose>>::new();
        for row in rows {
            if row.since_hour != self.since {
                return Err(invalid(
                    "ProductionLineOutput restarted its observation within the branch",
                ));
            }
            if row.ended {
                continue;
            }
            if output
                .entry(row.line)
                .or_default()
                .insert(row.ordinal, row.purpose())
                .is_some()
            {
                return Err(invalid("duplicate production output line ordinal"));
            }
        }
        for line in lines {
            if let Some(purposes) = output.remove(&line.handle) {
                self.compared += 1;
                for (ordinal, purpose) in purposes {
                    if purpose == OutputPurpose::Unknown {
                        self.unclassified += 1;
                    } else if !produces(line.kind, purpose) {
                        report.difference(
                            format!(
                                "ProductionLineOutput {}:{ordinal}.purpose at hour {}",
                                line.handle, frame.hour
                            ),
                            purpose,
                            format!("a purpose of the census line's kind {}", line.kind),
                        );
                    }
                }
            } else {
                report.difference(
                    format!(
                        "ProductionLineOutput {} at hour {}",
                        line.handle, frame.hour
                    ),
                    "absent",
                    "live in the same publication's ProductionLines census",
                );
            }
        }
        for handle in output.keys() {
            report.difference(
                format!("ProductionLineOutput {handle} at hour {}", frame.hour),
                "live",
                "absent from the same publication's ProductionLines census",
            );
        }
        Ok(())
    }

    pub(super) fn finish(self, report: &mut Report) {
        report.compared("ProductionLineOutput census self-check", self.compared);
        if self.unclassified != 0 {
            report.limitation(
                "a live output row whose ticks the collector could not classify has no purpose to compare with its line's kind",
                format!("ProductionLineOutput purpose: {}", self.unclassified),
            );
        }
        if self.unavailable != 0 {
            report.limitation(
                "production line output or its census unavailable at a read prevents that publication's census self-check",
                format!("ProductionLineOutput census: {}", self.unavailable),
            );
        }
    }
}

#[cfg(test)]
mod tests;
