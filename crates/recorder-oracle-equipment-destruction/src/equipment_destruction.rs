use super::{Report, complete, country_context, invalid, required_country_tag};
use content_reader_save::Save;
use replay::{campaign::Replay, campaign::SeekResult};
use replay_data::{
    rows::CountryRow, rows::DivisionDeletionDisposition, rows::EquipmentDestructionCause,
    rows::EquipmentDestructionCoverage, rows::Recorded, rows::RowCodec, rows::TableId,
    rows::tables,
};
use replay_frame::frame::DecodedFrame;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

pub fn compare(frame: &DecodedFrame, report: &mut Report) -> io::Result<()> {
    let rows = complete::<tables::EquipmentDestruction>(frame)?;
    let links: BTreeSet<_> = complete::<tables::CountryLinks>(frame)?
        .iter()
        .map(|r| r.id)
        .collect();
    let variants: BTreeSet<_> = complete::<tables::VariantDefinitions>(frame)?
        .iter()
        .map(|r| r.id)
        .collect();
    let mut unknown = 0;
    let mut native_gains = 0;
    let mut causes = BTreeMap::new();
    let mut amount_rows = 0;
    let mut signed_net = 0_i128;
    if rows
        .iter()
        .any(|row| row.coverage == Recorded::Value(EquipmentDestructionCoverage::PartialInterval))
    {
        if rows.len() != 1 {
            return Err(invalid(
                "EquipmentDestruction partial interval marker cannot appear with other rows",
            ));
        }
        rows[0].check_key_fields()?;
        let reason = rows[0]
            .unavailable_reason
            .require("EquipmentDestruction.unavailable_reason")
            .map_err(invalid)?;
        return Err(invalid(format!("EquipmentDestruction is unavailable: {reason}")));
    }
    for row in rows {
        row.check_key_fields()?;
        if row.coverage == Recorded::Value(EquipmentDestructionCoverage::SupplementalUnavailable) {
            let reason = row
                .unavailable_reason
                .require("EquipmentDestruction.unavailable_reason")
                .map_err(invalid)?;
            report.limitation("supplemental hooks are unavailable, so signed net removals omit supplemental causes in every equipment domain", format!("EquipmentDestruction.coverage: {reason}"));
            continue;
        }
        amount_rows += 1;
        if row.amount_raw == 0 {
            return Err(invalid("EquipmentDestruction contains a zero net removal"));
        }
        if !links.contains(&row.country) {
            return Err(invalid(format!(
                "EquipmentDestruction owner {} has no CountryLink",
                row.country
            )));
        }
        if row.variant as u32 != engine_native::cequipment_variant::REFERENCE_TYPE {
            return Err(invalid(
                "EquipmentDestruction invalid design reference type",
            ));
        }
        if !variants.contains(&row.variant) {
            return Err(invalid(format!(
                "EquipmentDestruction design {:#x} has no VariantDefinition in this frame",
                row.variant
            )));
        }
        unknown += usize::from(row.cause == EquipmentDestructionCause::Unknown);
        native_gains += usize::from(row.amount_raw < 0);
        signed_net = signed_net
            .checked_add(i128::from(row.amount_raw))
            .ok_or_else(|| invalid("EquipmentDestruction signed amount sum overflows"))?;
        *causes.entry(row.cause.as_str()).or_insert(0_usize) += 1;
    }
    report.compared("EquipmentDestruction", amount_rows);
    report.limitation("Saves serialize remaining pools rather than destruction events, so signed event amounts have only a recording self-check", format!("EquipmentDestruction.amount_raw: {amount_rows} aggregate rows; signed net {signed_net} raw"));
    report.limitation("deletion witnesses certify identity and absence without assigning individual divisions a share of aggregate losses", format!("EquipmentDestruction.cause: {amount_rows} aggregate rows"));
    for (cause, count) in causes {
        report.limitation("the save has no destruction-event field for this cause, so its signed net removals have no exact save amount comparator", format!("EquipmentDestruction.{cause}: {count} aggregate rows"));
    }
    if native_gains != 0 {
        report.limitation("native pool gains are recorded as negative net removals, so these rows correct earlier totals rather than counting losses", format!("EquipmentDestruction.amount_raw: {native_gains} negative aggregate rows are native gain corrections"));
    }
    if unknown != 0 {
        report.limitation("an unclassified native gameplay caller records explicit unknown; its amount is counted without guessing the cause", format!("EquipmentDestruction.cause: {unknown} unknown aggregate rows"));
    }
    Ok(())
}

