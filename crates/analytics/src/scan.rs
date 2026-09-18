use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

use replay::{
    BranchId,
    branches::{BranchStart, Parentage},
    container::Replay,
    frame::{DecodedFrame, FrameDecoder, TableView},
    rows::{CountryLinkRow, CountryRow, TableId, tables},
};

use crate::{
    BranchMetadata, CountryMetadata, CountrySeries, CountrySeriesRequest, MAX_POINTS, PointStatus,
    ReplayMetadata,
    reduce::{Reducer, Values, short_text},
};

const MAX_WARNINGS: usize = 32;

/// A replay plus compact presentation metadata and one completed query response.
/// Shared decoding retains its existing whole-file compressed backing and one
/// decoded block at a time. No map/game assets, database, or per-hour output cache.
pub struct Analytics {
    replay: Replay,
    metadata: ReplayMetadata,
    cache: Option<(CountrySeriesRequest, CountrySeries)>,
    #[cfg(test)]
    pub(crate) scans: usize,
}

impl Analytics {
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::open_cancellable(path, &AtomicBool::new(false))
    }

    /// Cancellation is checked while reading and between block/frame operations.
    /// A single shared read_block/decompression cannot be interrupted internally.
    pub fn open_cancellable(path: impl AsRef<Path>, cancel: &AtomicBool) -> io::Result<Self> {
        check_cancel(cancel)?;
        let path = path.as_ref().canonicalize()?;
        let bytes = std::fs::read(&path)?;
        check_cancel(cancel)?;
        Self::from_replay(Replay::open(bytes)?, path, cancel)
    }

    pub(crate) fn from_replay(
        replay: Replay,
        path: PathBuf,
        cancel: &AtomicBool,
    ) -> io::Result<Self> {
        check_cancel(cancel)?;
        let branches = replay
            .branches
            .iter()
            .map(|branch| branch_metadata(&replay, branch))
            .collect();
        let mut metadata = ReplayMetadata {
            path: path.to_string_lossy().into_owned(),
            name: path
                .file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy()
                .into_owned(),
            format: replay.format.version(),
            branches,
            countries: Vec::new(),
            player_country_id: None,
            content_verified: replay.content_verified(),
            warnings: Vec::new(),
        };
        if !metadata.content_verified {
            warn(&mut metadata.warnings, "The recorder did not verify the content identity. Analytics reads recorded values only and does not verify local game assets.".into());
        }
        for branch in &replay.branches {
            check_cancel(cancel)?;
            let mut catalogue = Catalogue::default();
            let Some(block) = replay.index.iter().find(|block| block.branch == branch.id) else {
                continue;
            };
            match replay.read_block(block) {
                Ok(frames) => {
                    let mut decoder = FrameDecoder::new();
                    for frame in frames {
                        check_cancel(cancel)?;
                        let decoded = match decoder.advance(&frame).and_then(|decoded| {
                            validate_campaign(decoded, &replay)?;
                            Ok(decoded)
                        }) {
                            Ok(decoded) => decoded,
                            Err(error) => {
                                warn(
                                    &mut metadata.warnings,
                                    format!("Branch {} catalogue: {error}", branch.id),
                                );
                                break;
                            }
                        };
                        if let Ok((rows, links)) = country_tables(decoded) {
                            catalogue.observe(rows, links, branch.id, frame.hour);
                            if metadata.player_country_id.is_none() {
                                metadata.player_country_id =
                                    player_slot(decoded, links).map(|slot| slot.to_string());
                            }
                            break;
                        }
                    }
                }
                Err(error) => warn(
                    &mut metadata.warnings,
                    format!("Branch {} initial block is unreadable: {error}", branch.id),
                ),
            }
            let countries = catalogue.finish(&mut metadata.warnings);
            merge_countries(&mut metadata, countries);
        }
        check_cancel(cancel)?;
        Ok(Self {
            replay,
            metadata,
            cache: None,
            #[cfg(test)]
            scans: 0,
        })
    }

    pub fn metadata(&self) -> &ReplayMetadata {
        &self.metadata
    }

    /// One sequential scan per uncached request. Keep this on a host worker;
    /// repeated chart renders should reuse the response, not call this again.
    /// Cancelled/failed scans leave the last completed cache and metadata intact.
    pub fn country_series(
        &mut self,
        request: &CountrySeriesRequest,
        cancel: &AtomicBool,
    ) -> io::Result<CountrySeries> {
        check_cancel(cancel)?;
        let branch_id = parse_id::<u64>(&request.branch_id, "branchId")?;
        let country_id = parse_id::<u32>(&request.country_id, "countryId")?;
        let max_points = request.max_points.unwrap_or(MAX_POINTS);
        if !(1..=MAX_POINTS).contains(&max_points) {
            return Err(input("maxPoints must be between 1 and 1200"));
        }
        let branch = self
            .replay
            .branches
            .iter()
            .find(|branch| branch.id == branch_id)
            .ok_or_else(|| input("branchId is not present in this replay"))?;
        let meta = branch_metadata(&self.replay, branch);
        let first_hour = request.first_hour.unwrap_or(meta.first_hour);
        let last_hour = request.last_hour.unwrap_or(meta.last_hour);
        if first_hour > last_hour || first_hour < meta.first_hour || last_hour > meta.last_hour {
            return Err(input(
                "firstHour/lastHour must be an ordered inclusive range inside the branch",
            ));
        }
        let key = CountrySeriesRequest {
            branch_id: branch_id.to_string(),
            country_id: country_id.to_string(),
            max_points: Some(max_points),
            first_hour: Some(first_hour),
            last_hour: Some(last_hour),
        };
        if let Some((cached_key, series)) = &self.cache
            && cached_key == &key
        {
            return Ok(series.clone());
        }
        let mut reducer = Reducer::new(first_hour, last_hour, max_points);
        let mut catalogue = Catalogue::default();
        let mut cursor = i64::from(first_hour);
        #[cfg(test)]
        {
            self.scans += 1;
        }

        for block in self.replay.index.iter().filter(|block| {
            block.branch == branch_id
                && block.last_hour >= first_hour
                && block.first_hour <= last_hour
        }) {
            check_cancel(cancel)?;
            let block_first = block.first_hour.max(first_hour);
            let block_last = block.last_hour.min(last_hour);
            if cursor < i64::from(block.first_hour) {
                fill_hole(
                    &self.replay,
                    branch_id,
                    &mut reducer,
                    cursor as i32,
                    block.first_hour - 1,
                );
                cursor = i64::from(block.first_hour);
            }
            let frames = match self.replay.read_block(block) {
                Ok(frames) => frames,
                Err(error) => {
                    reducer.missing(
                        block_first,
                        block_last,
                        PointStatus::Gap,
                        &format!("unreadable block: {error}"),
                    );
                    cursor = i64::from(block_last) + 1;
                    continue;
                }
            };
            let mut decoder = FrameDecoder::new();
            for frame in frames {
                check_cancel(cancel)?;
                if frame.hour > last_hour {
                    break;
                }
                if cursor < i64::from(frame.hour) {
                    fill_hole(
                        &self.replay,
                        branch_id,
                        &mut reducer,
                        cursor as i32,
                        frame.hour - 1,
                    );
                }
                let decoded = match decoder.advance(&frame).and_then(|decoded| {
                    validate_campaign(decoded, &self.replay)?;
                    Ok(decoded)
                }) {
                    Ok(decoded) => decoded,
                    Err(error) => {
                        // Match Replay::seek: a bad prefix invalidates the rest of
                        // this block, even when a later campaign snapshot is valid.
                        reducer.missing(
                            frame.hour.max(first_hour),
                            block_last,
                            PointStatus::Gap,
                            &format!("invalid frame baseline at hour {}: {error}", frame.hour),
                        );
                        cursor = i64::from(block_last) + 1;
                        break;
                    }
                };
                if frame.hour < first_hour {
                    continue;
                }
                match country_tables(decoded) {
                    Ok((rows, links)) => {
                        catalogue.observe(rows, links, branch_id, frame.hour);
                        match country_at(rows, links, country_id) {
                            Ok(Some(row)) => reducer.record(frame.hour, Values::from_row(row)),
                            Ok(None) => reducer.missing(
                                frame.hour,
                                frame.hour,
                                PointStatus::Absent,
                                "Country is absent from complete Countries and CountryLinks tables",
                            ),
                            Err(error) => reducer.missing(
                                frame.hour,
                                frame.hour,
                                PointStatus::Unavailable,
                                &error.to_string(),
                            ),
                        }
                    }
                    Err(error) => reducer.missing(
                        frame.hour,
                        frame.hour,
                        PointStatus::Unavailable,
                        &error.to_string(),
                    ),
                }
                cursor = i64::from(frame.hour) + 1;
            }
            if cursor <= i64::from(block_last) {
                fill_hole(
                    &self.replay,
                    branch_id,
                    &mut reducer,
                    cursor as i32,
                    block_last,
                );
                cursor = i64::from(block_last) + 1;
            }
        }
        if cursor <= i64::from(last_hour) {
            fill_hole(
                &self.replay,
                branch_id,
                &mut reducer,
                cursor as i32,
                last_hour,
            );
        }
        check_cancel(cancel)?;
        let (points, coverage) = reducer.finish();
        let mut warnings = Vec::new();
        if !self.metadata.content_verified {
            warn(&mut warnings, "Unverified recorded content identity; no local game assets were loaded or verified.".into());
        }
        let countries = catalogue.finish(&mut warnings);
        let country = countries
            .iter()
            .find(|country| country.id == request.country_id)
            .cloned();
        let series = CountrySeries {
            branch_id: meta.id,
            session_id: meta.session_id,
            country_id: country_id.to_string(),
            first_hour,
            last_hour,
            country,
            countries,
            points,
            coverage,
            downsampled: i64::from(last_hour) - i64::from(first_hour) + 1 > i64::from(max_points),
            sample_boundary: meta.sample_boundary,
            value_scale: "100000".into(),
            warnings,
        };
        check_cancel(cancel)?;
        merge_countries(&mut self.metadata, series.countries.clone());
        self.cache = Some((key, series.clone()));
        Ok(series)
    }
}

