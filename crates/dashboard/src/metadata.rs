//! Presentation metadata for an opened recording and the shared frame checks
//! every product uses.

use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use replay::{
    BranchId,
    branches::{BranchStart, Parentage},
    container::Replay,
    frame::{DecodedFrame, FrameDecoder, TableView},
    rows::{CountryLinkRow, CountryRow, TableId, tables},
};

use crate::{BranchMetadata, CountryMetadata, RecordingGap, ReplayMetadata, invalid};

const MAX_WARNINGS: usize = 32;
const MAX_COUNTRIES: usize = 4096;
/// Admission limit on a recorded block's raw length before decompression.
pub const MAX_BLOCK_RAW_BYTES: u64 = 512 * 1024 * 1024;

/// Branch ranges, gaps and each branch's first readable country catalogue.
pub fn read(
    replay: &Arc<Replay>,
    path: &Path,
    cancelled: &AtomicBool,
) -> io::Result<ReplayMetadata> {
    check_cancel(cancelled)?;
    let branches = replay
        .branches
        .iter()
        .map(|branch| branch_metadata(replay, branch))
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
        warn(&mut metadata.warnings, "The recorder did not verify the content identity. The dashboard reads recorded values only and does not verify local game assets.".into());
    }
    for branch in &replay.branches {
        check_cancel(cancelled)?;
        let mut catalogue = Catalogue::default();
        let Some(block) = replay.index.iter().find(|block| block.branch == branch.id) else {
            continue;
        };
        if block.raw_length > MAX_BLOCK_RAW_BYTES {
            return Err(invalid(
                "Initial catalogue block exceeds the raw-block budget",
            ));
        }
        match replay.read_block(block) {
            Ok(frames) => {
                let mut decoder = FrameDecoder::new();
                for frame in frames {
                    check_cancel(cancelled)?;
                    let decoded = match decoder.advance(&frame).and_then(|decoded| {
                        validate_campaign(decoded, replay)?;
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
                        catalogue.observe(rows, links, branch.id, frame.hour)?;
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
    check_cancel(cancelled)?;
    Ok(metadata)
}

pub fn country_tables(frame: &DecodedFrame) -> io::Result<(&[CountryRow], &[CountryLinkRow])> {
    Ok((
        frame.complete::<tables::Countries>()?,
        frame.complete::<tables::CountryLinks>()?,
    ))
}

pub fn mapped_slot(row: &CountryRow, links: &[CountryLinkRow]) -> io::Result<u32> {
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
pub fn validate_campaign(frame: &DecodedFrame, replay: &Replay) -> io::Result<()> {
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
        gaps: replay
            .gaps
            .iter()
            .filter(|gap| gap.branch == branch.id)
            .map(|gap| RecordingGap {
                first_hour: gap.first_hour,
                last_hour: gap.last_hour,
                reason: gap.reason.clone(),
            })
            .collect(),
    }
}

#[derive(Default)]
pub(crate) struct Catalogue {
    rows: BTreeMap<u32, CountryMetadata>,
    bad_mapping: bool,
}

impl Catalogue {
    pub(crate) fn observe_mapped(
        &mut self,
        row: &CountryRow,
        slot: Option<u32>,
        branch: BranchId,
        hour: i32,
    ) -> io::Result<()> {
        let Some(slot) = slot else {
            self.bad_mapping = true;
            return Ok(());
        };
        if self.rows.contains_key(&slot) {
            return Ok(());
        }
        if self.rows.len() >= MAX_COUNTRIES {
            return Err(invalid("Country catalogue exceeds its budget"));
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
        Ok(())
    }

    fn observe(
        &mut self,
        rows: &[CountryRow],
        links: &[CountryLinkRow],
        branch: BranchId,
        hour: i32,
    ) -> io::Result<()> {
        for row in rows {
            self.observe_mapped(row, mapped_slot(row, links).ok(), branch, hour)?;
        }
        Ok(())
    }

    pub(crate) fn finish(self, warnings: &mut Vec<String>) -> Vec<CountryMetadata> {
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

pub(crate) fn warn(warnings: &mut Vec<String>, message: String) {
    let message = short_text(&message, 1024);
    if !warnings.contains(&message) && warnings.len() < MAX_WARNINGS {
        warnings.push(message);
    }
}

/// Bounded text for wire fields, cut at a character boundary.
pub(crate) fn short_text(text: &str, maximum: usize) -> String {
    let mut end = text.len().min(maximum);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

pub fn check_cancel(cancelled: &AtomicBool) -> io::Result<()> {
    if cancelled.load(Ordering::Relaxed) {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Dashboard request cancelled",
        ))
    } else {
        Ok(())
    }
}