pub fn compare_deletions(
    frame: &DecodedFrame,
    saved: &Save,
    report: &mut Report,
) -> io::Result<()> {
    let rows = complete::<tables::DivisionEquipmentDeletions>(frame)?;
    let countries = country_context(frame)?;
    let live_handles = match complete::<tables::Divisions>(frame) {
        Ok(rows) => Some(rows.iter().map(|row| row.id).collect::<BTreeSet<_>>()),
        Err(error) => {
            if !rows.is_empty() {
                report.limitation("the current hour's Divisions table is incomplete; deletion absence from the live census cannot be checked or inferred", format!("DivisionEquipmentDeletions: {} witnesses; {error}", rows.len()));
            }
            None
        }
    };
    let mut seen = BTreeSet::new();
    let mut unknown = 0;
    for row in rows {
        if row.division_handle >> 32 == 0 || !seen.insert(row.division_handle) {
            return Err(invalid(
                "DivisionEquipmentDeletions has a null, inconsistent or duplicate division identity",
            ));
        }
        if !row.valid_disposition() {
            return Err(invalid(
                "DivisionEquipmentDeletions contains an invalid disposition",
            ));
        }
        let tag = required_country_tag(row.country, &countries.links)?;
        if (row.expeditionary_owner as i32) > 0 {
            required_country_tag(row.expeditionary_owner, &countries.links)?;
        }
        if !saved.countries.contains_key(tag) {
            report.difference(
                format!(
                    "DivisionEquipmentDeletions {:#x}.country",
                    row.division_handle
                ),
                String::from_utf8_lossy(tag),
                "country absent from save",
            );
        }
        if !row.disposition.is_temporary_transfer()
            && saved.divisions.contains_key(&row.division_handle)
        {
            report.difference(
                format!(
                    "DivisionEquipmentDeletions {:#x}.saved_unit",
                    row.division_handle
                ),
                "deleted during the recorded interval",
                "still present in the save",
            );
        }
        if !row.disposition.is_temporary_transfer()
            && live_handles
                .as_ref()
                .is_some_and(|handles| handles.contains(&row.division_handle))
        {
            report.difference(
                format!(
                    "DivisionEquipmentDeletions {:#x}.current_census",
                    row.division_handle
                ),
                "deleted during the recorded interval",
                "still present in the current Divisions census",
            );
        }
        unknown += usize::from(row.cause == Some(EquipmentDestructionCause::Unknown));
    }
    report.compared("DivisionEquipmentDeletions", rows.len());
    if live_handles.is_some() {
        report.compared("DivisionEquipmentDeletions.current_census", rows.len());
    }
    if !rows.is_empty() {
        report.limitation("deleted units lack saved pools and owners, while roster-only saves cannot certify transfer-held divisions or event dispositions", format!("DivisionEquipmentDeletions: {} deletion or roster-transfer witnesses", rows.len()));
    }
    if unknown != 0 {
        report.limitation("an unclassified native gameplay deletion caller records explicit unknown rather than an inferred reason", format!("DivisionEquipmentDeletions.cause: {unknown} unknown deletion witnesses"));
    }
    Ok(())
}

