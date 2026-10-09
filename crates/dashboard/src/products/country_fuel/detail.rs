//! Bounded lazy receipt windows. One visible window is shared by all countries
//! and recipient selections; the branch summary keeps no recipient payloads.
use super::*;

/// Matches the existing production-detail window duration; not a roster cap.
pub(super) const DETAIL_HOURS: i32 = super::super::production_lines::DETAIL_HOURS;

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

type Reads = BTreeMap<i32, BTreeMap<String, CountryFuelRow>>;
#[derive(Default)]
struct DetailCache {
    window: Option<((u64, i32), Arc<Reads>)>,
    // A pinned selection may be outside the currently visible history window.
    basis: Option<((u64, i32), Arc<Reads>)>,
}

/// Decode each necessary block once, retaining only actual requested reads.
/// All country's rows share the same decode; no country-proportional rescans.
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
    ];
    let replay = source.replay_ref();
    let mut reads = Reads::new();
    for block in replay.index.iter().filter(|block| {
        block.branch == branch
            && hours
                .range(block.first_hour..=block.last_hour)
                .next()
                .is_some()
    }) {
        crate::metadata::check_cancel(cancelled)?;
        let mut decoder = FrameDecoder::for_tables(&selected);
        let Ok(frames) = replay.read_block_selected(block, &selected) else {
            continue;
        };
        for frame in frames {
            crate::metadata::check_cancel(cancelled)?;
            let Ok(decoded) = decoder.advance(&frame) else {
                break;
            };
            if validate_campaign(decoded, replay).is_err() {
                break;
            }
            if !hours.contains(&frame.hour)
                || !slot_read(&decoded.metrics, TableId::CountryFuel, frame.hour)
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
            let mut by_slot = BTreeMap::new();
            for row in rows {
                if let Some(slot) = link_slot(links, row.country) {
                    by_slot.entry(slot).or_insert(row);
                }
            }
            let mut tagged_rows = BTreeMap::new();
            for (tag, lookup) in tagged_countries(countries, links) {
                crate::metadata::check_cancel(cancelled)?;
                if let Ok(tag) = std::str::from_utf8(tag)
                    && let Some((_, slot)) = lookup.found()
                    && let Some(row) = by_slot.get(&slot)
                {
                    tagged_rows.insert(tag.to_owned(), replay::rows::RowCodec::try_clone(*row)?);
                }
            }
            reads.insert(frame.hour, tagged_rows);
        }
    }
    Ok(reads)
}

pub(super) fn basis_reads(
    source: &Source,
    branch: u64,
    hour: i32,
    cancelled: &AtomicBool,
) -> io::Result<Arc<Reads>> {
    crate::metadata::check_cancel(cancelled)?;
    let state = source.state::<DetailCache>();
    let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(((cached_branch, _), reads)) = &cache.window
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

pub(super) fn authenticated<'a>(
    reads: &'a Reads,
    params: &FuelAwardsParams,
) -> io::Result<&'a CountryFuelRow> {
    let reference = params
        .country
        .parse::<u32>()
        .map_err(|_| crate::invalid("Fuel country reference invalid"))?;
    reads.get(&params.hour).and_then(|rows| rows.get(&params.tag))
        .filter(|row| row.country==reference
            && matches!(row.country_instance,replay::rows::Recorded::Value(Some(instance)) if instance==params.country_instance))
        .ok_or_else(|| crate::invalid("Fuel actor identity changed or supplying read is not recorded (carried display hours are not reads)"))
}

pub(super) fn window_reads(
    source: &Source,
    branch: u64,
    window: i32,
    summary: &ScannedBranch,
    cancelled: &AtomicBool,
) -> io::Result<Arc<Reads>> {
    crate::metadata::check_cancel(cancelled)?;
    let state = source.state::<DetailCache>();
    let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
    if let Some((key, reads)) = &cache.window
        && *key == (branch, window)
    {
        return Ok(Arc::clone(reads));
    }
    let (first, last) =
        super::super::production_lines::detail_window(window).expect("validated fuel window");
    let mut hours = BTreeSet::new();
    for country in summary.countries.values() {
        crate::metadata::check_cancel(cancelled)?;
        let start = country.reads.partition_point(|read| read.last_hour < first);
        for read in country.reads[start..]
            .iter()
            .take_while(|read| read.first_hour <= last)
        {
            hours.insert(read.first_hour);
        }
    }
    let reads = Arc::new(scan_reads(source, branch, &hours, cancelled)?);
    cache.window = Some(((branch, window), Arc::clone(&reads)));
    Ok(reads)
}

