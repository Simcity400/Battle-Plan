//! Whole-branch parity oracle; production reads accepted prepared pages.
use super::*;

pub(super) fn scan(
    replay: &Replay,
    branch: u64,
    tags: &[String],
    content: Option<&super::super::operative_content::OperativeContent>,
    cancelled: &AtomicBool,
    content_error: Option<String>,
    selected_identities: Option<&BTreeSet<(u32, u32)>>,
) -> io::Result<OperativeRosterReply> {
    let selected = [
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::SimulationFeatures,
        TableId::OperativeMemberships,
        TableId::Operations,
        TableId::LexicalTokens,
        TableId::CharacterOperatives,
        TableId::CharacterPortraits,
        TableId::CountryAppearance,
        TableId::ReservedCountryNaming,
        TableId::Politics,
        TableId::PoliticalParties,
        TableId::Autonomy,
    ];
    let mut scan = Scan::new(tags, content, content_error, false)?;
    crate::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|block| block.branch == branch),
        &selected,
        cancelled,
        |blocks| {
            for (_, frames) in blocks {
                let mut decoder = FrameDecoder::for_tables(&selected);
                let Ok(frames) = frames else {
                    scan.interrupt();
                    continue;
                };
                for frame in frames {
                    crate::metadata::check_cancel(cancelled)?;
                    let Ok(decoded) = decoder.advance(&frame) else {
                        scan.interrupt();
                        break;
                    };
                    if validate_campaign(decoded, replay).is_err() {
                        break;
                    }
                    scan.observe(decoded, content, selected_identities)?;
                }
            }
            Ok(())
        },
    )?;
    scan.reply(branch, selected_identities)
}

impl Scan {
    pub(super) fn observe(
        &mut self,
        frame: &replay::frame::DecodedFrame,
        content: Option<&super::super::operative_content::OperativeContent>,
        selected_identities: Option<&BTreeSet<(u32, u32)>>,
    ) -> io::Result<()> {
        let mut countries = std::mem::take(&mut self.native_countries);
        countries.update(frame, None);
        let result = self.observe_indices(
            frame,
            content,
            selected_identities,
            None,
            None,
            (false, true),
            &countries,
        );
        self.native_countries = countries;
        result
    }
}
