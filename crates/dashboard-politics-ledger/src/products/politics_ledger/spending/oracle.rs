//! Recording-scan parity oracle; compiled only by family tests.
use super::*;
use dashboard_base::products::politics_content::politics_content;
use dashboard_core::blocks::PerBlock;
use dashboard_text::{catalogue::Catalogue, metadata::validate_campaign};
use replay::container::Replay;
use replay_frame::frame::FrameDecoder;

#[derive(Default)]
struct Cache {
    branch: Option<u64>,
    catalogue: Option<Arc<Catalogue>>,
    scan: Option<Arc<Scanned>>,
}
/// The hours an item's text reads trigger operands at, and the reads whose
/// hour the next read dates a payment at: that payment's event frame is the
/// previous read. A block reads alike whatever tables are selected, so the
/// scan's previous read is this pass's.
fn event_hours(
    replay: &Replay,
    branch: u64,
    cancelled: &AtomicBool,
) -> io::Result<(BTreeSet<i32>, BTreeSet<i32>)> {
    let selected = [
        TableId::Campaign,
        TableId::CountryPowerChanges,
        TableId::CountryPowerDailyCosts,
    ];
    let mut hours = BTreeSet::new();
    let mut dated = BTreeSet::new();
    let mut previous = None;
    dashboard_core::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|block| block.branch == branch),
        &selected,
        cancelled,
        |blocks| {
            for (_, frames) in blocks {
                cancelled_request(cancelled)?;
                let Ok(frames) = frames else {
                    continue;
                };
                let mut decoder = FrameDecoder::for_tables(&selected);
                for frame in frames {
                    cancelled_request(cancelled)?;
                    let decoded = decoder.advance(&frame)?;
                    validate_campaign(decoded, replay)?;
                    for row in decoded
                        .complete::<tables::CountryPowerChanges>()
                        .unwrap_or_default()
                    {
                        if previous == Some(row.hour) {
                            dated.insert(row.hour);
                        }
                        if matches!(
                            row.cause,
                            PowerCause::Decision
                                | PowerCause::TargetedDecision
                                | PowerCause::DecisionTimeout
                                | PowerCause::Focus
                                | PowerCause::Idea
                                | PowerCause::Advisor
                                | PowerCause::AdvisorRemoval
                        ) {
                            hours.insert(row.hour);
                        }
                    }
                    for row in decoded
                        .complete::<tables::CountryPowerDailyCosts>()
                        .unwrap_or_default()
                    {
                        if previous == Some(row.day_hour) {
                            dated.insert(row.day_hour);
                        }
                        if row.identity.object_kind
                            == replay_data::rows::PowerObjectKind::Organisation
                        {
                            hours.insert(row.day_hour);
                        }
                    }
                    previous = Some(decoded.hour);
                }
            }
            Ok(())
        },
    )?;
    Ok((hours, dated))
}
fn scan_branch(
    replay: &Replay,
    branch: u64,
    content: Option<&PoliticsContent>,
    cancelled: &AtomicBool,
) -> io::Result<Scanned> {
    // First identify event hours from the ledger. Decode trigger operands only
    // in blocks that contain an exact event hour, including a preceding block
    // whose final hour is consumed by the next read's handoff. Only a read the
    // next read dates a payment at is retained whole for that handoff.
    let (needed, dated) = if content.is_some() {
        let (needed, dated) = event_hours(replay, branch, cancelled)?;
        (needed, Some(dated))
    } else {
        (BTreeSet::new(), None)
    };
    let mut selected = vec![
        TableId::Campaign,
        TableId::CountryPowerDaily,
        TableId::CountryPowerChanges,
        TableId::CountryPowerDailyCosts,
        TableId::GovernmentChanges,
        TableId::PoliticsCaptureCoverage,
        TableId::CharacterPortraits,
        TableId::AdvisorRoles,
        TableId::Characters,
        TableId::UnitLeaders,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::LexicalTokens,
        TableId::ModifierDefinitions,
        TableId::CountryAppearance,
        TableId::Politics,
        TableId::PoliticalParties,
        TableId::Autonomy,
        TableId::CountryCivilWars,
        TableId::CountryCounterInputs,
        TableId::CountryWars,
        TableId::WarRelations,
        TableId::CountryCommandPower,
        TableId::IndustrialOrganisations,
    ];
    selected.sort_unstable();
    selected.dedup();
    let mut with_world = selected.clone();
    with_world.extend_from_slice(dashboard_text::triggers::RECORDED_TABLES);
    with_world.sort_unstable();
    with_world.dedup();
    let mut scan = Scanned {
        first_hour: replay
            .index
            .iter()
            .filter(|block| block.branch == branch)
            .map(|block| block.first_hour)
            .min(),
        last_hour: replay
            .index
            .iter()
            .filter(|block| block.branch == branch)
            .map(|block| block.last_hour)
            .max(),
        dated,
        ..Default::default()
    };
    // Trigger operands only in blocks holding an exact event hour.
    let tables_of = |block: &replay::container::BlockIndex| -> &[TableId] {
        if needed
            .range(block.first_hour..=block.last_hour)
            .next()
            .is_some()
        {
            &with_world
        } else {
            &selected
        }
    };
    let blocks = replay.index.iter().filter(|block| block.branch == branch);
    dashboard_core::blocks::scan_blocks(
        replay,
        blocks,
        &PerBlock::new(tables_of),
        cancelled,
        |blocks| {
            for (block, frames) in blocks {
                cancelled_request(cancelled)?;
                let tables = tables_of(block);
                let frames = match frames {
                    Ok(frames) => frames,
                    Err(_) => {
                        scan.note(None, "A recording block could not be read".into(), 1)?;
                        continue;
                    }
                };
                let mut decoder = FrameDecoder::for_tables(tables);
                for frame in frames {
                    cancelled_request(cancelled)?;
                    let decoded = decoder.advance(&frame)?;
                    validate_campaign(decoded, replay)?;
                    scan.observe(decoded, content)?;
                }
            }
            Ok(())
        },
    )?;
    scan.previous = None;
    // Only observation reads its row indexes, and the cached rows stop growing.
    scan.events.clear();
    scan.daily.clear();
    scan.costs.clear();
    for country in scan.countries.values_mut() {
        country.mutations.shrink_to_fit();
        country.daily.shrink_to_fit();
        country.daily_costs.shrink_to_fit();
    }
    Ok(scan)
}

