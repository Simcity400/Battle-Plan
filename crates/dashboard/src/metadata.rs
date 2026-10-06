//! Presentation metadata for an opened recording and the shared frame checks
//! every product uses.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
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
    frame::{DecodedFrame, FrameDecoder, Metrics, SharedView, TableView},
    rows::{CountryLinkRow, CountryRow, TableId, tables},
};

use crate::{
    BranchMetadata, CountryMetadata, PartialRecording, RecordingGap, ReplayMetadata, invalid,
};

const MAX_WARNINGS: usize = 32;

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
        campaign_key: crate::map_settings::campaign_key(replay),
        branches,
        countries: Vec::new(),
        player_country_id: None,
        content_verified: replay.content_verified(),
        partial: replay.partial().and_then(partial_recording),
        warnings: replay.compatibility_gaps(),
    };
    if !metadata.content_verified {
        warn(&mut metadata.warnings, "The recorder did not verify the content identity. The dashboard reads recorded values only and does not verify local game assets.".into());
    }
    let mut known = HashSet::new();
    for branch in &replay.branches {
        check_cancel(cancelled)?;
        let mut catalogue = Catalogue::default();
        let Some(block) = replay.index.iter().find(|block| block.branch == branch.id) else {
            continue;
        };
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
        merge_countries(&mut metadata.countries, &mut known, countries);
    }
    check_cancel(cancelled)?;
    Ok(metadata)
}

/// The terminal marker is separate from gaps: an unknown failed hour has none.
pub fn partial_recording(control: &replay::spool::ControlRecord) -> Option<PartialRecording> {
    let replay::spool::ControlRecord::Partial {
        session,
        reason,
        failed_hour,
    } = control
    else {
        return None;
    };
    Some(PartialRecording {
        session_id: session.to_string(),
        reason: reason.clone(),
        failed_hour: *failed_hour,
    })
}

