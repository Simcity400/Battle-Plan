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

use replay::container::Replay;
use replay_data::{
    BranchId, format::FormatVersion, rows::CountryLinkRow, rows::CountryRow, rows::TableId,
    rows::tables,
};
use replay_frame::{
    branches::BranchStart, branches::Parentage, frame::DecodedFrame, frame::FrameDecoder,
    frame::Metrics, frame::TableView, frame::ViewIdentity,
};

use crate::{
    BranchMetadata, CountryMetadata, DamagedRecording, PartialRecording, RecordingEventLoss,
    RecordingGap, ReplayMetadata, invalid,
};

const MAX_WARNINGS: usize = 32;

/// Branch ranges, gaps and each branch's first readable country catalogue.
pub fn read(
    replay: &Arc<Replay>,
    path: &Path,
    cancelled: &AtomicBool,
) -> io::Result<ReplayMetadata> {
    read_from(replay, path, cancelled, None, 0)
}

/// Refresh trailer metadata and catalogue only branches introduced after the
/// accepted index prefix. Previously observed catalogues do not reread frames.
pub fn read_append(
    replay: &Arc<Replay>,
    path: &Path,
    cancelled: &AtomicBool,
    previous: &ReplayMetadata,
    first_new_block: usize,
) -> io::Result<ReplayMetadata> {
    read_from(replay, path, cancelled, Some(previous), first_new_block)
}

fn read_from(
    replay: &Arc<Replay>,
    path: &Path,
    cancelled: &AtomicBool,
    previous: Option<&ReplayMetadata>,
    first_new_block: usize,
) -> io::Result<ReplayMetadata> {
    check_cancel(cancelled)?;
    let branches = read_branches(replay, cancelled)?;
    let mut metadata = ReplayMetadata {
        path: path.to_string_lossy().into_owned(),
        name: path
            .file_name()
            .unwrap_or(path.as_os_str())
            .to_string_lossy()
            .into_owned(),
        format: replay.format.version(),
        campaign_key: crate::map_settings::try_campaign_key(replay)?,
        branches,
        countries: Vec::new(),
        player_country_id: None,
        content_verified: replay.content_verified(),
        partial: replay
            .partial()
            .map(partial_recording)
            .transpose()?
            .flatten(),
        damaged: (!replay.warnings.is_empty()).then(|| DamagedRecording {
            parts: replay.damaged_parts,
            first_hour: replay.damaged_range.map(|range| range.0),
            last_hour: replay.damaged_range.map(|range| range.1),
            reasons: Vec::new(),
        }),
        warnings: replay.compatibility_gaps(),
    };
    if !metadata.content_verified {
        warn(&mut metadata.warnings, "The recorder did not verify the content identity. The dashboard reads recorded values only and does not verify local game assets.".into());
    }
    for warning in &replay.warnings {
        if let Some(damaged) = &mut metadata.damaged {
            warn(&mut damaged.reasons, warning.clone());
        }
        warn(&mut metadata.warnings, warning.clone());
    }
    let mut known = HashSet::new();
    if let Some(previous) = previous {
        merge_countries(
            &mut metadata.countries,
            &mut known,
            previous.countries.clone(),
        )?;
        metadata.player_country_id = previous.player_country_id.clone();
        for warning in &previous.warnings {
            warn(&mut metadata.warnings, warning.clone());
        }
        let mut retained_damage = Vec::new();
        for old in &previous.branches {
            if let Some(current) = metadata
                .branches
                .iter_mut()
                .find(|branch| branch.id == old.id)
            {
                for gap in &old.gaps {
                    if !current.gaps.iter().any(|candidate| {
                        candidate.first_hour == gap.first_hour
                            && candidate.last_hour == gap.last_hour
                            && candidate.reason == gap.reason
                    }) {
                        current.gaps.push(gap.clone());
                        retained_damage.push(gap.clone());
                    }
                }
            }
        }
        for gap in retained_damage {
            damage(
                &mut metadata,
                1,
                Some((gap.first_hour, gap.last_hour)),
                gap.reason,
            );
        }
    }
    for branch in &replay.branches {
        if previous.is_some_and(|metadata| {
            metadata
                .branches
                .iter()
                .any(|old| old.id == branch.id.to_string())
        }) {
            continue;
        }
        // A branch's first readable block supplies its catalogue; an unreadable
        // block becomes a gap and the next is tried. Later blocks are read on demand.
        for block in replay
            .index
            .iter()
            .skip(first_new_block)
            .filter(|block| block.branch == branch.id)
        {
            check_cancel(cancelled)?;
            let mut catalogue = Catalogue::default();
            let mut player_country_id = None;
            // Only the tables the catalogue reads, up to its first complete country table.
            let mut decoder = FrameDecoder::for_tables(&CATALOGUE_TABLES);
            let read = replay.visit_block(block, Some(&CATALOGUE_TABLES), |frame| {
                check_cancel(cancelled)?;
                let decoded = decoder.advance(&frame)?;
                validate_campaign(decoded, replay)?;
                let (rows, links) = match country_tables(decoded) {
                    Ok(tables) => tables,
                    Err(error) if engine_allocation::is_allocation_error(&error) => {
                        return Err(error);
                    }
                    Err(_) => return Ok(true),
                };
                catalogue.observe(rows, links, branch.id, frame.hour)?;
                player_country_id = player_slot(decoded, links)?.map(|slot| slot.to_string());
                Ok(false)
            });
            if let Err(error) = read {
                check_cancel(cancelled)?;
                if engine_allocation::is_allocation_error(&error) {
                    return Err(error);
                }
                let reason = replay_data::codec::try_format(format_args!(
                    "Branch {} block {} – {} is unreadable: {}",
                    branch.id,
                    block.first_hour,
                    block.last_hour,
                    replay_data::codec::diagnostic_text(&error.to_string())
                ))?;
                let branch_metadata = metadata
                    .branches
                    .iter_mut()
                    .find(|entry| entry.id == branch.id.to_string())
                    .unwrap();
                engine_allocation::reserve(&mut branch_metadata.gaps, 1)?;
                branch_metadata.gaps.push(RecordingGap {
                    first_hour: block.first_hour,
                    last_hour: block.last_hour,
                    reason: reason.clone(),
                });
                damage(
                    &mut metadata,
                    1,
                    Some((block.first_hour, block.last_hour)),
                    reason,
                );
                continue;
            }
            if metadata.player_country_id.is_none() {
                metadata.player_country_id = player_country_id;
            }
            let countries = catalogue.finish(&mut metadata.warnings)?;
            merge_countries(&mut metadata.countries, &mut known, countries)?;
            break;
        }
    }
    check_cancel(cancelled)?;
    Ok(metadata)
}

