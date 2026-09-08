//! D20 output storage is funded by the same acquisition quota.
use super::*;

type Failure = DivisionSampleFailure;

pub(super) fn funded_vec<T>(
    count: usize,
    budget: &country_roster_readset::Budget,
) -> Result<Vec<T>, Failure> {
    let layout =
        std::alloc::Layout::array::<T>(count).map_err(|_| Failure::OutputStorageUnavailable)?;
    budget
        .retain(layout.size())
        .map_err(|_| Failure::OutputStorageUnavailable)?;
    if layout.size() == 0 {
        return Ok(Vec::new());
    }
    // The requested Layout is funded before allocation. Using that exact
    // layout/capacity also prevents allocator-provided excess capacity from
    // becoming an unaccounted retained country-word buffer.
    let pointer = unsafe { std::alloc::alloc(layout) }.cast::<T>();
    if pointer.is_null() {
        return Err(Failure::OutputStorageUnavailable);
    }
    // The allocation uses Vec's global allocator and exact T alignment; len
    // stays zero until initialized elements are pushed by the owning caller.
    Ok(unsafe { Vec::from_raw_parts(pointer, 0, count) })
}

fn clone_slice<T: Copy>(
    value: &[T],
    budget: &country_roster_readset::Budget,
) -> Result<Vec<T>, Failure> {
    let mut result = funded_vec(value.len(), budget)?;
    result.extend_from_slice(value);
    Ok(result)
}

fn clone_string(value: &str, budget: &country_roster_readset::Budget) -> Result<String, Failure> {
    let bytes = clone_slice(value.as_bytes(), budget)?;
    // A byte-for-byte copy of a str preserves its UTF-8 invariant.
    Ok(unsafe { String::from_utf8_unchecked(bytes) })
}

fn clone_preference(
    value: &compact_contract::division_routes::TemplateModelPreference,
    budget: &country_roster_readset::Budget,
) -> Result<compact_contract::division_routes::TemplateModelPreference, Failure> {
    Ok(compact_contract::division_routes::TemplateModelPreference {
        template_id: value.template_id,
        entity: clone_string(&value.entity, budget)?,
        category: value.category,
        excluded_variants: value
            .excluded_variants
            .as_deref()
            .map(|value| clone_slice(value, budget))
            .transpose()?,
    })
}

fn clone_acclimatization(
    value: &compact_contract::division_routes::Acclimatization,
    budget: &country_roster_readset::Budget,
) -> Result<compact_contract::division_routes::Acclimatization, Failure> {
    use compact_contract::division_routes::{Acclimatization, AcclimatizationCategory};
    let mut categories = funded_vec(value.categories.len(), budget)?;
    for entry in &value.categories {
        categories.push(AcclimatizationCategory {
            category: clone_string(&entry.category, budget)?,
            weight_raw: entry.weight_raw,
        });
    }
    Ok(Acclimatization {
        maximum_raw: value.maximum_raw,
        categories,
    })
}

fn clone_membership(
    value: &compact_contract::division_routes::FirstCombatCountriesObservation,
    budget: &country_roster_readset::Budget,
) -> Result<compact_contract::division_routes::FirstCombatCountriesObservation, Failure> {
    use compact_contract::division_routes::{
        CountryWordsObservation as Words, FirstCombatCountriesObservation as Observation,
    };
    Ok(match value {
        Observation::NoFirstCombat => Observation::NoFirstCombat,
        Observation::Unavailable(reason) => Observation::Unavailable(*reason),
        Observation::Present { combat, countries } => Observation::Present {
            combat: *combat,
            countries: match countries {
                Words::Measured(words) => Words::Measured(clone_slice(words, budget)?),
                Words::Unavailable(reason) => Words::Unavailable(*reason),
            },
        },
    })
}