fn country_tables(frame: &DecodedFrame) -> io::Result<(&[CountryRow], &[CountryLinkRow])> {
    Ok((
        frame.complete::<tables::Countries>()?,
        frame.complete::<tables::CountryLinks>()?,
    ))
}

fn mapped_slot(row: &CountryRow, links: &[CountryLinkRow]) -> io::Result<u32> {
    let index = links
        .binary_search_by_key(&row.id, |link| link.id)
        .map_err(|_| invalid("Country has no recorded CountryLinks mapping"))?;
    let slot = links[index].canonical_slot;
    if slot != row.canonical_slot {
        return Err(invalid(
            "Countries and CountryLinks disagree on the canonical slot",
        ));
    }
    Ok(slot)
}

fn country_at<'a>(
    rows: &'a [CountryRow],
    links: &[CountryLinkRow],
    slot: u32,
) -> io::Result<Option<&'a CountryRow>> {
    let mut found = None;
    for row in rows {
        let resolved = links
            .binary_search_by_key(&row.id, |link| link.id)
            .ok()
            .map(|index| links[index].canonical_slot);
        if row.canonical_slot == slot || resolved == Some(slot) {
            mapped_slot(row, links)?;
            if found.replace(row).is_some() {
                return Err(invalid(
                    "Multiple country rows resolve to the requested canonical slot",
                ));
            }
        }
    }
    Ok(found)
}