fn damage(metadata: &mut ReplayMetadata, parts: usize, range: Option<(i32, i32)>, reason: String) {
    let damaged = metadata
        .damaged
        .get_or_insert_with(DamagedRecording::default);
    damaged.parts += parts;
    if let Some((first, last)) = range {
        damaged.first_hour = Some(damaged.first_hour.map_or(first, |hour| hour.min(first)));
        damaged.last_hour = Some(damaged.last_hour.map_or(last, |hour| hour.max(last)));
    }
    warn(&mut damaged.reasons, reason.clone());
    warn(&mut metadata.warnings, reason);
}

/// The terminal marker is separate from gaps: an unknown failed hour has none.
pub fn partial_recording(
    control: &replay::spool::ControlRecord,
) -> io::Result<Option<PartialRecording>> {
    let replay::spool::ControlRecord::Partial {
        session,
        reason,
        failed_hour,
    } = control
    else {
        return Ok(None);
    };
    Ok(Some(PartialRecording {
        session_id: replay_data::codec::try_format(format_args!("{session}"))?,
        reason: engine_allocation::string(reason)?,
        failed_hour: *failed_hour,
    }))
}

/// Lists terminal metadata without reading or inflating recorded frame blocks.
pub fn read_partial_recording(path: &Path) -> io::Result<Option<PartialRecording>> {
    Replay::read_partial(path)?
        .as_ref()
        .map(partial_recording)
        .transpose()
        .map(Option::flatten)
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
/// keep the mapping. The tables are named by identity, which cannot match a
/// later allocation and does not hold their rows, so a later hour's update is
/// never copied for them. A link whose country has no tag, or a tag that is not
/// UTF-8, names no country.
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct LinkTags {
    #[serde(serialize_with = "dashboard_core::map_pairs::ordered_map")]
    tags: HashMap<u32, String>,
    #[serde(skip)]
    from: Option<ReadFrom>,
}

/// The country tables last read, and each country's slot and tag then.
#[derive(Clone)]
struct ReadFrom {
    countries: Option<ViewIdentity>,
    links: Option<ViewIdentity>,
    slots: Vec<(u32, Vec<u8>)>,
    link_slots: HashMap<u32, u32>,
}

impl ReadFrom {
    fn tags(&self) -> HashMap<u32, String> {
        // Raw aliases use the canonical slot's current tag, even when the
        // Countries row's own raw id is absent from the recorded link table.
        let slots: HashMap<u32, &[u8]> = self
            .slots
            .iter()
            .map(|(slot, tag)| (*slot, tag.as_slice()))
            .collect();
        self.link_slots
            .iter()
            .filter_map(|(&raw, slot)| {
                let tag = std::str::from_utf8(slots.get(slot)?).ok()?;
                (!tag.is_empty()).then(|| (raw, tag.to_owned()))
            })
            .collect()
    }
}

/// Each country's canonical slot and tag, and each raw link's slot, that
/// [`LinkTags`] were built from.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct LinkProjection {
    slots: Vec<(u32, Vec<u8>)>,
    links: Vec<(u32, u32)>,
}