// This explicit copy keeps newly added heap fields from silently taking a
// derived Clone path. The caller separately funds the destination row slots.
pub(super) fn try_clone_division_movement_sample(
    sample: &DivisionMovementSample,
    budget: &country_roster_readset::Budget,
) -> Result<DivisionMovementSample, Failure> {
    Ok(DivisionMovementSample {
        first_combat_countries: clone_membership(&sample.first_combat_countries, budget)?,
        training_scalars: sample.training_scalars,
        native_type: sample.native_type,
        ordinary_route: sample
            .ordinary_route
            .as_deref()
            .map(|value| clone_slice(value, budget))
            .transpose()?,
        alternate_route: sample
            .alternate_route
            .as_deref()
            .map(|value| clone_slice(value, budget))
            .transpose()?,
        current_template: sample.current_template,
        route_head: sample.route_head,
        template_preference: sample
            .template_preference
            .as_ref()
            .map(|value| clone_preference(value, budget))
            .transpose()?,
        acclimatization: sample
            .acclimatization
            .as_ref()
            .map(|value| clone_acclimatization(value, budget))
            .transpose()?,
        div_id: sample.div_id,
        deleted: sample.deleted,
        moving: sample.moving,
        movement_scalars: sample.movement_scalars,
        location: sample.location,
        move_target_province: sample.move_target_province,
        movement_source_province: sample.movement_source_province,
        raw_organisation: sample.raw_organisation,
        raw_max_organisation: sample.raw_max_organisation,
        organisation_bucket: sample.organisation_bucket,
        raw_strength: sample.raw_strength,
        strength_bucket: sample.strength_bucket,
        combat_ref_count: sample.combat_ref_count,
        in_combat: sample.in_combat,
        owner_tag_db_id: sample.owner_tag_db_id,
        army_id: sample.army_id,
        template_id: sample.template_id,
        raw_experience: sample.raw_experience,
        raw_max_supply: sample.raw_max_supply,
        raw_counter_supply_status: sample.raw_counter_supply_status,
        raw_current_supply: sample.raw_current_supply,
        raw_supply_gain: sample.raw_supply_gain,
        raw_fuel: sample.raw_fuel,
        raw_disrupted_supply: sample.raw_disrupted_supply,
        raw_dig_in: sample.raw_dig_in,
        raw_dig_in_cap: sample.raw_dig_in_cap,
        counter_training_visible: sample.counter_training_visible,
        counter_dig_in_frame: sample.counter_dig_in_frame,
        counter_invalid_orders_visible: sample.counter_invalid_orders_visible,
        strategic_redeployment: sample.strategic_redeployment,
        current_manpower: sample.current_manpower,
        cumulative_losses: sample.cumulative_losses,
        order_link: sample.order_link,
    })
}

// The quota starts fresh each frame, while the previous pin remains alive
// until publication is sealed. Moving that pin into the journal adds no copy.
pub(super) fn retain_baseline_membership(
    tracker: &FrameJournalled<BTreeMap<u32, DivisionMovementSample>>,
    budget: &country_roster_readset::Budget,
) -> Result<(), Failure> {
    use compact_contract::division_routes::{
        CountryWordsObservation, FirstCombatCountriesObservation,
    };
    let mut bytes = 0usize;
    for sample in tracker
        .current
        .values()
        .chain(tracker.frame_entry.iter().flat_map(|entry| entry.values()))
    {
        if let FirstCombatCountriesObservation::Present {
            countries: CountryWordsObservation::Measured(words),
            ..
        } = &sample.first_combat_countries
        {
            bytes = words
                .capacity()
                .checked_mul(std::mem::size_of::<i32>())
                .and_then(|n| bytes.checked_add(n))
                .ok_or(Failure::OutputStorageUnavailable)?;
        }
    }
    budget
        .retain(bytes)
        .map_err(|_| Failure::OutputStorageUnavailable)
}

pub(super) fn reserve_publication(
    out: &mut Vec<u8>,
    payload_len: usize,
    diagnostics: &[CompactDomainPerfDiagnosticUnit],
    budget: &country_roster_readset::Budget,
) -> Result<(), String> {
    let perf_len = diagnostics
        .iter()
        .try_fold(
            40usize + if diagnostics.is_empty() { 0 } else { 8 },
            |len, unit| len.checked_add(12)?.checked_add(unit.name.len()),
        )
        .ok_or("D20 PERF size overflow")?;
    let additional = usize::from(COMPACT_RECORD_HEADER_LEN)
        .checked_mul(2)
        .and_then(|n| n.checked_add(perf_len))
        .and_then(|n| n.checked_add(payload_len))
        .ok_or("D20 publication size overflow")?;
    reserve_exact_frame_growth(out, additional, budget)
        .map_err(|_| "D20 publication storage unavailable".into())
}

// Vec::try_reserve_exact may retain more than requested. Allocate the exact
// funded layout instead, preserving the existing bytes on any refusal. The
// old and new buffers overlap only until the final replacement assignment.
fn reserve_exact_frame_growth(
    out: &mut Vec<u8>,
    additional: usize,
    budget: &country_roster_readset::Budget,
) -> Result<(), Failure> {
    let next_len = out
        .len()
        .checked_add(additional)
        .ok_or(Failure::OutputStorageUnavailable)?;
    if next_len <= out.capacity() {
        return budget
            .retain(additional)
            .map_err(|_| Failure::OutputStorageUnavailable);
    }
    let mut replacement = funded_vec::<u8>(next_len, budget)?;
    replacement.extend_from_slice(out);
    *out = replacement;
    Ok(())
}