/// The viewer's default perspective: the single player's country, else the
/// lowest human-controlled slot.
fn player_slot(frame: &DecodedFrame, links: &[CountryLinkRow]) -> Option<u32> {
    let slot = |raw: u32| {
        links
            .binary_search_by_key(&raw, |link| link.id)
            .ok()
            .map(|index| links[index].canonical_slot)
    };
    let players: BTreeSet<u32> = frame
        .complete::<tables::Players>()
        .ok()?
        .iter()
        .filter_map(|player| slot(player.country))
        .collect();
    if players.len() == 1 {
        return players.into_iter().next();
    }
    frame
        .complete::<tables::HumanControllers>()
        .ok()?
        .iter()
        .filter(|row| row.count > 0)
        .filter_map(|row| slot(row.country))
        .min()
}

/// Identical campaign guard to Replay::seek, including intermediate frames.
fn validate_campaign(frame: &DecodedFrame, replay: &Replay) -> io::Result<()> {
    if let TableView::Complete(rows) = &frame.tables[TableId::Campaign.index()]
        && rows
            .rows::<tables::Campaign>()
            .is_none_or(|rows| rows.len() != 1 || rows[0].id != replay.campaign_id)
    {
        return Err(invalid(
            "frame campaign identity differs from replay header",
        ));
    }
    Ok(())
}