impl LinkTags {
    /// Bytes the tags' allocations hold, about: each tag by raw link, and the
    /// slots and links they were read from.
    pub fn heap_bytes(&self) -> usize {
        // A hash table's buckets: at most seven eighths full, a control byte each.
        let table = |capacity: usize, entry: usize| capacity * 8 / 7 * (entry + 1);
        table(self.tags.capacity(), size_of::<(u32, String)>())
            + self.tags.values().map(String::capacity).sum::<usize>()
            + self.from.as_ref().map_or(0, |from| {
                from.slots.capacity() * size_of::<(u32, Vec<u8>)>()
                    + from
                        .slots
                        .iter()
                        .map(|(_, tag)| tag.capacity())
                        .sum::<usize>()
                    + table(from.link_slots.capacity(), size_of::<(u32, u32)>())
            })
    }

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
            &frame.tables[TableId::Countries.index()],
            &frame.tables[TableId::CountryLinks.index()],
        );
        if let Some(from) = &mut self.from
            && from.links.as_ref().is_some_and(|seen| seen.is(views.1))
            && (from.countries.as_ref().is_some_and(|seen| seen.is(views.0))
                || from.slots.len() == countries.len()
                    && from
                        .slots
                        .iter()
                        .zip(countries)
                        .all(|((slot, tag), new)| *slot == new.canonical_slot && *tag == new.tag))
        {
            from.countries = Some(views.0.identity());
            return Ok(false);
        }
        let countries = CountryKeys::read(countries);
        let links = link_slots(links);
        let rebuilt = self.update_projected(Some(&countries), Some(&links))?;
        let from = self.from.as_mut().expect("complete country projections");
        from.countries = Some(views.0.identity());
        from.links = Some(views.1.identity());
        Ok(rebuilt)
    }

    /// Rebind the same country/link join from runtime current operands. Incomplete
    /// inputs leave tags held but invalidate reuse, exactly as `update` does.
    pub fn update_projected(
        &mut self,
        countries: Option<&CountryKeys>,
        links: Option<&HashMap<u32, u32>>,
    ) -> io::Result<bool> {
        let Some((countries, links)) = countries.zip(links) else {
            self.from = None;
            return Err(invalid("Country identity tables not recorded completely"));
        };
        if let Some(from) = &self.from
            && from.link_slots == *links
            && from.slots.len() == countries.0.len()
            && from.slots.iter().zip(countries.iter()).all(
                |((slot, tag), (_, new_slot, new_tag))| {
                    *slot == new_slot && tag.as_slice() == new_tag
                },
            )
        {
            return Ok(false);
        }
        let from = ReadFrom {
            countries: None,
            links: None,
            slots: countries
                .iter()
                .map(|(_, slot, tag)| (slot, tag.to_vec()))
                .collect(),
            link_slots: links.clone(),
        };
        self.tags = from.tags();
        self.from = Some(from);
        Ok(true)
    }

    /// The country projection the tags were last built from, for state saved
    /// beside them: an update after [`LinkTags::from_projection`] compares
    /// against it as a continued scan compares against its last read.
    pub fn projection(&self) -> Option<LinkProjection> {
        self.from.as_ref().map(|from| {
            let mut links: Vec<_> = from
                .link_slots
                .iter()
                .map(|(&raw, &slot)| (raw, slot))
                .collect();
            links.sort_unstable();
            LinkProjection {
                slots: from.slots.clone(),
                links,
            }
        })
    }

    /// Tags built from a saved [`LinkTags::projection`].
    pub fn from_projection(projection: LinkProjection) -> Self {
        let from = ReadFrom {
            countries: None,
            links: None,
            slots: projection.slots,
            link_slots: projection.links.into_iter().collect(),
        };
        Self {
            tags: from.tags(),
            from: Some(from),
        }
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

/// Each country's id, slot and tag in table order: all that country identity
/// reads from Countries, whose balances change it every hour.
#[derive(Clone, Debug, Default)]
pub struct CountryKeys(
    Vec<(u32, u32, Vec<u8>)>,
    #[cfg(any(test, feature = "test-support"))] std::sync::Arc<std::sync::atomic::AtomicUsize>,
);

impl CountryKeys {
    pub fn read(countries: &[CountryRow]) -> Self {
        Self(
            countries
                .iter()
                .map(|row| (row.id, row.canonical_slot, row.tag.clone()))
                .collect(),
            #[cfg(any(test, feature = "test-support"))]
            Default::default(),
        )
    }

    /// Borrow retained id, canonical slot and tag operands in native table order.
    pub fn iter(&self) -> impl Iterator<Item = (u32, u32, &[u8])> {
        self.0
            .iter()
            .map(|(id, slot, tag)| (*id, *slot, tag.as_slice()))
    }

    /// Bytes the keys' allocations hold: the list and each tag.
    pub fn heap_bytes(&self) -> usize {
        self.0.capacity() * size_of::<(u32, u32, Vec<u8>)>()
            + self
                .0
                .iter()
                .map(|(_, _, tag)| tag.capacity())
                .sum::<usize>()
    }

    /// Whether `countries` hold these keys.
    pub fn same(&self, countries: &[CountryRow]) -> bool {
        self.0.len() == countries.len()
            && self.0.iter().zip(countries).all(|((id, slot, tag), row)| {
                #[cfg(any(test, feature = "test-support"))]
                self.1.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                *id == row.id && *slot == row.canonical_slot && *tag == row.tag
            })
    }
    /// Whether the previous hour's key projection still holds after these
    /// recorded row changes. Snapshots and identity edits compare normally.
    pub fn same_changes(
        &self,
        countries: &[CountryRow],
        delta: Option<(&[replay_data::rows::DeltaRow], &[u128])>,
    ) -> bool {
        self.kept_changes(countries.len(), delta) || self.same(countries)
    }

    /// Whether one consecutive hour's delta proves these identity operands
    /// unchanged, without walking the keys. Callers delaying their comparison
    /// must retain every failed proof until they compare the full projection.
    pub fn kept_changes(
        &self,
        country_count: usize,
        delta: Option<(&[replay_data::rows::DeltaRow], &[u128])>,
    ) -> bool {
        let _cpu = replay_frame::cpu::scope_phase("country_keys:changed_fields");
        static FIELDS: std::sync::LazyLock<u64> = std::sync::LazyLock::new(|| {
            let mask = (0..64)
                .filter(|&field| {
                    TableId::Countries
                        .field_name(field)
                        .is_some_and(|name| matches!(name, "canonical_slot" | "tag"))
                })
                .fold(0_u64, |mask, field| mask | (1_u64 << field));
            if mask.count_ones() == 2 {
                mask
            } else {
                u64::MAX
            }
        });
        self.0.len() == country_count
            && delta.is_some_and(|(changed, deleted)| {
                deleted.is_empty()
                    && changed.iter().all(|row| {
                        #[cfg(any(test, feature = "test-support"))]
                        self.1.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        row.mask & *FIELDS == 0
                    })
            })
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn row_visits(&self) -> usize {
        self.1.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// One raw link's canonical slot only when every row agrees. True aliases
/// remain valid; a contradictory raw link cannot name any country safely.
pub fn link_slots(links: &[CountryLinkRow]) -> HashMap<u32, u32> {
    let mut slots = HashMap::new();
    let mut ambiguous = std::collections::HashSet::new();
    for link in links {
        if slots
            .insert(link.id, link.canonical_slot)
            .is_some_and(|slot| slot != link.canonical_slot)
        {
            ambiguous.insert(link.id);
        }
    }
    for raw in ambiguous {
        slots.remove(&raw);
    }
    slots
}

/// A tag's country hour by hour, as (canonical slot, country id): whether this
/// hour goes on from the previous one, the same country one hour later. A gap,
/// an hour without the country or another holder of the tag breaks it.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct IdentityTrack {
    last: Option<((u32, u32), i32)>,
}

impl IdentityTrack {
    /// Extend the known holder through authenticated unchanged hours. Callers
    /// close at a gap with `step(None, hour)` before observing its next holder.
    pub fn advance(&mut self, hour: i32) {
        if let Some((_, frontier)) = &mut self.last {
            *frontier = (*frontier).max(hour);
        }
    }
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

/// The same lineage lookup using a caller's raw-link index, for an hourly product
/// that already aggregates other tables by slot. Each recorded row is visited once.
pub fn tagged_countries_indexed<'a>(
    countries: &'a [CountryRow],
    slots: &HashMap<u32, u32>,
) -> BTreeMap<&'a [u8], TaggedCountry<'a>> {
    let mut tagged = BTreeMap::new();
    for country in countries {
        let entry = tagged
            .entry(country.tag.as_slice())
            .or_insert(TaggedCountry::Unresolved);
        if matches!(entry, TaggedCountry::Unresolved)
            && let Some(&slot) = slots.get(&country.id)
            && slot == country.canonical_slot
        {
            *entry = TaggedCountry::Found(country, slot);
        }
    }
    tagged
}

/// The tables the catalogue reads from each branch's first frames.
const CATALOGUE_TABLES: [TableId; 4] = [
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::LocalPlayer,
];

/// The local player's slot; no player when the table is not complete. Only an
/// allocation failure is an error.
fn player_slot(frame: &DecodedFrame, links: &[CountryLinkRow]) -> io::Result<Option<u32>> {
    let players = match frame.complete::<tables::LocalPlayer>() {
        Ok(players) => players,
        Err(error) if engine_allocation::is_allocation_error(&error) => return Err(error),
        Err(_) => return Ok(None),
    };
    let [player] = players else {
        return Ok(None);
    };
    Ok(player
        .viewing_country()
        .and_then(|country| link_slot(links, country)))
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
/// (`replay_data::cadence::is_refresh`), or any hour where a walk of a table of its
/// slot was timed, which covers full hours (branch start, load, gap, refresh)
/// and a shared read that times only its anchor (as the fresh-save oracle dates
/// daily tables, `recorder::oracle::last_read_hours`). Timing alone is not
/// enough: a shared read may be charged to a table outside the slot, as the
/// economy-rest raw copy charges its daily tables' read to the hourly
/// ProvinceSupply, so their slot hours show no timing. CountryIntel's walk can
/// move off its slot alone, so only its own timing dates it; it and the other
/// tables of its slot are no evidence for each other. A failed walk leaves its
/// table incomplete, which callers check. Other hours carry the last read.
/// Cadences are those `format` recorded with (`replay_data::cadence::cadence_in`):
/// a table read daily then is dated at its slot even if it is hourly now.
/// Every product that dates a daily table's values by their read uses this.
pub fn slot_read(format: FormatVersion, metrics: &Metrics, table: TableId, hour: i32) -> bool {
    if table == TableId::CountryIntel {
        return metrics.table_ns[table.index()] != 0;
    }
    if replay_data::cadence::cadence_in(table, format).is_refresh(hour) {
        return true;
    }
    slot_reads(format, metrics, hour)[table.index()]
}

/// Every table's supplying-read flag, as [`slot_read`] decides it. The
/// recording's cadence groups are made once per thread and format; one
/// hour's read evidence is shared by consumers with the same timed tables.
pub fn slot_reads(
    format: FormatVersion,
    metrics: &Metrics,
    hour: i32,
) -> [bool; replay_data::rows::TABLE_COUNT] {
    SLOT_READS.with_borrow_mut(|held| {
        if held.as_ref().is_none_or(|held| held.format != format) {
            *held = Some(ReadSlots::new(format));
        }
        held.as_mut()
            .expect("the recording's read slots")
            .at(metrics, hour)
    })
}

struct ReadSlots {
    format: FormatVersion,
    groups: Vec<(replay_data::cadence::Cadence, Vec<TableId>)>,
    last: Option<(
        i32,
        [bool; replay_data::rows::TABLE_COUNT],
        [bool; replay_data::rows::TABLE_COUNT],
    )>,
}

impl ReadSlots {
    fn new(format: FormatVersion) -> Self {
        let mut groups: Vec<(replay_data::cadence::Cadence, Vec<TableId>)> = Vec::new();
        for table in TableId::ALL {
            // This table's measured walk dates only itself.
            if table == TableId::CountryIntel {
                continue;
            }
            let cadence = replay_data::cadence::cadence_in(table, format);
            match groups.iter_mut().find(|(group, _)| *group == cadence) {
                Some((_, tables)) => tables.push(table),
                None => groups.push((cadence, vec![table])),
            }
        }
        Self {
            format,
            groups,
            last: None,
        }
    }

    fn at(&mut self, metrics: &Metrics, hour: i32) -> [bool; replay_data::rows::TABLE_COUNT] {
        let timed = metrics.table_ns.map(|duration| duration != 0);
        if let Some((previous, reads, flags)) = &self.last
            && *previous == hour
            && *reads == timed
        {
            return *flags;
        }
        let _cpu = replay_frame::cpu::scope_phase("slot_reads:refresh");
        #[cfg(any(test, feature = "test-support"))]
        SLOT_READ_BATCHES.with(|count| count.set(count.get() + 1));
        let mut flags = [false; replay_data::rows::TABLE_COUNT];
        for (cadence, tables) in &self.groups {
            let read = cadence.is_refresh(hour) || tables.iter().any(|table| timed[table.index()]);
            for table in tables {
                flags[table.index()] = read;
            }
        }
        flags[TableId::CountryIntel.index()] = timed[TableId::CountryIntel.index()];
        self.last = Some((hour, timed, flags));
        flags
    }
}

thread_local! {
    static SLOT_READS: std::cell::RefCell<Option<ReadSlots>> = const { std::cell::RefCell::new(None) };
    #[cfg(any(test, feature = "test-support"))]
    static SLOT_READ_BATCHES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Distinct supplying-read batches computed on this thread.
#[cfg(any(test, feature = "test-support"))]
pub fn slot_read_batches() -> usize {
    SLOT_READ_BATCHES.with(std::cell::Cell::get)
}

/// Why a recorded event loss happened: only a same-process reattach writes one.
pub const EVENT_LOSS_REASON: &str = "The recorder was detached for part of the interval between these reads; its recorded events are incomplete.";

/// A branch's first and last hour: its start, every block and every recorded gap.
pub fn branch_bounds(replay: &Replay, branch: &BranchStart) -> (i32, i32) {
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

fn read_branches(replay: &Replay, cancelled: &AtomicBool) -> io::Result<Vec<BranchMetadata>> {
    let mut branches = engine_allocation::vec(replay.branches.len())?;
    for branch in &replay.branches {
        check_cancel(cancelled)?;
        let (first_hour, last_hour) = branch_bounds(replay, branch);
        let (parent_id, parent_hour) = match branch.parent {
            Parentage::Unknown => (None, None),
            Parentage::Probable { branch, hour } => (Some(branch.to_string()), Some(hour)),
        };
        let session = replay
            .sessions
            .iter()
            .find(|session| session.id == branch.session)
            .ok_or_else(|| invalid("branch refers to an unknown session"))?;
        let mut gaps = Vec::new();
        for gap in replay.gaps.iter().filter(|gap| gap.branch == branch.id) {
            engine_allocation::reserve(&mut gaps, 1)?;
            gaps.push(RecordingGap {
                first_hour: gap.first_hour,
                last_hour: gap.last_hour,
                reason: engine_allocation::string(&gap.reason)?,
            });
        }
        let mut event_losses = Vec::new();
        for (after_hour, hour) in replay.event_losses(branch.id) {
            engine_allocation::reserve(&mut event_losses, 1)?;
            event_losses.push(RecordingEventLoss {
                after_hour,
                hour,
                reason: engine_allocation::string(EVENT_LOSS_REASON)?,
            });
        }
        branches.push(BranchMetadata {
            id: branch.id.to_string(),
            session_id: branch.session.to_string(),
            first_hour,
            last_hour,
            parent_id,
            parent_hour,
            sample_boundary: engine_allocation::string(session.sample_boundary.name())?,
            gaps,
            event_losses: (!event_losses.is_empty()).then_some(event_losses),
        });
    }
    Ok(branches)
}

#[derive(Default)]
pub(crate) struct Catalogue {
    rows: Vec<CountryMetadata>,
    bad_mapping: bool,
}

impl Catalogue {
    fn observe(
        &mut self,
        rows: &[CountryRow],
        links: &[CountryLinkRow],
        branch: BranchId,
        hour: i32,
    ) -> io::Result<()> {
        // The first complete country read supplies this branch's catalogue.
        // Index borrowed rows, then allocate just the admitted output countries.
        let mut first = HashMap::new();
        for row in rows {
            let Ok(slot) = mapped_slot(row, links) else {
                self.bad_mapping = true;
                continue;
            };
            if !first.contains_key(&slot) {
                engine_allocation::reserve_map(&mut first, 1)?;
                first.insert(slot, row);
            }
        }
        self.rows = engine_allocation::vec(first.len())?;
        for (slot, row) in first {
            let tag = String::from_utf8_lossy(&row.tag[..row.tag.len().min(512 + 3)]);
            self.rows.push(CountryMetadata {
                id: slot.to_string(),
                tag: short_text(&tag, 512),
                // Named by Source::name_countries once game content is known.
                name: short_text(&tag, 512),
                observed_branch_id: branch.to_string(),
                observed_hour: hour,
            });
        }
        Ok(())
    }

    pub(crate) fn finish(mut self, warnings: &mut Vec<String>) -> io::Result<Vec<CountryMetadata>> {
        if self.bad_mapping {
            warn(warnings, "Some country rows have missing or conflicting CountryLinks mappings and are excluded from the catalogue.".into());
        }
        // Canonical slot IDs are decimal without leading zeroes.
        self.rows
            .sort_unstable_by(|a, b| a.id.len().cmp(&b.id.len()).then_with(|| a.id.cmp(&b.id)));
        Ok(self.rows)
    }
}

fn merge_countries(
    merged: &mut Vec<CountryMetadata>,
    known: &mut HashSet<(String, String)>,
    countries: Vec<CountryMetadata>,
) -> io::Result<()> {
    for country in countries {
        engine_allocation::reserve_set(known, 1)?;
        if known.insert((country.id.clone(), country.tag.clone())) {
            engine_allocation::reserve(merged, 1)?;
            merged.push(country);
        }
    }
    Ok(())
}

pub fn warn(warnings: &mut Vec<String>, message: String) {
    let message = short_text(&message, 1024);
    if !warnings.contains(&message) && warnings.len() < MAX_WARNINGS {
        warnings.push(message);
    }
}

/// Bounded text for wire fields, cut at a character boundary.
pub fn short_text(text: &str, maximum: usize) -> String {
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
    use replay_data::{rows::LocalPlayerRow, rows::PlayerRow, rows::TableRows};

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
        assert_eq!(player_slot(&frame, &links).unwrap(), None);
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
            assert_eq!(player_slot(&frame, &links).unwrap(), expected);
        }
    }
}

/// The rows a table holds this hour, complete or truncated; an unavailable table
/// holds none. For products that use whatever rows were recorded.
pub fn recorded_rows<T: replay_data::rows::TableSchema>(
    frame: &replay_frame::frame::DecodedFrame,
) -> io::Result<&[T::Row]> {
    use replay_frame::frame::TableView;
    match &*frame.tables[T::ID.index()] {
        TableView::Complete(rows) | TableView::Truncated { rows, .. } => {
            rows.rows::<T>().ok_or_else(|| {
                crate::invalid(format!(
                    "hour {} {:?} contains another table's rows",
                    frame.hour,
                    T::ID
                ))
            })
        }
        TableView::Unavailable { .. } => Ok(&[]),
    }
}

#[cfg(test)]
mod tests;
