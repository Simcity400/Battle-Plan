//! Recording scan retained only as the loss parity oracle.
use super::*;

pub(super) const SELECTED: [TableId; 19] = [
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::Players,
    TableId::DivisionCensus,
    TableId::SimulationFeatures,
    TableId::CombatLogEquipment,
    TableId::CombatLogState,
    TableId::SliceReads,
    TableId::SunkShips,
    TableId::SunkConvoys,
    TableId::LendLeaseEquipment,
    TableId::VariantDefinitions,
    TableId::VariantChanges,
    TableId::LexicalTokens,
    TableId::CountryAppearance,
    TableId::VariantModules,
    TableId::VariantUpgrades,
    TableId::EquipmentDestruction,
];

pub(super) fn selected(land_source: LandSource) -> Vec<TableId> {
    SELECTED
        .into_iter()
        .filter(|table| {
            land_source == LandSource::CombatLog
                || !matches!(
                    table,
                    TableId::Players
                        | TableId::DivisionCensus
                        | TableId::SimulationFeatures
                        | TableId::CombatLogEquipment
                        | TableId::CombatLogState
                        | TableId::SliceReads
                )
        })
        .collect()
}

/// Scans one branch for one country's losses.
pub fn scan(
    replay: &Replay,
    branch: u64,
    tag: &str,
    land_source: LandSource,
    country: Option<LossCountry>,
    cancelled: &AtomicBool,
) -> io::Result<Scanned> {
    let cancel = || io::Error::new(io::ErrorKind::Interrupted, "Equipment losses cancelled");
    let mut scanned = Scanned {
        format: replay.format,
        ..Scanned::default()
    };
    let mut state = State {
        land_source,
        country,
        destruction_coverage_not_recorded: replay
            .format
            .field_gaps(TableId::EquipmentDestruction)
            .next()
            .is_some(),
        ..State::default()
    };
    let selected = selected(land_source);
    let start = replay
        .branches
        .iter()
        .find(|start| start.id == branch)
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "Equipment losses branch not found")
        })?;
    let (first, last) = branch_bounds(replay, start);
    let mut next = i64::from(first);
    let mut blocks: Vec<_> = replay
        .index
        .iter()
        .filter(|block| block.branch == branch)
        .collect();
    blocks.sort_unstable_by_key(|block| block.first_hour);
    let mut gaps: Vec<_> = replay
        .gaps
        .iter()
        .filter(|gap| gap.branch == branch)
        .collect();
    gaps.sort_unstable_by_key(|gap| gap.first_hour);
    let mut gap = 0;
    dashboard_core::blocks::scan_blocks(replay, blocks, &selected, cancelled, |blocks| {
        for (_, frames) in blocks {
            if cancelled.load(Ordering::Acquire) {
                return Err(cancel());
            }
            let mut decoder = FrameDecoder::for_tables(&selected);
            let Ok(frames) = frames else {
                continue;
            };
            for frame in frames {
                if cancelled.load(Ordering::Acquire) {
                    return Err(cancel());
                }
                let Ok(decoded) = decoder.advance(&frame) else {
                    break;
                };
                if validate_campaign(decoded, replay).is_err() {
                    break;
                }
                // Failed blocks/frames leave the cursor here. The next readable
                // hour (or branch end) marks the entire missing interval once.
                if i64::from(decoded.hour) < next {
                    continue;
                }
                while gap < gaps.len() && gaps[gap].last_hour < decoded.hour {
                    gap += 1;
                }
                if gaps
                    .get(gap)
                    .is_some_and(|gap| gap.first_hour <= decoded.hour)
                {
                    continue;
                }
                if next < i64::from(decoded.hour) {
                    scanned.missing(next as i32, decoded.hour - 1, land_source);
                }
                observe(&mut scanned, &mut state, decoded, tag)?;
                next = i64::from(decoded.hour) + 1;
            }
        }
        Ok(())
    })?;
    if next <= i64::from(last) {
        scanned.missing(next as i32, last, land_source);
    }
    Ok(scanned)
}

/// Native identity comes from the origin read, never from a later holder of its tag.
pub(super) fn selected_country<'a>(
    countries: &'a [replay_data::rows::CountryRow],
    links: &[CountryLinkRow],
    tag: &str,
    selected: Option<LossCountry>,
) -> TaggedCountry<'a> {
    let Some(selected) = selected else {
        return tagged_country(countries, links, tag);
    };
    let mut candidates = countries
        .iter()
        .filter(|row| row.canonical_slot == selected.slot && row.id == selected.country_id);
    let Some(row) = candidates.next() else {
        return TaggedCountry::Absent;
    };
    if candidates.next().is_some() || mapped_slot(row, links).ok() != Some(selected.slot) {
        return TaggedCountry::Unresolved;
    }
    TaggedCountry::Found(row, selected.slot)
}

