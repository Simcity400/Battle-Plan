use super::*;

/// Each publication's ProductionLineOutput against the same read's
/// ProductionLines census. ProductionLineOutputRow's live-line contract and
/// the spool's Totals::rows seed every census line before publication, so
/// every live line holds output rows and every live row's line is in the
/// census; ended rows have left it. The save keeps no per-line output, so this
/// is a recording self-check along the branch.
pub(super) struct CensusCheck {
    since: Hour,
    branch: Option<replay_data::BranchId>,
    compared: usize,
    unavailable: usize,
}

impl CensusCheck {
    pub(super) fn new(since: Hour) -> Self {
        Self {
            since,
            branch: None,
            compared: 0,
            unavailable: 0,
        }
    }

    /// A frame at or after the observation start. Only frames that read the
    /// lines' daily slot published the table; carried hours repeat the last
    /// read.
    pub(super) fn observe(&mut self, frame: &DecodedFrame, report: &mut Report) -> io::Result<()> {
        if !daily_read(frame.hour, &frame.metrics, TableId::ProductionLineOutput) {
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
        let mut output = BTreeMap::<u64, BTreeSet<u32>>::new();
        for row in rows {
            if row.since_hour != self.since {
                return Err(invalid(
                    "ProductionLineOutput restarted its observation within the branch",
                ));
            }
            if row.ended {
                continue;
            }
            if !output.entry(row.line).or_default().insert(row.ordinal) {
                return Err(invalid("duplicate production output line ordinal"));
            }
        }
        for line in lines {
            if output.remove(&line.handle).is_some() {
                self.compared += 1;
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