fn branch_metadata(replay: &Replay, branch: &BranchStart) -> BranchMetadata {
    let mut first = branch.first_hour;
    let mut last = first;
    for block in replay
        .index
        .iter()
        .filter(|block| block.branch == branch.id)
    {
        first = first.min(block.first_hour);
        last = last.max(block.last_hour);
    }
    for gap in replay.gaps.iter().filter(|gap| gap.branch == branch.id) {
        first = first.min(gap.first_hour);
        last = last.max(gap.last_hour);
    }
    let (parent_id, parent_hour) = match branch.parent {
        Parentage::Unknown => (None, None),
        Parentage::Probable { branch, hour } => (Some(branch.to_string()), Some(hour)),
    };
    let sample_boundary = replay
        .sessions
        .iter()
        .find(|session| session.id == branch.session)
        .expect("Replay::open validated branch session")
        .sample_boundary
        .name()
        .to_owned();
    BranchMetadata {
        id: branch.id.to_string(),
        session_id: branch.session.to_string(),
        first_hour: first,
        last_hour: last,
        parent_id,
        parent_hour,
        sample_boundary,
    }
}

fn fill_hole(replay: &Replay, branch: BranchId, reducer: &mut Reducer, first: i32, last: i32) {
    // A recovered frame wins over a reported loss diagnostic. This function is
    // called only for actual holes, never over a successfully decoded frame.
    let diagnostic = replay
        .gaps
        .iter()
        .find(|gap| gap.branch == branch && gap.first_hour <= last && first <= gap.last_hour);
    let reason = diagnostic.map_or_else(
        || "hour was not recorded".to_owned(),
        |gap| {
            format!(
                "hour was not recorded; overlapping gap diagnostic {}..{}: {}",
                gap.first_hour, gap.last_hour, gap.reason,
            )
        },
    );
    reducer.missing(first, last, PointStatus::Gap, &reason);
}

#[derive(Default)]
struct Catalogue {
    rows: BTreeMap<u32, CountryMetadata>,
    bad_mapping: bool,
}

impl Catalogue {
    fn observe(
        &mut self,
        rows: &[CountryRow],
        links: &[CountryLinkRow],
        branch: BranchId,
        hour: i32,
    ) {
        for row in rows {
            let Ok(slot) = mapped_slot(row, links) else {
                self.bad_mapping = true;
                continue;
            };
            if self.rows.contains_key(&slot) {
                continue;
            }
            self.rows.insert(
                slot,
                CountryMetadata {
                    id: slot.to_string(),
                    tag: short_text(&String::from_utf8_lossy(&row.tag), 512),
                    name: short_text(&String::from_utf8_lossy(&row.name), 512),
                    observed_branch_id: branch.to_string(),
                    observed_hour: hour,
                },
            );
        }
    }

    fn finish(self, warnings: &mut Vec<String>) -> Vec<CountryMetadata> {
        if self.bad_mapping {
            warn(warnings, "Some country rows have missing or conflicting CountryLinks mappings and are excluded from the catalogue.".into());
        }
        self.rows.into_values().collect()
    }
}

fn merge_countries(metadata: &mut ReplayMetadata, countries: Vec<CountryMetadata>) {
    for country in countries {
        if metadata
            .countries
            .iter()
            .any(|old| old.id == country.id && old.observed_branch_id == country.observed_branch_id)
        {
            continue;
        }
        metadata.countries.push(country);
    }
}

fn warn(warnings: &mut Vec<String>, message: String) {
    let message = short_text(&message, 1024);
    if !warnings.contains(&message) && warnings.len() < MAX_WARNINGS {
        warnings.push(message);
    }
}

fn check_cancel(cancel: &AtomicBool) -> io::Result<()> {
    if cancel.load(Ordering::Relaxed) {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Analytics query cancelled",
        ))
    } else {
        Ok(())
    }
}

fn parse_id<T>(text: &str, field: &str) -> io::Result<T>
where
    T: std::str::FromStr + ToString,
{
    let value = text
        .parse::<T>()
        .map_err(|_| input(format!("{field} must be a decimal identifier string")))?;
    if value.to_string() != text {
        return Err(input(format!(
            "{field} must use canonical decimal notation"
        )));
    }
    Ok(value)
}

fn input(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}
fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
