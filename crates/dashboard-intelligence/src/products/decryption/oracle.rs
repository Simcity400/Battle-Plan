//! Whole-branch parity oracle; production reads accepted prepared pages.
use super::*;

pub(super) fn scan(
    replay: &Replay,
    branch: u64,
    defines: Result<cryptology::Defines, &str>,
    cancelled: &AtomicBool,
) -> io::Result<DecryptionReply> {
    if !replay.branches.iter().any(|item| item.id == branch) {
        return Err(dashboard_core::invalid("Unknown branch"));
    }
    let selected = [
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::Cryptology,
        TableId::RaidIntelInputs,
    ];
    let mut scan = Scan::default();
    dashboard_core::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|block| block.branch == branch),
        &selected,
        cancelled,
        |blocks| {
            for (block, frames) in blocks {
                metadata::check_cancel(cancelled)?;
                let mut decoder = FrameDecoder::for_tables(&selected);
                let frames = match frames {
                    Ok(frames) => frames,
                    Err(error) => {
                        scan.refusals.note(
                            format!("unreadable decryption block: {error}"),
                            block.first_hour.to_string(),
                        );
                        continue;
                    }
                };
                for frame in frames {
                    metadata::check_cancel(cancelled)?;
                    let decoded = match decoder.advance(&frame).and_then(|decoded| {
                        metadata::validate_campaign(decoded, replay)?;
                        Ok(decoded)
                    }) {
                        Ok(decoded) => decoded,
                        Err(error) => {
                            scan.refusals.note(
                                format!("unreadable decryption frame: {error}"),
                                frame.hour.to_string(),
                            );
                            break;
                        }
                    };
                    scan.frame(decoded, defines)?;
                }
            }
            Ok(())
        },
    )?;
    let reply = DecryptionReply {
        branch_id: branch.to_string(),
        pairs: scan.pairs.into_values().map(|state| state.pair).collect(),
        coverage: scan.coverage,
        warnings: scan.refusals.diagnostic_lines("cipher entries affected"),
    };
    check_reply_budget(&reply)?;
    Ok(reply)
}

impl Scan {
    pub(super) fn frame(
        &mut self,
        frame: &DecodedFrame,
        defines: Result<cryptology::Defines, &str>,
    ) -> io::Result<Vec<ObservedPair>> {
        let mut countries = std::mem::take(&mut self.native_countries);
        countries.update(frame, None);
        let read = self
            .reader
            .read(frame, defines.ok(), &countries, None);
        self.native_countries = countries;
        self.observe_read(frame.hour, &read, defines)
    }
}