// The bounded control refusal uses stack storage independently of semantic
// payload quota. No failed sample or prefix of its country list is emitted.
pub(super) struct StorageGapReservation {
    frame_len: usize,
    copies: FrameCopyReservation,
}

pub(super) fn reserve_storage_gap(
    out: &mut Vec<u8>,
    budget: &Arc<country_roster_readset::Budget>,
    game_hour_raw: i32,
) -> Result<StorageGapReservation, Failure> {
    let bytes =
        usize::from(COMPACT_RECORD_HEADER_LEN) + compact_contract::COMPACT_DOMAIN_GAP_PAYLOAD_LEN;
    let copies = reserve_contribution_copies(bytes, game_hour_raw, budget)
        .map_err(|_| Failure::OutputStorageUnavailable)?;
    reserve_exact_frame_growth(out, bytes, budget)?;
    Ok(StorageGapReservation {
        frame_len: out.len(),
        copies,
    })
}

pub(super) fn append_storage_gap(
    out: &mut Vec<u8>,
    sequence: u64,
    hour: i32,
    ordinal: u64,
    reservation: StorageGapReservation,
) -> FrameCopyReservation {
    // D20 is the first output after acquisition: no other publication may
    // consume the fixed spare capacity between reservation and this refusal.
    assert_eq!(out.len(), reservation.frame_len);
    assert!(
        out.capacity() - out.len()
            >= usize::from(COMPACT_RECORD_HEADER_LEN)
                + compact_contract::COMPACT_DOMAIN_GAP_PAYLOAD_LEN
    );
    use compact_contract::domain_gap::{
        DomainGapReason, DomainGapRecordV1, DomainGapWitnessProvenance,
    };
    let payload = DomainGapRecordV1 {
        part: domain_gap_part(COMPACT_DOMAIN_DIVISIONS),
        reason: DomainGapReason::SourceUnavailable,
        target_hour_raw: hour,
        range_start_hour_raw: hour,
        range_end_hour_raw: hour,
        tick_before: 0,
        tick_after: 0,
        queue_capacity: 0,
        queue_depth: 0,
        queue_dropped: 0,
        outcome_ordinal: ordinal,
    }
    .encode_array_with_witness_provenance(DomainGapWitnessProvenance::Unobserved)
    .expect("bounded D20 storage DOMAIN_GAP");
    append_compact_record(
        out,
        sequence,
        hour,
        COMPACT_DOMAIN_DIVISIONS,
        compact_contract::COMPACT_RECORD_TYPE_DOMAIN_GAP,
        0,
        &payload,
    );
    reservation.copies
}

// D20 owns exactly these five optional diagnostics. Stack storage keeps
// metadata allocation out of the prepared publication's commit interval.
pub(super) fn diagnostics(
    read: &CompactDivisionRead,
) -> ([CompactDomainPerfDiagnosticUnit; 5], usize) {
    let mut units = [CompactDomainPerfDiagnosticUnit { name: "", value: 0 }; 5];
    let mut count = 0;
    if let Some(index) = read.shard_panic_index {
        units[count] = CompactDomainPerfDiagnosticUnit {
            name: "d20_shard_panic_index",
            value: u64::from(index),
        };
        count += 1;
    }
    if read.output_storage_unavailable {
        units[count] = CompactDomainPerfDiagnosticUnit {
            name: "d20_output_storage_unavailable",
            value: 1,
        };
        count += 1;
    }
    if let Some(refusal) = read.frame_identity_refusal {
        for (name, value) in [
            ("d20_frame_identity_cause", refusal.cause.code()),
            ("d20_frame_identity_field", refusal.field.code()),
            ("d20_frame_identity_stage", refusal.stage.code()),
        ] {
            units[count] = CompactDomainPerfDiagnosticUnit {
                name,
                value: u64::from(value),
            };
            count += 1;
        }
    }
    (units, count)
}

pub(super) struct PerfPayload {
    bytes: [u8; 428],
    len: usize,
    flags: u32,
}

