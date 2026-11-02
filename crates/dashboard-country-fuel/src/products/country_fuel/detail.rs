//! A recipient's receipts over a window, from the applied days fuel's publication holds of
//! the supplying reads. The test fixtures read the same days from the recording's rows.
use super::*;

/// Matches the existing production-detail window duration; not a roster cap.
pub(super) const DETAIL_HOURS: i32 = dashboard_base::products::production_lines::DETAIL_HOURS;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FuelAwardHistoryParams {
    pub branch_id: String,
    pub tag: String,
    pub hour: i32,
    pub country: String,
    pub country_instance: u32,
    pub window: i32,
    pub recipient_key: String,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FuelAwardRun {
    /// Actual supplying read, including a whole read carried into the window.
    /// Unknown gap rows instead begin at the first missing hour.
    pub first_hour: i32,
    pub last_hour: i32,
    pub starts_segment: bool,
    pub amount_raw: Option<String>,
    pub reason: Option<String>,
}
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FuelAwardHistoryReply {
    pub branch_id: String,
    pub tag: String,
    pub hour: i32,
    pub country: String,
    pub country_instance: u32,
    pub continuity_start_hour: i32,
    pub window: i32,
    pub recipient_key: String,
    pub first_hour: i32,
    pub last_hour: i32,
    pub runs: Vec<FuelAwardRun>,
    pub warnings: Vec<String>,
}

#[cfg(any(test, feature = "test-support"))]
type Reads = BTreeMap<i32, BTreeMap<String, CountryFuelRow>>;
#[cfg(any(test, feature = "test-support"))]
#[derive(Default)]
struct DetailCache {
    /// A window's reads, and the hours scanned for them.
    window: Option<((u64, i32), BTreeSet<i32>, Arc<Reads>)>,
    // A pinned selection may be outside the currently visible history window.
    basis: Option<((u64, i32), Arc<Reads>)>,
}

/// Decode each necessary block once, retaining only actual requested reads.
/// All country's rows share the same decode; no country-proportional rescans.
#[cfg(any(test, feature = "test-support"))]
fn scan_reads(
    source: &Source,
    branch: u64,
    hours: &BTreeSet<i32>,
    cancelled: &AtomicBool,
) -> io::Result<Reads> {
    let selected = [
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::CountryFuel,
        TableId::FuelDemandParts,
    ];
    let replay = source.replay_ref();
    let mut reads = Reads::new();
    dashboard_core::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|block| {
            block.branch == branch
                && hours
                    .range(block.first_hour..=block.last_hour)
                    .next()
                    .is_some()
        }),
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
                    if !hours.contains(&frame.hour)
                        || !slot_read(
                            replay.format,
                            &decoded.metrics,
                            TableId::CountryFuel,
                            frame.hour,
                        )
                    {
                        continue;
                    }
                    let (Ok(countries), Ok(links), Ok(rows)) = (
                        decoded.complete::<tables::Countries>(),
                        decoded.complete::<tables::CountryLinks>(),
                        decoded.complete::<tables::CountryFuel>(),
                    ) else {
                        continue;
                    };
                    // Each pack with its lists, which later formats record apart.
                    let rows = replay_frame::rows::with_demand_parts(
                        rows,
                        &decoded.tables[TableId::FuelDemandParts.index()],
                        replay.format,
                    )?;
                    let mut by_slot = BTreeMap::new();
                    for row in &rows {
                        if let Some(slot) = link_slot(links, row.country) {
                            by_slot.entry(slot).or_insert(row);
                        }
                    }
                    let mut tagged_rows = BTreeMap::new();
                    for (tag, lookup) in tagged_countries(countries, links) {
                        dashboard_text::metadata::check_cancel(cancelled)?;
                        if let Ok(tag) = std::str::from_utf8(tag)
                            && let Some((_, slot)) = lookup.found()
                            && let Some(row) = by_slot.get(&slot)
                        {
                            tagged_rows.insert(
                                tag.to_owned(),
                                replay_data::rows::RowCodec::try_clone(*row)?,
                            );
                        }
                    }
                    reads.insert(frame.hour, tagged_rows);
                }
            }
            Ok(())
        },
    )?;
    Ok(reads)
}

#[cfg(any(test, feature = "test-support"))]
pub(super) fn basis_reads(
    source: &Source,
    branch: u64,
    hour: i32,
    cancelled: &AtomicBool,
) -> io::Result<Arc<Reads>> {
    dashboard_text::metadata::check_cancel(cancelled)?;
    let state = source.state::<DetailCache>();
    let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(((cached_branch, _), _, reads)) = &cache.window
        && *cached_branch == branch
        && reads.contains_key(&hour)
    {
        return Ok(Arc::clone(reads));
    }
    if let Some((key, reads)) = &cache.basis
        && *key == (branch, hour)
    {
        return Ok(Arc::clone(reads));
    }
    let reads = Arc::new(scan_reads(
        source,
        branch,
        &BTreeSet::from([hour]),
        cancelled,
    )?);
    cache.basis = Some(((branch, hour), Arc::clone(&reads)));
    Ok(reads)
}

