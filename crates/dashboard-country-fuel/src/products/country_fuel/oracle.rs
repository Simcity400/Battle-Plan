//! Whole-branch reference scans for fuel parity tests only.
use super::*;
use replay::container::Replay;

impl Scan {
    pub(super) fn observe(
        &mut self,
        decoded: &replay::frame::DecodedFrame,
        format: replay::format::FormatVersion,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        self.observe_inner(decoded, format, None, None, cancelled)
            .map(|_| ())
    }
    pub(super) fn finish(self, format: replay::format::FormatVersion) -> ScannedBranch {
        let mut result = ScannedBranch {
            unnamed: self.unnamed,
            #[cfg(test)]
            decoded_frames: self.decoded_frames,
            #[cfg(test)]
            ownership_rows: self.ownership_rows,
            ..Default::default()
        };
        for (tag, mut country) in self.countries {
            for read in &country.result.reads {
                if let Some(instance) = read.country_instance {
                    result
                        .actor_aliases
                        .entry((read.country.to_string(), instance))
                        .or_default()
                        .insert(tag.clone());
                }
            }
            keep_days(format, &mut country.result.reads);
            result.countries.insert(tag, Arc::new(country.result));
        }
        result
    }
}

/// One branch decode feeds the shared all-country observer.
pub(super) fn scan_branch(
    replay: &Replay,
    branch: u64,
    cancelled: &AtomicBool,
) -> io::Result<ScannedBranch> {
    let selected = [
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::States,
        TableId::CountryFuel,
        TableId::FuelDemandParts,
    ];
    let mut scan = Scan::default();
    dashboard_core::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|block| block.branch == branch),
        &selected,
        cancelled,
        |blocks| {
            for (_, frames) in blocks {
                dashboard_text::metadata::check_cancel(cancelled)?;
                let mut decoder = FrameDecoder::for_tables(&selected);
                let Ok(frames) = frames else {
                    continue;
                };
                for frame in frames {
                    dashboard_text::metadata::check_cancel(cancelled)?;
                    let Ok(decoded) = decoder.advance(&frame) else {
                        break;
                    };
                    if validate_campaign(decoded, replay).is_err() {
                        break;
                    }
                    scan.observe(decoded, replay.format, cancelled)?;
                }
            }
            Ok(())
        },
    )?;
    Ok(scan.finish(replay.format))
}

/// A source-only single-country projection of the shared branch scan, as its
/// reply shows it.
#[cfg(any(test, feature = "test-support"))]
pub fn scan(
    replay: &Replay,
    branch: u64,
    tag: &str,
    cancelled: &AtomicBool,
) -> io::Result<Vec<FuelRead>> {
    Ok(scan_branch(replay, branch, cancelled)?
        .countries
        .get(tag)
        .map(|country| country.reads.iter().map(FuelRead::from).collect())
        .unwrap_or_default())
}

#[derive(Default)]
pub(super) struct Cache {
    pub(super) branches: BTreeMap<u64, Arc<ScannedBranch>>,
}

pub(super) fn scanned(
    source: &Source,
    branch: u64,
    cancelled: &AtomicBool,
) -> io::Result<Arc<ScannedBranch>> {
    let state = source.state::<Cache>();
    // Hold the same source cache lock during the first scan: concurrent country
    // requests share its result instead of each starting a branch decode.
    let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(scanned) = cache.branches.get(&branch) {
        return Ok(Arc::clone(scanned));
    }
    let scanned = Arc::new(scan_branch(&source.replay(), branch, cancelled)?);
    cache.branches.insert(branch, Arc::clone(&scanned));
    Ok(scanned)
}