fn recipient_amount(
    row: &CountryFuelRow,
    hour: i32,
    key: &str,
    cancelled: &AtomicBool,
) -> io::Result<Result<String, String>> {
    if let Err(reason) = awards_of(row, hour) {
        return Ok(Err(reason));
    }
    let replay::rows::Recorded::Value(window) = &row.awards else {
        unreachable!()
    };
    let mut amount = 0_i128;
    for entry in &window.hours {
        for part in &entry.recipients {
            crate::metadata::check_cancel(cancelled)?;
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
                amount = amount
                    .checked_add(i128::from(part.amount_raw))
                    .ok_or_else(|| crate::invalid("Fuel recipient total exceeds exact range"))?;
            }
        }
    }
    // Absence is zero only after the actual complete 24-update census above.
    Ok(Ok(amount.to_string()))
}

fn history_runs(
    summary: &ScannedFuel,
    reads: &Reads,
    aliases: &[&String],
    params: &FuelAwardHistoryParams,
    continuity: i32,
    (first, last): (i32, i32),
    cancelled: &AtomicBool,
) -> io::Result<Vec<FuelAwardRun>> {
    let mut runs = Vec::new();
    let mut cursor = i64::from(first);
    let mut previous_known = false;
    let start = summary.reads.partition_point(|read| read.last_hour < first);
    for read in summary.reads[start..]
        .iter()
        .take_while(|read| read.first_hour <= last)
    {
        crate::metadata::check_cancel(cancelled)?;
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
        let value = if read.country != params.country
            || read.country_instance != Some(params.country_instance)
        {
            Err("Fuel read belongs to another or unrecorded native country identity".into())
        } else if read.continuity_start_hour != Some(continuity) {
            Err(
                "Fuel receipt observation belongs to another or unrecorded reset/gap segment"
                    .into(),
            )
        } else if read.day.is_none() {
            Err(read
                .awards_reason
                .clone()
                .unwrap_or_else(|| format!("Fuel day left out: {:?}", read.left_out)))
        } else if let Some(row) = reads.get(&read.first_hour).and_then(|rows| {
            let mut matching = aliases.iter().filter_map(|tag| rows.get(*tag)).filter(|row| row.country.to_string() == params.country
                && matches!(row.country_instance, replay::rows::Recorded::Value(Some(instance)) if instance == params.country_instance));
            let first = matching.next()?;
            matching.next().is_none().then_some(first)
        }) {
            recipient_amount(row, read.first_hour, &params.recipient_key, cancelled)?
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
        if super::super::production_lines::detail_window(params.window).is_none() {
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
        _context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let branch = params
            .branch_id
            .parse::<u64>()
            .map_err(|_| crate::invalid("Fuel branch invalid"))?;
        let basis_params = FuelAwardsParams {
            branch_id: params.branch_id.clone(),
            tag: params.tag.clone(),
            hour: params.hour,
            country: params.country.clone(),
            country_instance: params.country_instance,
        };
        let basis = basis_reads(source, branch, params.hour, cancelled)?;
        let row = authenticated(&basis, &basis_params)?;
        awards_of(row, params.hour).map_err(crate::invalid)?;
        let replay::rows::Recorded::Value(window) = &row.awards else {
            unreachable!()
        };
        let continuity = window
            .continuity_start_hour
            .ok_or_else(|| crate::invalid("Fuel receipt continuity boundary not recorded"))?;
        let parts = recipients_of(row, cancelled, |_| Ok(String::new()))?;
        if !parts.iter().any(|part| part.key == params.recipient_key) {
            return Err(crate::invalid(
                "Fuel recipient key is absent from the selected supplying read",
            ));
        }
        let summary = scanned(source, branch, cancelled)?;
        let aliases: Vec<_> = summary
            .actor_tags(&params.country, params.country_instance)
            .collect();
        let mut country = ScannedFuel {
            reads: aliases
                .iter()
                .flat_map(|tag| summary.countries[*tag].reads.iter())
                .filter(|read| {
                    read.country == params.country
                        && read.country_instance == Some(params.country_instance)
                })
                .cloned()
                .collect(),
            ..Default::default()
        };
        country.reads.sort_by_key(|read| read.first_hour);
        if country
            .reads
            .windows(2)
            .any(|reads| reads[0].last_hour >= reads[1].first_hour)
        {
            return Err(crate::invalid(
                "Conflicting aliases for the selected native fuel recipient",
            ));
        }
        let reads = window_reads(source, branch, params.window, &summary, cancelled)?;
        let (first_hour, last_hour) = super::super::production_lines::detail_window(params.window)
            .expect("validated fuel window");
        let runs = history_runs(
            &country,
            &reads,
            &aliases,
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
        if crate::transport::json_len(&reply, super::super::MAX_PRODUCT_REPLY_BYTES).is_none() {
            return Err(crate::invalid(
                "Fuel receipt history exceeds the product byte budget; this receipt cannot be shown",
            ));
        }
        Ok(reply)
    }
}

#[cfg(test)]
mod tests;