#[derive(Default)]
struct DivisionHistory {
    ever_censused: BTreeSet<u64>,
    transfers: BTreeMap<u64, DivisionDeletionDisposition>,
    incomplete_witness_intervals: usize,
    incomplete_censuses: usize,
    unrecorded_hours: u64,
}

impl DivisionHistory {
    fn observe(&mut self, frame: &DecodedFrame) -> io::Result<()> {
        if let Ok(rows) = complete::<tables::DivisionEquipmentDeletions>(frame) {
            for row in rows {
                row.check_key_fields()?;
                if row.disposition.is_temporary_transfer() {
                    self.transfers.insert(row.division_handle, row.disposition);
                } else {
                    self.transfers.remove(&row.division_handle);
                }
            }
        } else {
            self.incomplete_witness_intervals += 1;
        }
        if let Ok(rows) = complete::<tables::Divisions>(frame) {
            for row in rows {
                self.ever_censused.insert(row.id);
                self.transfers.remove(&row.id);
            }
        } else {
            self.incomplete_censuses += 1;
        }
        Ok(())
    }
}

fn division_history(replay: &Replay, frame: &DecodedFrame) -> io::Result<DivisionHistory> {
    let selected = [TableId::Divisions, TableId::DivisionEquipmentDeletions];
    let mut history = DivisionHistory::default();
    let mut next_hour = replay
        .branches
        .iter()
        .find(|branch| branch.id == frame.branch)
        .map(|branch| branch.first_hour)
        .ok_or_else(|| invalid("division deletion history has no branch declaration"))?;
    for block in replay
        .index
        .iter()
        .filter(|block| block.branch == frame.branch && block.first_hour < frame.hour)
    {
        replay.visit_blocks(
            &[block],
            &selected,
            |_, _, hour_frame| {
            if hour_frame.hour >= frame.hour {
                return Ok(false);
            }
            if hour_frame.hour > next_hour {
                history.unrecorded_hours += (i64::from(hour_frame.hour) - i64::from(next_hour)) as u64;
            }
            next_hour = hour_frame
                .hour
                .checked_add(1)
                .ok_or_else(|| invalid("division deletion history hour overflows"))?;
            history.observe(Ok::<&replay_frame::frame::DecodedFrame, std::io::Error>(hour_frame)?)?;
                Ok(true)
            },
            || Ok(()),
        )?;
    }
    if frame.hour > next_hour {
        history.unrecorded_hours += (i64::from(frame.hour) - i64::from(next_hour)) as u64;
    }
    Ok(history)
}

pub fn compare_deletion_history(
    replay: &Replay,
    frame: &DecodedFrame,
    saved: &Save,
    report: &mut Report,
) -> io::Result<()> {
    complete::<tables::DivisionEquipmentDeletions>(frame)?;
    if report.load_hour {
        report.limitation("load cleanup is not gameplay and has no deletion witnesses; no census-disappearance comparison crosses a load boundary", "DivisionEquipmentDeletions: load boundary");
        return Ok(());
    }
    let previous_hour = frame
        .hour
        .checked_sub(1)
        .ok_or_else(|| invalid("division deletion preceding hour overflows"))?;
    let history = division_history(replay, frame)?;
    match replay.seek(frame.branch, previous_hour)? {
        SeekResult::Frame(previous) => compare_previous_census_with_history(
            frame,
            Some(&previous),
            saved,
            "",
            &history,
            report,
        ),
        SeekResult::Gap { reason, .. } => {
            compare_previous_census_with_history(frame, None, saved, &reason, &history, report)
        }
    }
}

#[cfg(test)]
fn compare_previous_census(
    frame: &DecodedFrame,
    previous: Option<&DecodedFrame>,
    saved: &Save,
    unavailable: &str,
    report: &mut Report,
) -> io::Result<()> {
    let mut history = DivisionHistory::default();
    if let Some(previous) = previous {
        history.observe(previous)?;
    }
    compare_previous_census_with_history(frame, previous, saved, unavailable, &history, report)
}