pub(super) fn encode_perf(
    read: &CompactDivisionRead,
    elapsed: u64,
    diagnostics: &[CompactDomainPerfDiagnosticUnit],
) -> Result<PerfPayload, String> {
    let record = CompactDomainPerfRecord {
        domain_id: COMPACT_DOMAIN_DIVISIONS,
        read_time_us: read.read_time_us,
        row_count: read.root_rows,
        changed_rows: read.changed_rows,
        unit_count: read.root_rows,
        changed_unit_count: read.changed_rows,
        status_flags: read.status_flags,
    };
    let mut result = PerfPayload {
        bytes: [0; 428],
        len: 40,
        flags: 0,
    };
    result.bytes[..8].copy_from_slice(&elapsed.to_le_bytes());
    result.bytes[8..16].copy_from_slice(&record.read_time_us.to_le_bytes());
    for (offset, word) in [
        (16, record.row_count),
        (20, record.changed_rows),
        (24, record.unit_count),
        (28, record.changed_unit_count),
        (32, record.status_flags),
    ] {
        result.bytes[offset..offset + 4].copy_from_slice(&word.to_le_bytes());
    }
    if diagnostics.is_empty() {
        return Ok(result);
    }
    if diagnostics.len() > 5 {
        return Err("D20 diagnostic count overflow".into());
    }
    result.flags = compact_contract::COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER;
    result.bytes[40..44].copy_from_slice(COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_MAGIC);
    result.bytes[44..46]
        .copy_from_slice(&COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION.to_le_bytes());
    result.bytes[46..48].copy_from_slice(&(diagnostics.len() as u16).to_le_bytes());
    result.len = 48;
    for unit in diagnostics {
        let name = unit.name.as_bytes();
        let end = result
            .len
            .checked_add(12)
            .and_then(|n| n.checked_add(name.len()))
            .filter(|end| *end <= result.bytes.len())
            .ok_or("D20 diagnostic size overflow")?;
        let length = u16::try_from(name.len()).map_err(|_| "D20 diagnostic name overflow")?;
        let declaration =
            u16::from(compact_contract::diagnostic_channel::UnitRole::Undeclared.code())
                | (u16::from(
                    compact_contract::diagnostic_channel::UnitPresence::Undeclared.code(),
                ) << 8);
        result.bytes[result.len..result.len + 2].copy_from_slice(&length.to_le_bytes());
        result.bytes[result.len + 2..result.len + 4].copy_from_slice(&declaration.to_le_bytes());
        result.bytes[result.len + 4..result.len + 12].copy_from_slice(&unit.value.to_le_bytes());
        result.bytes[result.len + 12..end].copy_from_slice(name);
        result.len = end;
    }
    Ok(result)
}

pub(super) fn append_perf(out: &mut Vec<u8>, sequence: u64, hour: i32, perf: &PerfPayload) {
    debug_assert!(out.capacity() - out.len() >= usize::from(COMPACT_RECORD_HEADER_LEN) + perf.len);
    append_compact_record(
        out,
        sequence,
        hour,
        COMPACT_DOMAIN_DIVISIONS,
        COMPACT_RECORD_TYPE_DOMAIN_PERF,
        perf.flags,
        &perf.bytes[..perf.len],
    );
}

// The source-pinned graph in division_output_storage_tests has a peak of four
// current-frame backing stores: original + terminal staged clone + old/new
// terminal replacement. Splices and other replacements need at most two;
// synchronous transport borrows the bytes. D20 has no repeat-gate payload pin.
pub(super) const MAX_LIVE_FRAME_COPIES: usize = 4;

#[derive(Clone)]
pub(super) struct FrameCopyReservation {
    // Keeps this frame's monotonic quota alive through pending/terminal/staged
    // publication. Dropping a temporary buffer never refunds the reservation.
    budget: Arc<country_roster_readset::Budget>,
    game_hour_raw: i32,
    contribution_capacity: usize,
}

impl std::fmt::Debug for FrameCopyReservation {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        out.debug_struct("FrameCopyReservation")
            .field("game_hour_raw", &self.game_hour_raw)
            .field("contribution_capacity", &self.contribution_capacity)
            .finish()
    }
}

pub(super) fn reserve_frame_copies(
    payload_capacity: usize,
    perf: &PerfPayload,
    game_hour_raw: i32,
    budget: &Arc<country_roster_readset::Budget>,
) -> Result<FrameCopyReservation, String> {
    // Use the standalone payload's actual retained capacity, not its length.
    // Later whole-frame Vecs preserve only the initialized D20 record ranges;
    // growth slack introduced by other domains contains no additional D20
    // observation. Include both D20 envelopes and the exact PERF contribution.
    let contribution_capacity = usize::from(COMPACT_RECORD_HEADER_LEN)
        .checked_mul(2)
        .and_then(|n| n.checked_add(perf.len))
        .and_then(|n| n.checked_add(payload_capacity))
        .ok_or("D20 frame copy capacity overflow")?;
    reserve_contribution_copies(contribution_capacity, game_hour_raw, budget)
}