#[cfg(any(test, feature = "test-support"))]
pub(super) fn authenticated<'a>(
    reads: &'a Reads,
    params: &FuelAwardsParams,
) -> io::Result<&'a CountryFuelRow> {
    let reference = params
        .country
        .parse::<u32>()
        .map_err(|_| dashboard_core::invalid("Fuel country reference invalid"))?;
    reads.get(&params.hour).and_then(|rows| rows.get(&params.tag))
        .filter(|row| row.country==reference
            && matches!(row.country_instance,replay_data::rows::Recorded::Value(Some(value)) if value==params.country_instance))
        .ok_or_else(|| dashboard_core::refused("Fuel actor identity changed or supplying read is not recorded (carried display hours are not reads)"))
}

/// The reads of window `window` at `hours`, its supplying reads a selection asks: one window is
/// kept, each hour of it scanned once whatever selection asked it first.
#[cfg(any(test, feature = "test-support"))]
pub(super) fn window_reads(
    source: &Source,
    branch: u64,
    window: i32,
    hours: &BTreeSet<i32>,
    cancelled: &AtomicBool,
) -> io::Result<Arc<Reads>> {
    dashboard_text::metadata::check_cancel(cancelled)?;
    let state = source.state::<DetailCache>();
    let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
    let (mut scanned, mut reads) = match cache.window.take() {
        Some((key, scanned, reads)) if key == (branch, window) => (scanned, reads),
        _ => (BTreeSet::new(), Arc::new(Reads::new())),
    };
    let missing: BTreeSet<i32> = hours.difference(&scanned).copied().collect();
    if !missing.is_empty() {
        let found = scan_reads(source, branch, &missing, cancelled)?;
        Arc::make_mut(&mut reads).extend(found);
        scanned.extend(missing);
    }
    cache.window = Some(((branch, window), scanned, Arc::clone(&reads)));
    Ok(reads)
}

fn recipient_amount(
    awards: &pack::Awards,
    key: &str,
    cancelled: &AtomicBool,
) -> io::Result<Result<String, String>> {
    let recipients = match awards {
        Ok((_, recipients)) => recipients,
        Err(reason) => return Ok(Err(reason.clone())),
    };
    let mut amount = 0_i128;
    for part in recipients {
        dashboard_text::metadata::check_cancel(cancelled)?;
        if part.kind > 2 {
            return Ok(Err(format!(
                "Fuel recipient kind {} unsupported",
                part.kind
            )));
        }
        if recipient_key(
            part.kind,
            part.reference,
            part.owner,
            part.host,
            part.name.as_deref(),
            &part.name_reason,
        )? == key
        {
            amount += i128::from(part.amount_raw);
        }
    }
    // Absence is zero only after the actual complete 24-update census above.
    Ok(Ok(amount.to_string()))
}

fn history_runs(
    supplying: &[(String, RawRead)],
    awards: &BTreeMap<(String, i32), pack::Awards>,
    params: &FuelAwardHistoryParams,
    continuity: i32,
    (first, last): (i32, i32),
    cancelled: &AtomicBool,
) -> io::Result<Vec<FuelAwardRun>> {
    let mut runs = Vec::new();
    let mut cursor = i64::from(first);
    let mut previous_known = false;
    let start = supplying.partition_point(|(_, read)| read.last_hour < first);
    for (tag, read) in supplying[start..]
        .iter()
        .take_while(|(_, read)| read.first_hour <= last)
    {
        dashboard_text::metadata::check_cancel(cancelled)?;
        if i64::from(read.first_hour) > cursor {
            runs.push(FuelAwardRun {
                first_hour: cursor as i32,
                last_hour: read.first_hour - 1,
                starts_segment: true,
                amount_raw: None,
                reason: Some("Fuel supplying read missing in this interval".into()),
            });
            previous_known = false;
        }
        let value = if read.country.to_string() != params.country {
            Err("Fuel read belongs to another or unrecorded native country identity".into())
        } else if read.continuity_start_hour != Some(continuity) {
            Err(
                "Fuel receipt observation belongs to another or unrecorded reset/gap segment"
                    .into(),
            )
        } else if read.day.is_none() {
            Err(read.awards_reason.as_deref().map_or_else(
                || format!("Fuel day left out: {:?}", read.left_out),
                str::to_owned,
            ))
        } else if let Some(day) = awards.get(&(tag.clone(), read.first_hour)) {
            recipient_amount(day, &params.recipient_key, cancelled)?
        } else {
            Err("Fuel supplying read unavailable in the detail window".into())
        };
        let known = value.is_ok();
        let (amount_raw, reason) = match value {
            Ok(amount) => (Some(amount), None),
            Err(reason) => (None, Some(reason)),
        };
        runs.push(FuelAwardRun {
            first_hour: read.first_hour,
            last_hour: read.last_hour,
            starts_segment: read.starts_segment || !previous_known,
            amount_raw,
            reason,
        });
        previous_known = known;
        cursor = i64::from(read.last_hour) + 1;
    }
    if cursor <= i64::from(last) {
        runs.push(FuelAwardRun {
            first_hour: cursor as i32,
            last_hour: last,
            starts_segment: true,
            amount_raw: None,
            reason: Some("Fuel supplying read missing in this interval".into()),
        });
    }
    Ok(runs)
}

