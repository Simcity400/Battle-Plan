//! Whole-branch parity oracle; production reads accepted prepared pages.
use super::*;

pub(super) fn scan(
    replay: &Replay,
    branch: u64,
    cancelled: &AtomicBool,
) -> io::Result<ScannedAgencies> {
    let selected = [
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::IntelligenceAgencies,
        TableId::AgencyUpgradeLevels,
        TableId::LexicalTokens,
        TableId::OperativeMemberships,
    ];
    let mut scanned = ScannedAgencies::default();
    dashboard_core::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|block| block.branch == branch),
        &selected,
        cancelled,
        |blocks| {
            for (_, frames) in blocks {
                check_cancelled(cancelled)?;
                let mut decoder = FrameDecoder::for_tables(&selected);
                let Ok(frames) = frames else {
                    *scanned
                        .warnings
                        .entry("Unread replay block; agency history has a gap".into())
                        .or_default() += 1;
                    continue;
                };
                for frame in frames {
                    check_cancelled(cancelled)?;
                    let Ok(decoded) = decoder.advance(&frame) else {
                        *scanned
                            .warnings
                            .entry("Unread replay frame; agency history has a gap".into())
                            .or_default() += 1;
                        break;
                    };
                    if validate_campaign(decoded, replay).is_err() {
                        *scanned
                            .warnings
                            .entry(
                                "Replay campaign identity mismatch; agency history has a gap"
                                    .into(),
                            )
                            .or_default() += 1;
                        break;
                    }
                    scanned.observe_frame(decoded, replay)?;
                }
            }
            Ok(())
        },
    )?;
    Ok(scanned)
}

impl ScannedAgencies {
    pub(super) fn observe_frame(
        &mut self,
        frame: &replay_frame::frame::DecodedFrame,
        replay: &Replay,
    ) -> io::Result<()> {
        let mut countries = std::mem::take(&mut self.native_countries);
        countries.update(frame, None);
        let mut active = BTreeMap::new();
        let result = self.observe_frame_countries(frame, replay, &countries, (&mut active, true));
        self.native_countries = countries;
        result
    }
}