/// Lists terminal metadata without reading or inflating recorded frame blocks.
pub fn read_partial_recording(path: &Path) -> io::Result<Option<PartialRecording>> {
    Ok(Replay::read_partial(path)?
        .as_ref()
        .and_then(partial_recording))
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

/// The canonical slot a raw country link id names, when the recorded links name
/// it. Recorded tables name countries by raw link id; every product resolves
/// them through this.
pub fn link_slot(links: &[CountryLinkRow], raw: u32) -> Option<u32> {
    links
        .binary_search_by_key(&raw, |link| link.id)
        .ok()
        .map(|index| links[index].canonical_slot)
}

/// Each raw country link id's tag this hour, for a scan over a branch's hours.
/// Rebuilt only when country tags, slots or links change; balance-only updates
/// keep the mapping. The views are held, not just their addresses,
/// so a freed table's address can never be mistaken for them. A link whose
/// country has no tag, or a tag that is not UTF-8, names no country.
#[derive(Default)]
pub struct LinkTags {
    tags: HashMap<u32, String>,
    from: Option<(SharedView, SharedView)>,
}

impl LinkTags {
    /// Reads this hour's country tables: `Ok(true)` when the tags were rebuilt,
    /// `Ok(false)` when their country identities are unchanged, and an error when
    /// either table is not complete (tags stay held; the next complete read rebuilds).
    pub fn update(&mut self, frame: &DecodedFrame) -> io::Result<bool> {
        let (countries, links) = match country_tables(frame) {
            Ok(tables) => tables,
            Err(error) => {
                self.from = None;
                return Err(error);
            }
        };
        let views = (
            frame.tables[TableId::Countries.index()].clone(),
            frame.tables[TableId::CountryLinks.index()].clone(),
        );
        let same = self.from.as_ref().is_some_and(|(previous, old_links)| {
            if !SharedView::ptr_eq(old_links, &views.1) {
                return false;
            }
            if SharedView::ptr_eq(previous, &views.0) {
                return true;
            }
            let TableView::Complete(rows) = &**previous else {
                return false;
            };
            let Some(old) = rows.rows::<tables::Countries>() else {
                return false;
            };
            old.len() == countries.len()
                && old.iter().zip(countries).all(|(old, new)| {
                    old.canonical_slot == new.canonical_slot && old.tag == new.tag
                })
        });
        if same {
            self.from = Some(views);
            return Ok(false);
        }
        // Resolve the link's country (140BA5140 / 140BA6240), then use its
        // recorded current tag; 140BA5C20's raw alias string is not recorded here.
        let slots: HashMap<u32, &[u8]> = countries
            .iter()
            .map(|country| (country.canonical_slot, country.tag.as_slice()))
            .collect();
        self.tags = links
            .iter()
            .filter_map(|link| {
                let tag = std::str::from_utf8(slots.get(&link.canonical_slot)?).ok()?;
                (!tag.is_empty()).then(|| (link.id, tag.to_owned()))
            })
            .collect();
        self.from = Some(views);
        Ok(true)
    }

    /// The tag a raw link id names, if any.
    pub fn get(&self, raw: u32) -> Option<&String> {
        self.tags.get(&raw)
    }

    /// Every raw link id and its tag.
    pub fn iter(&self) -> impl Iterator<Item = (&u32, &String)> {
        self.tags.iter()
    }
}

/// A tag's country hour by hour, as (canonical slot, country id): whether this
/// hour goes on from the previous one, the same country one hour later. A gap,
/// an hour without the country or another holder of the tag breaks it.
#[derive(Debug, Default)]
pub struct IdentityTrack {
    last: Option<((u32, u32), i32)>,
}

impl IdentityTrack {
    pub fn step(&mut self, identity: Option<(u32, u32)>, hour: i32) -> bool {
        let continues = identity.is_some_and(|identity| {
            self.last
                .is_some_and(|(seen, at)| seen == identity && at.checked_add(1) == Some(hour))
        });
        self.last = identity.map(|identity| (identity, hour));
        continues
    }
}

/// What complete Countries and CountryLinks tables say of a tag this hour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaggedCountry<'a> {
    /// The first row with the tag whose identity the links resolve, with its
    /// canonical slot, as every per-country product reads it.
    Found(&'a CountryRow, u32),
    /// No row has the tag: no country holds it this hour.
    Absent,
    /// Rows have the tag, but the links resolve none of them: which country
    /// holds it, and so anything about it, is unknown, never absent.
    Unresolved,
}

impl<'a> TaggedCountry<'a> {
    /// The country when found; absent and unresolved alike give none.
    pub fn found(self) -> Option<(&'a CountryRow, u32)> {
        match self {
            Self::Found(country, slot) => Some((country, slot)),
            Self::Absent | Self::Unresolved => None,
        }
    }
}

/// The country recorded under `tag`, told apart from no country holding it and
/// from one whose identity the links cannot resolve.
pub fn tagged_country<'a>(
    countries: &'a [CountryRow],
    links: &[CountryLinkRow],
    tag: &str,
) -> TaggedCountry<'a> {
    let mut named = countries
        .iter()
        .filter(|country| country.tag == tag.as_bytes())
        .peekable();
    if named.peek().is_none() {
        return TaggedCountry::Absent;
    }
    named
        .find_map(|country| {
            mapped_slot(country, links)
                .ok()
                .map(|slot| TaggedCountry::Found(country, slot))
        })
        .unwrap_or(TaggedCountry::Unresolved)
}

/// Every tag's [`TaggedCountry`] this hour in one pass over the rows, each as
/// [`tagged_country`] gives it, for a product that reads every country at once.
/// Tags are the recorded bytes.
pub fn tagged_countries<'a>(
    countries: &'a [CountryRow],
    links: &[CountryLinkRow],
) -> BTreeMap<&'a [u8], TaggedCountry<'a>> {
    let mut tagged = BTreeMap::new();
    for country in countries {
        let entry = tagged
            .entry(country.tag.as_slice())
            .or_insert(TaggedCountry::Unresolved);
        if matches!(entry, TaggedCountry::Unresolved)
            && let Ok(slot) = mapped_slot(country, links)
        {
            *entry = TaggedCountry::Found(country, slot);
        }
    }
    tagged
}

/// The recording client's stored viewpoint, including an observer's selected
/// country. Multiplayer roster order never identifies the local client.
fn player_slot(frame: &DecodedFrame, links: &[CountryLinkRow]) -> Option<u32> {
    let [player] = frame.complete::<tables::LocalPlayer>().ok()? else {
        return None;
    };
    link_slot(links, player.viewing_country()?)
}