pub struct CountryFuelAwardHistory;
impl Product for CountryFuelAwardHistory {
    const KEY: &'static str = "country-fuel-award-history";
    // Keys use receipt-time raw operands; numeric history does not read content.
    const CONTENT_DEPENDENT: bool = false;
    const PUBLISHED: bool = true;
    type Params = FuelAwardHistoryParams;
    type Reply = FuelAwardHistoryReply;
    fn types() -> Vec<String> {
        vec![
            FuelAwardHistoryParams::decl(),
            FuelAwardRun::decl(),
            FuelAwardHistoryReply::decl(),
        ]
    }
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        CountryFuelAwards::validate(&FuelAwardsParams {
            branch_id: params.branch_id.clone(),
            tag: params.tag.clone(),
            hour: params.hour,
            country: params.country.clone(),
            country_instance: params.country_instance,
        })?;
        if dashboard_base::products::production_lines::detail_window(params.window).is_none() {
            return Err("Invalid fuel detail window");
        }
        if params.recipient_key.is_empty() {
            return Err("Fuel recipient key is empty");
        }
        Ok(())
    }
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let branch = params
            .branch_id
            .parse::<u64>()
            .map_err(|_| dashboard_core::invalid("Fuel branch invalid"))?;
        let basis_params = FuelAwardsParams {
            branch_id: params.branch_id.clone(),
            tag: params.tag.clone(),
            hour: params.hour,
            country: params.country.clone(),
            country_instance: params.country_instance,
        };
        let (read, awards) = query::basis_awards(source, &basis_params, context, cancelled)?;
        let (_, recipients) = awards.map_err(dashboard_core::refused)?;
        let continuity = read.continuity_start_hour.ok_or_else(|| {
            dashboard_core::refused("Fuel receipt continuity boundary not recorded")
        })?;
        let parts = recipients_of(&recipients, cancelled, |_| Ok(String::new()))?;
        if !parts.iter().any(|part| part.key == params.recipient_key) {
            return Err(dashboard_core::invalid(
                "Fuel recipient key is absent from the selected supplying read",
            ));
        }
        let actor = query::actor_window(
            source,
            context,
            branch,
            &params.country,
            params.window,
            cancelled,
        )?;
        let mut supplying: Vec<(String, RawRead)> = actor
            .reads
            .into_iter()
            .filter(|(_, read)| read.country.to_string() == params.country)
            .collect();
        supplying.sort_by_key(|(_, read)| read.first_hour);
        if actor.conflict
            || supplying
                .windows(2)
                .any(|reads| reads[0].1.last_hour >= reads[1].1.first_hour)
        {
            return Err(dashboard_core::invalid(
                "Conflicting aliases for the selected native fuel recipient",
            ));
        }
        let (first_hour, last_hour) =
            dashboard_base::products::production_lines::detail_window(params.window)
                .expect("validated fuel window");
        let within: Vec<(String, i32)> = supplying
            .iter()
            .filter(|(_, read)| read.last_hour >= first_hour && read.first_hour <= last_hour)
            .map(|(tag, read)| (tag.clone(), read.first_hour))
            .collect();
        let awards = query::window_awards(
            source,
            &basis_params,
            params.window,
            context,
            &within,
            cancelled,
        )?;
        let runs = history_runs(
            &supplying,
            &awards,
            params,
            continuity,
            (first_hour, last_hour),
            cancelled,
        )?;
        let mut refusals = BTreeMap::new();
        for run in &runs {
            if let Some(reason) = &run.reason {
                *refusals.entry(reason).or_insert(0_usize) += 1;
            }
        }
        let warnings = refusals
            .into_iter()
            .map(|(reason, count)| format!("{count} fuel receipt intervals refused: {reason}"))
            .collect();
        let reply = FuelAwardHistoryReply {
            branch_id: params.branch_id.clone(),
            tag: params.tag.clone(),
            hour: params.hour,
            country: params.country.clone(),
            country_instance: params.country_instance,
            continuity_start_hour: continuity,
            window: params.window,
            recipient_key: params.recipient_key.clone(),
            first_hour,
            last_hour,
            runs,
            warnings,
        };
        dashboard_base::products::check_reply_budget(&reply).map_err(|_| {
            dashboard_core::invalid(
                "Fuel receipt history exceeds the product byte budget; this receipt cannot be shown",
            )
        })?;
        Ok(reply)
    }
}

#[cfg(test)]
mod tests;