/// Adds one recorded hour.
pub(super) fn observe(
    scanned: &mut Scanned,
    state: &mut State,
    decoded: &DecodedFrame,
    tag: &str,
) -> io::Result<()> {
    scanned.variants.observe(scanned.format, decoded);
    let links = decoded.complete::<tables::CountryLinks>().ok();
    let country = decoded
        .complete::<tables::Countries>()
        .ok()
        .zip(links)
        .map(|(countries, links)| selected_country(countries, links, tag, state.country))
        .unwrap_or(TaggedCountry::Unresolved);
    let slot = country.found().map(|(_, slot)| slot);
    let membership = (state.land_source == LandSource::CombatLog)
        .then(|| state.division_census.advance(decoded, slot));
    observe_country(scanned, state, decoded, links, country, membership)
}

/// One country's existing scan step; preparation resolves the frame's native
/// census once and shares variant observations across all countries.
fn observe_country(
    scanned: &mut Scanned,
    state: &mut State,
    decoded: &DecodedFrame,
    links: Option<&[CountryLinkRow]>,
    country: TaggedCountry<'_>,
    membership: Option<Option<census::Membership>>,
) -> io::Result<()> {
    let inputs = if country == TaggedCountry::Absent || state.destruction_withheld.is_some() {
        DestructionInputs::default()
    } else {
        destruction_inputs(decoded, links, scanned.format)?
    };
    let land_coverage = membership.and_then(|membership| {
        country
            .found()
            .zip(links)
            .map(|((_, slot), links)| coverage(decoded, links, slot, membership))
    });
    country_metadata(
        scanned,
        state,
        decoded,
        links,
        country,
        membership.is_some(),
        land_coverage,
        &inputs,
    )?;
    let hour = decoded.hour;
    let (Some(slot), Some(links)) = (country.found().map(|(_, slot)| slot), links) else {
        return Ok(());
    };
    let mut unresolved = BTreeSet::new();
    let mut ours = |raw: u32| match link_slot(links, raw) {
        Some(owner) => owner == slot,
        None => {
            unresolved.insert(raw);
            false
        }
    };
    if membership.is_some() {
        combat_log(scanned, state, decoded, links, slot)?;
    }
    let ships = decoded.complete::<tables::SunkShips>();
    if let Ok([ships]) = ships {
        ship_rows(
            scanned,
            state,
            ships
                .entries
                .iter()
                .filter(|entry| ours(entry.victim_country)),
        );
    }
    let convoys = decoded.complete::<tables::SunkConvoys>();
    if slot_read(scanned.format, &decoded.metrics, TableId::SunkConvoys, hour)
        && let Ok(rows) = convoys
    {
        convoy_rows(
            scanned,
            state,
            hour,
            rows.iter().filter(|row| ours(row.owner_country)),
        );
    }
    let lend_lease = decoded.complete::<tables::LendLeaseEquipment>();
    if slot_read(
        scanned.format,
        &decoded.metrics,
        TableId::LendLeaseEquipment,
        hour,
    ) && let Ok(rows) = lend_lease
    {
        let given: Vec<_> = rows.iter().filter(|row| ours(row.giver)).collect();
        lend_lease_rows(scanned, state, hour, given.into_iter());
    }
    scanned.unresolved.extend(unresolved);
    Ok(())
}

fn combat_log(
    scanned: &mut Scanned,
    state: &mut State,
    decoded: &DecodedFrame,
    links: &[CountryLinkRow],
    slot: u32,
) -> io::Result<()> {
    let Ok(reads) = decoded.complete::<tables::SliceReads>() else {
        return Ok(());
    };
    let hour = decoded.hour;
    let table = TableId::CombatLogEquipment.index() as u32;
    let first_read = reads.partition_point(|read| read.table < table);
    let last_read = reads.partition_point(|read| read.table <= table);
    let countries = links
        .iter()
        .filter(|link| link.canonical_slot == slot)
        .map(|link| link.id)
        .filter(|&raw| {
            replay_data::sliced::read_hour(reads, TableId::CombatLogEquipment, raw) == Some(hour)
        });
    let unresolved = &reads[first_read..last_read];
    let unresolved = unresolved
        .iter()
        .filter(|read| read.sample_hour == hour && link_slot(links, read.owner).is_none());
    if !countries.clone().any(|_| true) && !unresolved.clone().any(|_| true) {
        return Ok(());
    }
    let rows = match &*decoded.tables[TableId::CombatLogEquipment.index()] {
        replay_frame::frame::TableView::Complete(rows)
        | replay_frame::frame::TableView::Truncated { rows, .. } => {
            rows.rows::<tables::CombatLogEquipment>().unwrap_or(&[])
        }
        replay_frame::frame::TableView::Unavailable { .. } => return Ok(()),
    };
    for read in unresolved {
        let first = rows.partition_point(|row| row.country < read.owner);
        if rows.get(first).is_some_and(|row| row.country == read.owner) {
            if state.include_all_destruction {
                scanned.combat_unresolved.insert(read.owner);
            } else {
                scanned.unresolved.insert(read.owner);
            }
        }
    }
    // Decoding validates strictly ascending keys, including after delta/slice
    // merges. Country is the key's high prefix (replay_data::rows), so only visit
    // the selected country's ranges, never the observer's entire merged log.
    for country in countries {
        let first = rows.partition_point(|row| row.country < country);
        let last = rows.partition_point(|row| row.country <= country);
        combat_rows(scanned, state, hour, rows[first..last].iter());
    }
    Ok(())
}