fn compare_previous_census_with_history(
    frame: &DecodedFrame,
    previous: Option<&DecodedFrame>,
    saved: &Save,
    unavailable: &str,
    history: &DivisionHistory,
    report: &mut Report,
) -> io::Result<()> {
    let rows = complete::<tables::DivisionEquipmentDeletions>(frame)?;
    let Some(previous) = previous else {
        report.limitation("the preceding hour on the same branch is unavailable; no prior division census or native reference-type witness can be inferred", format!("DivisionEquipmentDeletions: {} witnesses; preceding frame: {unavailable}", rows.len()));
        return Ok(());
    };
    if previous.branch != frame.branch || previous.hour.checked_add(1) != Some(frame.hour) {
        return Err(invalid(
            "division deletion census is not the preceding branch hour",
        ));
    }
    let census = match complete::<tables::Divisions>(previous) {
        Ok(rows) => rows,
        Err(error) => {
            report.limitation("the preceding hour's Divisions table is incomplete; no prior division census or native reference-type witness can be inferred", format!("DivisionEquipmentDeletions: {} witnesses; {error}", rows.len()));
            return Ok(());
        }
    };
    let handles: BTreeMap<_, _> = census.iter().map(|row| (row.id, row)).collect();
    let witnessed = rows
        .iter()
        .filter(|row| handles.contains_key(&row.division_handle))
        .count();
    report.compared("DivisionEquipmentDeletions.previous_census", witnessed);
    let mut unseen = BTreeMap::new();
    let mut transferred_before_census = BTreeMap::new();
    for row in rows {
        if !handles.contains_key(&row.division_handle) {
            if history.transfers.contains_key(&row.division_handle) {
                *transferred_before_census
                    .entry(row.event_name())
                    .or_insert(0_usize) += 1;
            } else if history.ever_censused.contains(&row.division_handle)
                && matches!(
                    row.cause,
                    Some(
                        EquipmentDestructionCause::DivisionDestroyed
                            | EquipmentDestructionCause::DivisionOverrun
                    )
                )
            {
                report.difference(
                    format!(
                        "DivisionEquipmentDeletions {:#x}.previous_census",
                        row.division_handle
                    ),
                    "combat wipe or overrun of a division seen in an earlier complete census",
                    "absent from the preceding complete Divisions census",
                );
            } else {
                *unseen.entry(row.event_name()).or_insert(0_usize) += 1;
            }
        }
    }
    for (disposition, count) in transferred_before_census {
        report.limitation("a witnessed transfer may return and be deleted before a roster census, or be deleted as a transfer-held orphan", format!("DivisionEquipmentDeletions.{disposition}: {count} transfer-held deletion witnesses absent from the preceding census"));
    }
    for (disposition, count) in unseen {
        let reason = if matches!(disposition, "division_destroyed" | "division_overrun") {
            "a division may be wiped or overrun before its first complete census, so its prior identity is unverified"
        } else if matches!(disposition, "division_scripted" | "unknown") {
            "a scripted or unclassified gameplay deletion may discard a never-censused division or orphaned transfer-held division"
        } else {
            "an event between censuses or orphaned transfer-held cleanup has no preceding census identity witness"
        };
        report.limitation(reason, format!("DivisionEquipmentDeletions.{disposition}: {count} deletion witnesses not previously censused"));
    }

    let countries = country_context(frame)?;
    let previous_countries = country_context(previous)?;
    let mut changed_expeditionary = 0;
    for row in rows {
        let Some(prior) = handles.get(&row.division_handle) else {
            continue;
        };
        let prior_expeditionary = ((prior.expeditionary_owner as i32) > 0)
            .then(|| country_identity(prior.expeditionary_owner, &previous_countries.links))
            .transpose()?;
        let captured_expeditionary = ((row.expeditionary_owner as i32) > 0)
            .then(|| country_identity(row.expeditionary_owner, &countries.links))
            .transpose()?;
        if prior_expeditionary != captured_expeditionary {
            changed_expeditionary += 1;
            continue;
        }
        let prior_owner = prior_expeditionary
            .map(Ok)
            .unwrap_or_else(|| country_identity(prior.roster_owner, &previous_countries.links))?;
        let owner = country_identity(row.country, &countries.links)?;
        if prior_owner != owner {
            report.difference(
                format!(
                    "DivisionEquipmentDeletions {:#x}.previous_owner",
                    row.division_handle
                ),
                format!("{}:{}", prior_owner.0, prior_owner.1),
                format!("{}:{}", owner.0, owner.1),
            );
        }
        report.compared("DivisionEquipmentDeletions.previous_owner", 1);
    }
    if changed_expeditionary != 0 {
        report.limitation("expeditionary ownership changed since the preceding census; the earlier owner cannot independently certify the deletion owner", format!("DivisionEquipmentDeletions: {changed_expeditionary} witnesses with changed expeditionary status"));
    }

    let current = match complete::<tables::Divisions>(frame) {
        Ok(census) => census.iter().map(|row| row.id).collect::<BTreeSet<_>>(),
        Err(error) => {
            report.limitation("the current hour's Divisions table is incomplete; unwitnessed disappearances cannot be checked", format!("DivisionEquipmentDeletions: preceding census {} divisions; {error}", census.len()));
            return Ok(());
        }
    };
    let witnesses: BTreeSet<_> = rows.iter().map(|row| row.division_handle).collect();
    let mut disappeared = 0;
    let mut fake = 0;
    for prior in census {
        if prior.reveal_skip != 0 {
            fake += 1;
            continue;
        }
        if !current.contains(&prior.id) && !saved.divisions.contains_key(&prior.id) {
            disappeared += 1;
            if !witnesses.contains(&prior.id) {
                report.difference(
                    format!("DivisionEquipmentDeletions {:#x}.disappearance", prior.id),
                    "a gameplay deletion witness",
                    "absent from current complete census and exact-hour save without a witness",
                );
            }
        }
    }
    report.compared("DivisionEquipmentDeletions.disappearances", disappeared);
    let mut active = history.transfers.clone();
    for row in rows {
        if row.disposition.is_temporary_transfer() {
            active.insert(row.division_handle, row.disposition);
        } else {
            active.remove(&row.division_handle);
        }
    }
    active.retain(|handle, _| !current.contains(handle) && !saved.divisions.contains_key(handle));
    let mut held = BTreeMap::new();
    for disposition in active.values() {
        *held.entry(disposition.as_str()).or_insert(0_usize) += 1;
    }
    for (disposition, count) in held {
        report.limitation("a witnessed volunteer/exile transfer holds a live division outside rosters until reappearance or witnessed deletion", format!("DivisionEquipmentDeletions.{disposition}: {count} transfer-held divisions"));
    }
    if !active.is_empty()
        && (history.incomplete_witness_intervals != 0
            || history.incomplete_censuses != 0
            || history.unrecorded_hours != 0)
    {
        report.limitation("unavailable witness intervals and incomplete census history cannot certify intervening deletions or reappearance", format!("DivisionEquipmentDeletions: {} active transfers; {} incomplete witness intervals; {} incomplete censuses; {} unrecorded hours", active.len(), history.incomplete_witness_intervals, history.incomplete_censuses, history.unrecorded_hours));
    }
    if fake != 0 {
        report.limitation("fake-intel census entries are deception objects whose lifecycle has no gameplay loss witness", format!("DivisionEquipmentDeletions: {fake} fake-intel census entries excluded"));
    }
    Ok(())
}

fn country_identity(reference: u32, links: &BTreeMap<u32, &CountryRow>) -> io::Result<(u32, u32)> {
    required_country_tag(reference, links)?;
    let country = links[&reference];
    Ok((country.canonical_slot, country.id))
}

#[cfg(test)]
mod tests;
