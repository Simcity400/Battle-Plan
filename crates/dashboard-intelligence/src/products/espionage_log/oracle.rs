//! Whole-branch parity oracle; production reads accepted prepared pages.
use replay_frame::frame::DecodedFrame;
use super::*;

pub(super) fn scan(
    replay: &Replay,
    params: &BranchParams,
    cancelled: &AtomicBool,
) -> io::Result<EspionageLogReply> {
    let branch = replay
        .branches
        .iter()
        .find(|branch| branch.id == params.branch())
        .ok_or_else(|| dashboard_core::invalid("Branch not in this recording"))?;
    let (first_hour, last_hour) = branch_bounds(replay, branch);
    let selected = [
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::Operations,
        TableId::OperationCompletions,
        TableId::OperationAssignments,
        TableId::OperativeMemberships,
        TableId::LexicalTokens,
    ];
    let mut builder = Builder::new(replay);
    dashboard_core::blocks::scan_blocks(
        replay,
        replay
            .index
            .iter()
            .filter(|block| block.branch == params.branch()),
        &selected,
        cancelled,
        |blocks| {
            for (_, frames) in blocks {
                let Ok(frames) = frames else {
                    builder.warn("Unread replay block; operation history has a gap");
                    continue;
                };
                for frame in frames {
                    if cancelled.load(Ordering::Acquire) {
                        return Err(io::Error::new(
                            io::ErrorKind::Interrupted,
                            "Espionage request cancelled",
                        ));
                    }
                    let Ok(decoded) = Ok::<&DecodedFrame, std::io::Error>(&frame.decoded) else {
                        builder.warn("Unread replay frame; operation history has a gap");
                        break;
                    };
                    if validate_campaign(decoded, replay).is_err() {
                        break;
                    }
                    builder.observe(decoded);
                    builder.observe_completions(decoded, replay, params.branch())?;
                }
            }
            Ok(())
        },
    )?;
    Ok(builder.reply(
        params.branch_id.clone(),
        first_hour,
        last_hour,
        replay.format.version(),
    ))
}

impl Builder {
    pub(super) fn observe(&mut self, frame: &DecodedFrame) {
        let mut countries = std::mem::take(&mut self.native_countries);
        countries.update(frame, None);
        self.observe_countries(frame, &countries);
        self.native_countries = countries;
    }
}

impl Builder {
    pub(super) fn observe_completions(
        &mut self,
        frame: &DecodedFrame,
        replay: &Replay,
        branch: u64,
    ) -> io::Result<()> {
        let mut countries = std::mem::take(&mut self.native_countries);
        countries.update(frame, None);
        let result = self.observe_completions_countries(frame, replay, branch, &countries);
        self.native_countries = countries;
        result
    }
}