/// Identical campaign guard to Replay::seek, including intermediate frames.
pub fn validate_campaign(frame: &DecodedFrame, replay: &Replay) -> io::Result<()> {
    if let TableView::Complete(rows) = &*frame.tables[TableId::Campaign.index()]
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

/// Whether this hour's frame read a daily table: its slot hour by the cadence
/// (`replay::cadence::is_refresh`), or any hour where a walk of a table of its
/// slot was timed, which covers full hours (branch start, load, gap, refresh)
/// and a shared read that times only its anchor (as the fresh-save oracle dates
/// daily tables, `recorder::oracle::last_read_hours`). Timing alone is not
/// enough: a shared read may be charged to a table outside the slot, as the
/// economy-rest raw copy charges its daily tables' read to the hourly
/// ProvinceSupply, so their slot hours show no timing. CountryIntel's walk can
/// move off its slot alone, so only its own timing dates it; it and the other
/// tables of its slot are no evidence for each other. A failed walk leaves its
/// table incomplete, which callers check. Other hours carry the last read.
/// Every product that dates a daily table's values by their read uses this.
pub fn slot_read(metrics: &Metrics, table: TableId, hour: i32) -> bool {
    let timed = |table: TableId| metrics.table_ns[table.index()] != 0;
    if table == TableId::CountryIntel {
        return timed(table);
    }
    let slot = replay::cadence::cadence(table);
    replay::cadence::is_refresh(table, hour)
        || TableId::ALL.into_iter().any(|other| {
            other != TableId::CountryIntel
                && replay::cadence::cadence(other) == slot
                && timed(other)
        })
}

/// A branch's first and last hour: its start, every block and every recorded gap.
pub(crate) fn branch_bounds(replay: &Replay, branch: &BranchStart) -> (i32, i32) {
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
    (first, last)
}

fn branch_metadata(replay: &Replay, branch: &BranchStart) -> BranchMetadata {
    let (first, last) = branch_bounds(replay, branch);
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
    fn observe_mapped(&mut self, row: &CountryRow, slot: Option<u32>, branch: BranchId, hour: i32) {
        let Some(slot) = slot else {
            self.bad_mapping = true;
            return;
        };
        self.rows.entry(slot).or_insert_with(|| CountryMetadata {
            id: slot.to_string(),
            tag: short_text(&String::from_utf8_lossy(&row.tag), 512),
            // Named by `Source::name_countries` once game content is known.
            name: short_text(&String::from_utf8_lossy(&row.tag), 512),
            observed_branch_id: branch.to_string(),
            observed_hour: hour,
        });
    }

    fn observe(
        &mut self,
        rows: &[CountryRow],
        links: &[CountryLinkRow],
        branch: BranchId,
        hour: i32,
    ) {
        for row in rows {
            self.observe_mapped(row, mapped_slot(row, links).ok(), branch, hour);
        }
    }

    pub(crate) fn finish(self, warnings: &mut Vec<String>) -> Vec<CountryMetadata> {
        if self.bad_mapping {
            warn(warnings, "Some country rows have missing or conflicting CountryLinks mappings and are excluded from the catalogue.".into());
        }
        self.rows.into_values().collect()
    }
}

/// One entry per (slot, tag): a country that keeps its identity across branches
/// is listed once, at its first observation in branch order, so the catalogue
/// scales with countries rather than countries x branches. Every consumer looks
/// countries up by slot or tag and takes the first match, which this keeps.
fn merge_countries(
    merged: &mut Vec<CountryMetadata>,
    known: &mut HashSet<(String, String)>,
    countries: Vec<CountryMetadata>,
) {
    for country in countries {
        if known.insert((country.id.clone(), country.tag.clone())) {
            merged.push(country);
        }
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

#[cfg(test)]
mod local_player_tests {
    use super::*;
    use replay::rows::{LocalPlayerRow, PlayerRow, TableRows};

    #[test]
    fn initial_country_uses_the_client_and_its_observer_view_not_the_mp_roster() {
        let links = [
            CountryLinkRow {
                id: 101,
                canonical_slot: 7,
            },
            CountryLinkRow {
                id: 102,
                canonical_slot: 9,
            },
        ];
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 0,
            metrics: Metrics::default(),
            tables: std::array::from_fn(|_| {
                TableView::Unavailable {
                    reason: "fixture".into(),
                }
                .into()
            }),
        };
        *frame.tables[TableId::Players.index()] =
            TableView::Complete(TableRows::Players(vec![PlayerRow {
                country: 101,
                ..Default::default()
            }]));
        assert_eq!(player_slot(&frame, &links), None);
        for (country, observer_country, expected) in [
            (102, 0, Some(9)),
            (0, 101, Some(7)),
            (0, 0, None),
            (103, 101, None),
        ] {
            *frame.tables[TableId::LocalPlayer.index()] =
                TableView::Complete(TableRows::LocalPlayer(vec![LocalPlayerRow {
                    country,
                    observer_country,
                }]));
            assert_eq!(player_slot(&frame, &links), expected);
        }
    }
}

#[cfg(test)]
mod tests;
