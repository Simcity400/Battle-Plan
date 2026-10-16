//! Whole-branch parity oracle; production reads accepted prepared pages.
use super::*;

pub(super) fn scan(
    source: &Source,
    branch: u64,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<OperativePlacementReply> {
    let replay = source.replay();
    let selected = [
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::SimulationFeatures,
        TableId::OperativeMemberships,
    ];
    let mut scan = Scan::default();
    crate::blocks::scan_blocks(
        &replay,
        replay.index.iter().filter(|block| block.branch == branch),
        &selected,
        cancelled,
        |blocks| {
            for (_, frames) in blocks {
                let mut decoder = FrameDecoder::for_tables(&selected);
                let Ok(frames) = frames else {
                    continue;
                };
                for frame in frames {
                    if cancelled.load(Ordering::Acquire) {
                        return Err(io::Error::new(
                            io::ErrorKind::Interrupted,
                            "Operative placement request cancelled",
                        ));
                    }
                    let Ok(decoded) = decoder.advance(&frame) else {
                        break;
                    };
                    if validate_campaign(decoded, &replay).is_err() {
                        break;
                    }
                    scan.observe(source, context, decoded)?;
                }
            }
            Ok(())
        },
    )?;
    Ok(scan.reply(branch))
}

impl Scan {
    pub(super) fn observe(
        &mut self,
        source: &Source,
        context: &LoadContext,
        decoded: &DecodedFrame,
    ) -> io::Result<()> {
        let bound = if self.catalogue.is_none()
            && operative_rows::membership_read(decoded)
            && recorded_operatives(decoded).is_some_and(|rows| !rows.is_empty())
        {
            context
                .content
                .as_ref()
                .map(|options| crate::catalogue::catalogue(source, options))
                .transpose()
                .map_err(crate::catalogue::ContentError::from)
        } else {
            Ok(self.catalogue.clone())
        };
        let mut countries = std::mem::take(&mut self.native_countries);
        countries.update(decoded, None);
        let result = self.observe_bound(decoded, &countries, &bound);
        self.native_countries = countries;
        result
    }
}

pub(super) fn identities<'a>(
    countries: &'a [CountryRow],
    links: &[CountryLinkRow],
) -> Option<HashMap<u32, &'a CountryRow>> {
    for country in countries {
        std::str::from_utf8(&country.tag).ok()?;
    }
    operative_rows::country_rows(countries, links)
}