pub(in crate::products::politics_ledger) fn load(
    source: &Source,
    params: &PoliticsLedgerParams,
    context: &LoadContext,
    cancelled: &AtomicBool,
) -> io::Result<PoliticsLedgerReply> {
    let content = context
        .content
        .as_ref()
        .map(|options| politics_content(source, options))
        .transpose()?;
    let catalogue = content
        .as_ref()
        .map(|content| Arc::clone(&content.catalogue));
    let state = source.state::<Cache>();
    let mut state = state.lock().unwrap_or_else(|error| error.into_inner());
    let same = match (&state.catalogue, &catalogue) {
        (None, None) => true,
        (Some(a), Some(b)) => Arc::ptr_eq(a, b),
        _ => false,
    };
    if state.branch != Some(params.branch()) || !same || state.scan.is_none() {
        state.scan = Some(Arc::new(scan_branch(
            &source.replay(),
            params.branch(),
            content.as_deref(),
            cancelled,
        )?));
        state.branch = Some(params.branch());
        state.catalogue = catalogue;
    }
    let scan = Arc::clone(state.scan.as_ref().expect("scan loaded"));
    drop(state);
    present_view(
        &scan,
        params.branch(),
        &params.tag,
        content.as_deref(),
        cancelled,
        params.view.clone(),
    )
}