fn reserve_contribution_copies(
    contribution_capacity: usize,
    game_hour_raw: i32,
    budget: &Arc<country_roster_readset::Budget>,
) -> Result<FrameCopyReservation, String> {
    let additional = contribution_capacity
        .checked_mul(MAX_LIVE_FRAME_COPIES - 1)
        .ok_or("D20 frame copy reservation overflow")?;
    budget
        .retain(additional)
        .map_err(|_| "D20 downstream frame copy storage budget exhausted")?;
    Ok(FrameCopyReservation {
        budget: Arc::clone(budget),
        game_hour_raw,
        contribution_capacity,
    })
}

// The production publisher and assembled refusal tests use this same commit
// interval: all variable storage is prepared before DATA, or the pre-funded
// GAP is committed and its receipt travels with the frame.
pub(super) fn publish_read(
    mut read: CompactDivisionRead,
    out: &mut Vec<u8>,
    sequence: &mut u64,
    elapsed_wall_ms: u64,
    game_hour_raw: i32,
    roots: &CompactDivisionRootCache,
    budget: &Arc<country_roster_readset::Budget>,
    division_tracker: &mut FrameJournalled<BTreeMap<u32, DivisionMovementSample>>,
    gap_ordinals: &mut BTreeMap<(u16, u16), u64>,
    domain_outcomes: &mut compact_contract::domain_outcome::DomainOutcomeLedger,
    gap: StorageGapReservation,
) -> FrameCopyReservation {
    let (diagnostic_storage, diagnostic_count) = division_output_storage::diagnostics(&read);
    let shard_diagnostics = &diagnostic_storage[..diagnostic_count];
    let prepared = if read.output_storage_unavailable {
        Err("D20 sampled row output storage unavailable".to_string())
    } else {
        compact_divisions_data_payload(
            &read.rows,
            elapsed_wall_ms,
            read.status_flags,
            read.changed_rows,
            read.root_rows,
            roots.country_roster.as_ref(),
            roots.railway_roster.as_ref(),
            budget,
        )
        .and_then(|payload| {
            let perf =
                division_output_storage::encode_perf(&read, elapsed_wall_ms, shard_diagnostics)?;
            let copies = division_output_storage::reserve_frame_copies(
                payload.capacity(),
                &perf,
                game_hour_raw,
                budget,
            )?;
            division_output_storage::reserve_publication(
                out,
                payload.len(),
                shard_diagnostics,
                budget,
            )?;
            Ok((payload, perf, copies))
        })
    };
    let payload = match prepared {
        Ok((payload, perf, copies)) => Some((payload, perf, copies)),
        Err(reason) => {
            read.status_flags = record_read_failed(
                domain_outcomes,
                COMPACT_DOMAIN_DIVISIONS,
                compact_contract::domain_outcome::READ_PREDICATE_READER_RETURNED_ROWS,
                compact_contract::domain_outcome::ReadCompletion::DidNotComplete,
                compact_contract::domain_outcome::AnswerExtent::Unprovable,
                reason.clone(),
                read.status_flags,
            )
            .bits();
            read.rows.clear();
            read.changed_rows = 0;
            read.output_storage_unavailable = true;
            compact_contract::status_origins::record_status_origin_sample(
                compact_contract::status_origins::SO_D20_OUTPUT_STORAGE_UNAVAILABLE,
                fnv1a_64(reason.as_bytes()),
            );
            None
        }
    };
    super::stage_division_publication_baseline(&read, division_tracker);
    let copies = if let Some((payload, perf, copies)) = payload {
        division_output_storage::append_perf(out, *sequence, game_hour_raw, &perf);
        *sequence = sequence.saturating_add(1);
        append_compact_divisions_data_record(out, *sequence, game_hour_raw, &payload);
        copies
    } else {
        let part = domain_gap_part(COMPACT_DOMAIN_DIVISIONS);
        let key = (COMPACT_DOMAIN_DIVISIONS, part.code());
        let ordinal = *gap_ordinals.get(&key).unwrap_or(&0);
        let copies = division_output_storage::append_storage_gap(
            out,
            *sequence,
            game_hour_raw,
            ordinal,
            gap,
        );
        gap_ordinals.insert(key, ordinal.saturating_add(1));
        copies
    };
    *sequence = sequence.saturating_add(1);
    copies
}
