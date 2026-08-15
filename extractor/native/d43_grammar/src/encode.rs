//! The encoder. It lives in the leaf because a grammar with only readers is
//! half a grammar: the shapes the readers refuse are exactly the shapes the
//! writers must not be able to produce.
//!
//! # What "unconstructible" means here, precisely
//!
//! STRUCTURAL invalidity is unconstructible: no caller supplies a magic, a
//! version word, a `header_len`, a `row_len`, a reserved structural word, a
//! row COUNT, or a block length. Every one of those is derived here from the
//! typed input, so a payload this module produces always delimits.
//!
//! SEMANTIC invalidity stays constructible, and that is deliberate rather than
//! an omission. The lane's public `encode_*` functions are used by tests that
//! deliberately build a row with a variant id and no variant-valid flag, or two
//! residual badges with one id, and then require the decoder to REFUSE it. A
//! leaf that made those unconstructible would either change those functions'
//! signatures or force the tests to be edited, and playbook step 9 forbids
//! editing a test to make the conversion pass. (Codex design thread
//! `01a00772-ce5c-7f53-892d-887c3711ca13`, section F, which reached the same
//! reading of playbook step 6 independently.)

use alloc::vec::Vec;

use crate::{
    fixed_row_len, flow_row_len, CoreRow, CoreWord, DealIdentityShape, DealRow, DealVersion,
    DetailRow, FactoryRow, FlowVersion, Limits, ResourceRow, ResourceRowVersion, SingleVersion,
    StockpileCountryRow, StockpileItemRow, Trailer, FIXED7_RESOURCE_COUNT, FIXED_SCALE_RAW,
    FLOW_FIXED7_ROW_LEN, FLOW_KIND_MASK, FLOW_RESOURCE_MASK, STOCKPILE_COUNTRY_ROW_LEN,
    STOCKPILE_GLOBAL_FLAGS, STOCKPILE_ITEM_ROW_LEN,
};

/// The reasons this module refuses to encode.
///
/// Every one of them is a shape no reader in the tree accepts, so refusing here
/// is refusing to write a payload that would be a total record loss later.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EncodeError {
    /// An `MTRF` row carried a value count that is not the trailer's.
    FlowRowWidth {
        row: usize,
        values: usize,
        expected: usize,
    },
    /// An `MTRF` v3 or `MTRR` resource count outside `1..=`the ceiling, or a
    /// fixed-seven family given a count that is not seven.
    ResourceCount { declared: u32, max: u32 },
    /// An `MTRR` identity token longer than the stride can hold.
    IdentityTokenLen { slot: usize, len: usize, max: usize },
    /// An `MTRR` identity block whose entry count is not the resource count.
    IdentityCount { entries: usize, expected: usize },
    /// An `MTRD` v1/v2 given incomplete owners, which those versions cannot
    /// express.
    IncompleteOwnersUnsupported { version: u16, owners: usize },
    /// An `MTRD` v1-v3 identity that does not fit the packed key's field widths.
    PackedIdentityOverflow { row: usize },
    /// A count that does not fit the `u32` the header spends on it.
    CountOverflow { count: usize },
}

fn u32_count(count: usize) -> Result<u32, EncodeError> {
    u32::try_from(count).map_err(|_| EncodeError::CountOverflow { count })
}

fn push_trailer_header(
    out: &mut Vec<u8>,
    trailer: Trailer,
    version: u16,
    row_len: usize,
    reserved0: u16,
    row_count: u32,
) {
    out.extend_from_slice(trailer.magic());
    out.extend_from_slice(&version.to_le_bytes());
    out.extend_from_slice(&(trailer.header_len() as u16).to_le_bytes());
    out.extend_from_slice(&(row_len as u16).to_le_bytes());
    out.extend_from_slice(&reserved0.to_le_bytes());
    out.extend_from_slice(&row_count.to_le_bytes());
}

// --- the core --------------------------------------------------------------

/// The core header's counted words, named rather than positional.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CoreCounts {
    pub changed_rows: u32,
    pub status_flags: u32,
    pub observed_contract_rows: u32,
    pub emitted_contract_rows: u32,
    pub buyer_ptr_valid_rows: u32,
    pub variant_valid_rows: u32,
    pub amount_valid_rows: u32,
    pub partial_field_rows: u32,
}

/// Encode the core header and rows. `row_count` and `scale_raw` are DERIVED --
/// the caller cannot state either, which is what makes a payload whose header
/// disagrees with its own row count unconstructible.
pub fn encode_core(
    elapsed_wall_ms: u64,
    counts: CoreCounts,
    rows: &[CoreRow],
) -> Result<Vec<u8>, EncodeError> {
    let row_count = u32_count(rows.len())?;
    let mut out = Vec::with_capacity(crate::CORE_HEADER_LEN + rows.len() * crate::CORE_ROW_LEN);
    out.extend_from_slice(&elapsed_wall_ms.to_le_bytes());
    // In `CoreWord::ALL` order, and typed as an array of that length, so a new
    // word cannot be appended to the table without this list failing to build.
    let words: [u32; CoreWord::ALL.len()] = [
        row_count,
        counts.changed_rows,
        counts.status_flags,
        counts.observed_contract_rows,
        counts.emitted_contract_rows,
        counts.buyer_ptr_valid_rows,
        counts.variant_valid_rows,
        counts.amount_valid_rows,
        counts.partial_field_rows,
        FIXED_SCALE_RAW,
    ];
    for word in words {
        out.extend_from_slice(&word.to_le_bytes());
    }
    for row in rows {
        let CoreRow {
            contract_id,
            contract_type,
            flags,
            seller_tag_db_id,
            buyer_tag_db_id,
            equipment_variant_id,
            amount_raw,
            equipment_subbuf_status,
            delivery_subbuf_status,
            buyer_ptr_status,
            reserved,
        } = *row;
        out.extend_from_slice(&contract_id.to_le_bytes());
        out.extend_from_slice(&contract_type.to_le_bytes());
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend_from_slice(&seller_tag_db_id.to_le_bytes());
        out.extend_from_slice(&buyer_tag_db_id.to_le_bytes());
        out.extend_from_slice(&equipment_variant_id.to_le_bytes());
        out.extend_from_slice(&amount_raw.to_le_bytes());
        out.extend_from_slice(&equipment_subbuf_status.to_le_bytes());
        out.extend_from_slice(&delivery_subbuf_status.to_le_bytes());
        out.extend_from_slice(&buyer_ptr_status.to_le_bytes());
        out.extend_from_slice(&reserved.to_le_bytes());
    }
    Ok(out)
}

// --- MTD1 ------------------------------------------------------------------

/// `MTD1`'s four counted summaries.
///
/// The caller states them because they are the producer's OWN claim about its
/// rows, and every reader cross-checks the claim against the rows it decodes --
/// a claim this encoder derived would make that check vacuous.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DetailCounts {
    pub price_valid_rows: u32,
    pub speed_valid_rows: u32,
    pub delivery_route_valid_rows: u32,
    pub progress_valid_rows: u32,
}

pub fn encode_detail(rows: &[DetailRow], counts: DetailCounts) -> Result<Vec<u8>, EncodeError> {
    let trailer = Trailer::Detail;
    let row_len = fixed_row_len(trailer);
    let row_count = u32_count(rows.len())?;
    let mut out = Vec::with_capacity(trailer.header_len() + rows.len() * row_len);
    push_trailer_header(
        &mut out,
        trailer,
        SingleVersion::V1.wire(),
        row_len,
        0,
        row_count,
    );
    out.extend_from_slice(&counts.price_valid_rows.to_le_bytes());
    out.extend_from_slice(&counts.speed_valid_rows.to_le_bytes());
    out.extend_from_slice(&counts.delivery_route_valid_rows.to_le_bytes());
    out.extend_from_slice(&counts.progress_valid_rows.to_le_bytes());
    for row in rows {
        let DetailRow {
            contract_id,
            flags,
            price_raw,
            amount_raw,
            speed_raw,
            completed_raw,
            days,
            delivery_country_tag_db_id,
            convoys_assigned,
            convoys_total,
            efficiency_raw,
            factory_cic_progress_raw,
            equipment_transfer_progress_raw,
            reserved,
        } = *row;
        out.extend_from_slice(&contract_id.to_le_bytes());
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend_from_slice(&price_raw.to_le_bytes());
        out.extend_from_slice(&amount_raw.to_le_bytes());
        out.extend_from_slice(&speed_raw.to_le_bytes());
        out.extend_from_slice(&completed_raw.to_le_bytes());
        out.extend_from_slice(&days.to_le_bytes());
        out.extend_from_slice(&delivery_country_tag_db_id.to_le_bytes());
        out.extend_from_slice(&convoys_assigned.to_le_bytes());
        out.extend_from_slice(&convoys_total.to_le_bytes());
        out.extend_from_slice(&efficiency_raw.to_le_bytes());
        out.extend_from_slice(&factory_cic_progress_raw.to_le_bytes());
        out.extend_from_slice(&equipment_transfer_progress_raw.to_le_bytes());
        out.extend_from_slice(&reserved.to_le_bytes());
    }
    Ok(out)
}

// --- MTRH ------------------------------------------------------------------

pub fn encode_factory_header(rows: &[FactoryRow]) -> Result<Vec<u8>, EncodeError> {
    let trailer = Trailer::FactoryHeader;
    let row_len = fixed_row_len(trailer);
    let row_count = u32_count(rows.len())?;
    let mut out = Vec::with_capacity(trailer.header_len() + rows.len() * row_len);
    push_trailer_header(
        &mut out,
        trailer,
        SingleVersion::V1.wire(),
        row_len,
        0,
        row_count,
    );
    // The country count IS the row count by construction in this family, and
    // the reserved word is zero. Neither is a caller's to state.
    out.extend_from_slice(&row_count.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    for row in rows {
        let FactoryRow {
            tag_db_id,
            total,
            used,
            available,
        } = *row;
        out.extend_from_slice(&tag_db_id.to_le_bytes());
        out.extend_from_slice(&total.to_le_bytes());
        out.extend_from_slice(&used.to_le_bytes());
        out.extend_from_slice(&available.to_le_bytes());
    }
    Ok(out)
}

// --- MTRF ------------------------------------------------------------------

/// One `MTRF` row: the row header's country and kind, plus one value per
/// resource slot.
///
/// The value count is checked against the trailer's resource count, which is
/// what makes a row narrower than its own header unconstructible.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FlowRowInput<'a> {
    pub tag_db_id: u32,
    pub flow_kind: u32,
    pub values_raw: &'a [i64],
}

/// Encode an `MTRF` trailer at `version`.
///
/// `row_len`, the mask/count word and the kind mask are all derived from the
/// version and the resource count; `country_count_or_exported` is the
/// producer's own claim and stays the caller's, for the same reason
/// [`DetailCounts`] does.
pub fn encode_resource_flow(
    version: FlowVersion,
    resource_count: u32,
    country_count_or_exported: u32,
    rows: &[FlowRowInput<'_>],
    limits: Limits,
) -> Result<Vec<u8>, EncodeError> {
    let trailer = Trailer::ResourceFlow;
    let (effective_count, row_len, mask_or_count) = match version {
        FlowVersion::Legacy | FlowVersion::Fixed7 => {
            if resource_count != FIXED7_RESOURCE_COUNT {
                return Err(EncodeError::ResourceCount {
                    declared: resource_count,
                    max: FIXED7_RESOURCE_COUNT,
                });
            }
            (
                FIXED7_RESOURCE_COUNT,
                FLOW_FIXED7_ROW_LEN,
                FLOW_RESOURCE_MASK,
            )
        }
        FlowVersion::Variable => {
            if resource_count == 0 || resource_count > limits.max_resource_count() {
                return Err(EncodeError::ResourceCount {
                    declared: resource_count,
                    max: limits.max_resource_count(),
                });
            }
            (resource_count, flow_row_len(resource_count), resource_count)
        }
    };
    let kind_mask = match version {
        FlowVersion::Legacy => 0,
        FlowVersion::Fixed7 | FlowVersion::Variable => FLOW_KIND_MASK,
    };
    for (index, row) in rows.iter().enumerate() {
        if row.values_raw.len() != effective_count as usize {
            return Err(EncodeError::FlowRowWidth {
                row: index,
                values: row.values_raw.len(),
                expected: effective_count as usize,
            });
        }
    }
    let row_count = u32_count(rows.len())?;
    let mut out = Vec::with_capacity(trailer.header_len() + rows.len() * row_len);
    push_trailer_header(&mut out, trailer, version.wire(), row_len, 0, row_count);
    out.extend_from_slice(&country_count_or_exported.to_le_bytes());
    out.extend_from_slice(&FIXED_SCALE_RAW.to_le_bytes());
    out.extend_from_slice(&mask_or_count.to_le_bytes());
    out.extend_from_slice(&kind_mask.to_le_bytes());
    for row in rows {
        out.extend_from_slice(&row.tag_db_id.to_le_bytes());
        out.extend_from_slice(&row.flow_kind.to_le_bytes());
        out.extend_from_slice(&mask_or_count.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        for value in row.values_raw {
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
    Ok(out)
}

// --- MTRR ------------------------------------------------------------------

/// Encode an `MTRR` trailer at `version`.
///
/// `tokens` must be empty for the versions that carry no identity block and
/// must have one entry per resource for the versions that do; the identity
/// stride, the `+28` word, `row_len` and the row count are derived.
pub fn encode_resource_rows(
    version: ResourceRowVersion,
    resource_count: u32,
    country_count: u32,
    tokens: &[&[u8]],
    rows: &[ResourceRow],
    limits: Limits,
) -> Result<Vec<u8>, EncodeError> {
    let trailer = Trailer::ResourceRow;
    let row_len = fixed_row_len(trailer);
    if version.pins_fixed7() {
        if resource_count != FIXED7_RESOURCE_COUNT {
            return Err(EncodeError::ResourceCount {
                declared: resource_count,
                max: FIXED7_RESOURCE_COUNT,
            });
        }
    } else if resource_count == 0 || resource_count > limits.max_resource_count() {
        return Err(EncodeError::ResourceCount {
            declared: resource_count,
            max: limits.max_resource_count(),
        });
    }
    let expected_tokens = if version.carries_identity() {
        resource_count as usize
    } else {
        0
    };
    if tokens.len() != expected_tokens {
        return Err(EncodeError::IdentityCount {
            entries: tokens.len(),
            expected: expected_tokens,
        });
    }
    let max_token = limits.identity_stride() - 2;
    for (slot, token) in tokens.iter().enumerate() {
        if token.len() > max_token {
            return Err(EncodeError::IdentityTokenLen {
                slot,
                len: token.len(),
                max: max_token,
            });
        }
    }
    let identity_word = if version.carries_identity() {
        limits.identity_stride() as u32
    } else {
        0
    };
    let row_count = u32_count(rows.len())?;
    let mut out = Vec::with_capacity(
        trailer.header_len() + tokens.len() * limits.identity_stride() + rows.len() * row_len,
    );
    push_trailer_header(&mut out, trailer, version.wire(), row_len, 0, row_count);
    out.extend_from_slice(&country_count.to_le_bytes());
    out.extend_from_slice(&resource_count.to_le_bytes());
    // The display scale is one: this family publishes whole units.
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&identity_word.to_le_bytes());
    for token in tokens {
        out.extend_from_slice(&(token.len() as u16).to_le_bytes());
        out.extend_from_slice(token);
        for _ in 0..(limits.identity_stride() - 2 - token.len()) {
            out.push(0);
        }
    }
    for row in rows {
        let ResourceRow {
            tag_db_id,
            resource_index,
            visible_order,
            needed_type,
            balance,
            produced,
            imported,
            exported,
            special_project,
            production_line,
            needed,
            needed_current,
            needed_max,
            flags,
            descriptor_db_index,
            reserved,
        } = *row;
        out.extend_from_slice(&tag_db_id.to_le_bytes());
        out.extend_from_slice(&resource_index.to_le_bytes());
        out.extend_from_slice(&visible_order.to_le_bytes());
        out.extend_from_slice(&needed_type.to_le_bytes());
        out.extend_from_slice(&balance.to_le_bytes());
        out.extend_from_slice(&produced.to_le_bytes());
        out.extend_from_slice(&imported.to_le_bytes());
        out.extend_from_slice(&exported.to_le_bytes());
        out.extend_from_slice(&special_project.to_le_bytes());
        out.extend_from_slice(&production_line.to_le_bytes());
        out.extend_from_slice(&needed.to_le_bytes());
        out.extend_from_slice(&needed_current.to_le_bytes());
        out.extend_from_slice(&needed_max.to_le_bytes());
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend_from_slice(&descriptor_db_index.to_le_bytes());
        out.extend_from_slice(&reserved.to_le_bytes());
    }
    Ok(out)
}

// --- MTRD ------------------------------------------------------------------

/// Encode an `MTRD` trailer at `version`.
///
/// `incomplete_owners` must be empty for v1/v2, which have no word to put a
/// count in: that is a STRUCTURAL refusal, not a taste judgement, because a
/// non-zero word there is how a v3 payload mislabelled v2 is caught.
pub fn encode_classic_deals(
    version: DealVersion,
    witnesses: &[u32],
    incomplete_owners: &[u32],
    rows: &[DealRow],
) -> Result<Vec<u8>, EncodeError> {
    let trailer = Trailer::ClassicDeal;
    let row_len = fixed_row_len(trailer);
    if !version.carries_incomplete_owners() && !incomplete_owners.is_empty() {
        return Err(EncodeError::IncompleteOwnersUnsupported {
            version: version.wire(),
            owners: incomplete_owners.len(),
        });
    }
    let row_count = u32_count(rows.len())?;
    let witness_count = u32_count(witnesses.len())?;
    let incomplete_count = u32_count(incomplete_owners.len())?;
    let mut out = Vec::with_capacity(
        trailer.header_len()
            + witnesses.len() * 4
            + incomplete_owners.len() * 4
            + rows.len() * row_len,
    );
    push_trailer_header(&mut out, trailer, version.wire(), row_len, 0, row_count);
    // The country count IS the witness count in every version of this family.
    out.extend_from_slice(&witness_count.to_le_bytes());
    out.extend_from_slice(&FIXED_SCALE_RAW.to_le_bytes());
    out.extend_from_slice(&witness_count.to_le_bytes());
    out.extend_from_slice(&incomplete_count.to_le_bytes());
    for witness in witnesses {
        out.extend_from_slice(&witness.to_le_bytes());
    }
    for owner in incomplete_owners {
        out.extend_from_slice(&owner.to_le_bytes());
    }
    for (index, row) in rows.iter().enumerate() {
        let DealRow {
            owner_tag_db_id,
            counterpart_tag_db_id,
            resource_index,
            visible_order,
            direction,
            route_kind,
            flags,
            route_status_or_reserved,
            identity_exporter_tag_db_id,
            identity_importer_tag_db_id,
            identity_ordinal,
            packed_identity: _,
            reserved_ordinal_word: _,
            requested_raw,
            delivered_raw,
            available_export_raw,
            factory_cost_raw,
            efficiency_raw,
            civilian_factories,
            convoys_assigned,
            convoys_required,
        } = *row;
        out.extend_from_slice(&owner_tag_db_id.to_le_bytes());
        out.extend_from_slice(&counterpart_tag_db_id.to_le_bytes());
        out.extend_from_slice(&resource_index.to_le_bytes());
        out.extend_from_slice(&visible_order.to_le_bytes());
        out.extend_from_slice(&direction.to_le_bytes());
        out.extend_from_slice(&route_kind.to_le_bytes());
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend_from_slice(&route_status_or_reserved.to_le_bytes());
        // The identity is written at the width THIS VERSION spells, from the
        // same three operands. A caller cannot hand a wide identity to a packed
        // trailer and have it silently truncate: the packed key's field widths
        // are checked, and an operand that does not fit is refused.
        match version.identity_shape() {
            DealIdentityShape::Wide => {
                out.extend_from_slice(&identity_exporter_tag_db_id.to_le_bytes());
                out.extend_from_slice(&identity_importer_tag_db_id.to_le_bytes());
            }
            DealIdentityShape::Packed => {
                if identity_exporter_tag_db_id > 0xFFFF
                    || identity_importer_tag_db_id > 0xFFFF
                    || identity_ordinal > 0xFF
                    || resource_index > 0xFF
                {
                    return Err(EncodeError::PackedIdentityOverflow { row: index });
                }
                let packed = ((identity_exporter_tag_db_id as u64) << 32)
                    | ((identity_importer_tag_db_id as u64) << 16)
                    | ((resource_index as u64) << 8)
                    | (identity_ordinal as u64);
                out.extend_from_slice(&packed.to_le_bytes());
            }
        }
        out.extend_from_slice(&requested_raw.to_le_bytes());
        out.extend_from_slice(&delivered_raw.to_le_bytes());
        out.extend_from_slice(&available_export_raw.to_le_bytes());
        out.extend_from_slice(&factory_cost_raw.to_le_bytes());
        out.extend_from_slice(&efficiency_raw.to_le_bytes());
        out.extend_from_slice(&civilian_factories.to_le_bytes());
        out.extend_from_slice(&convoys_assigned.to_le_bytes());
        out.extend_from_slice(&convoys_required.to_le_bytes());
        match version.identity_shape() {
            DealIdentityShape::Wide => out.extend_from_slice(&identity_ordinal.to_le_bytes()),
            DealIdentityShape::Packed => out.extend_from_slice(&0u32.to_le_bytes()),
        }
    }
    Ok(out)
}

// --- MTRA ------------------------------------------------------------------

pub fn encode_activation(
    flags: u32,
    country_tag_db_id: u32,
    observation_sequence: u64,
    root_kind: u32,
    open_rva: u32,
) -> Vec<u8> {
    let trailer = Trailer::Activation;
    let mut out = Vec::with_capacity(trailer.header_len());
    out.extend_from_slice(trailer.magic());
    out.extend_from_slice(&SingleVersion::V1.wire().to_le_bytes());
    out.extend_from_slice(&(trailer.header_len() as u16).to_le_bytes());
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&country_tag_db_id.to_le_bytes());
    out.extend_from_slice(&observation_sequence.to_le_bytes());
    out.extend_from_slice(&root_kind.to_le_bytes());
    out.extend_from_slice(&open_rva.to_le_bytes());
    out
}

// --- MTS1 ------------------------------------------------------------------

pub fn encode_stockpile(
    expected_country_count: u32,
    countries: &[StockpileCountryRow],
    items: &[StockpileItemRow],
) -> Result<Vec<u8>, EncodeError> {
    let trailer = Trailer::Stockpile;
    let country_count = u32_count(countries.len())?;
    let item_count = u32_count(items.len())?;
    let segment_len = trailer.header_len()
        + countries.len() * STOCKPILE_COUNTRY_ROW_LEN
        + items.len() * STOCKPILE_ITEM_ROW_LEN;
    let mut out = Vec::with_capacity(segment_len);
    out.extend_from_slice(trailer.magic());
    out.extend_from_slice(&SingleVersion::V1.wire().to_le_bytes());
    out.extend_from_slice(&(trailer.header_len() as u16).to_le_bytes());
    out.extend_from_slice(&(STOCKPILE_COUNTRY_ROW_LEN as u16).to_le_bytes());
    out.extend_from_slice(&(STOCKPILE_ITEM_ROW_LEN as u16).to_le_bytes());
    out.extend_from_slice(&u32_count(segment_len)?.to_le_bytes());
    out.extend_from_slice(&country_count.to_le_bytes());
    out.extend_from_slice(&item_count.to_le_bytes());
    out.extend_from_slice(&expected_country_count.to_le_bytes());
    out.extend_from_slice(&FIXED_SCALE_RAW.to_le_bytes());
    out.extend_from_slice(&STOCKPILE_GLOBAL_FLAGS.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    for row in countries {
        let StockpileCountryRow {
            tag_db_id,
            item_first,
            item_count,
            flags,
            reserved,
        } = *row;
        out.extend_from_slice(&tag_db_id.to_le_bytes());
        out.extend_from_slice(&u32_count(item_first)?.to_le_bytes());
        out.extend_from_slice(&u32_count(item_count)?.to_le_bytes());
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend_from_slice(&reserved.to_le_bytes());
    }
    for row in items {
        let StockpileItemRow {
            equipment_variant_id,
            variant_type,
            ordinal,
            reserved,
            amount_raw,
        } = *row;
        out.extend_from_slice(&equipment_variant_id.to_le_bytes());
        out.extend_from_slice(&variant_type.to_le_bytes());
        out.extend_from_slice(&ordinal.to_le_bytes());
        out.extend_from_slice(&reserved.to_le_bytes());
        out.extend_from_slice(&amount_raw.to_le_bytes());
    }
    Ok(out)
}

// --- MTRB and MTLL (lane gate only) ----------------------------------------
//
// The lane gate both DECODES and ENCODES these two families. A leaf that could
// only delimit them would leave the lane's encoder as a second writer of a
// grammar this crate claims to own -- exactly the shape the programme exists to
// remove. (Codex review thread `01a00794-8e7e-7a02-b932-0eeb40336eb3`,
// finding 1.)

/// `MTRB`'s four status counts. The caller states them for the same reason
/// [`DetailCounts`] is stated: they are the producer's own claim about its rows
/// and the decoder cross-checks the claim.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResidualCounts {
    pub source_blocked_rows: u32,
    pub schema_blocked_rows: u32,
    pub owned_elsewhere_rows: u32,
    pub shared_patch_blocked_rows: u32,
}

pub fn encode_residual(
    rows: &[crate::ResidualRow],
    counts: ResidualCounts,
) -> Result<Vec<u8>, EncodeError> {
    let trailer = Trailer::Residual;
    let row_len = fixed_row_len(trailer);
    let row_count = u32_count(rows.len())?;
    let mut out = Vec::with_capacity(trailer.header_len() + rows.len() * row_len);
    push_trailer_header(
        &mut out,
        trailer,
        SingleVersion::V1.wire(),
        row_len,
        0,
        row_count,
    );
    out.extend_from_slice(&counts.source_blocked_rows.to_le_bytes());
    out.extend_from_slice(&counts.schema_blocked_rows.to_le_bytes());
    out.extend_from_slice(&counts.owned_elsewhere_rows.to_le_bytes());
    out.extend_from_slice(&counts.shared_patch_blocked_rows.to_le_bytes());
    for row in rows {
        let crate::ResidualRow {
            gap_id,
            status,
            evidence_mask,
            current_support,
            current_domain_id,
            proposed_record_kind,
            current_row_len,
            source_ref_id,
            owner_boundary,
            root_status,
            proof_status,
            reserved,
        } = *row;
        for word in [
            gap_id,
            status,
            evidence_mask,
            current_support,
            current_domain_id,
            proposed_record_kind,
            current_row_len,
            source_ref_id,
            owner_boundary,
            root_status,
            proof_status,
            reserved,
        ] {
            out.extend_from_slice(&word.to_le_bytes());
        }
    }
    Ok(out)
}

/// `MTLL`'s four direction and presence counts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LendLeaseCounts {
    pub outgoing_rows: u32,
    pub incoming_rows: u32,
    pub amount_valid_rows: u32,
    pub price_valid_rows: u32,
}

pub fn encode_lend_lease(
    rows: &[crate::LendLeaseRow],
    counts: LendLeaseCounts,
) -> Result<Vec<u8>, EncodeError> {
    let trailer = Trailer::LendLease;
    let row_len = fixed_row_len(trailer);
    let row_count = u32_count(rows.len())?;
    let mut out = Vec::with_capacity(trailer.header_len() + rows.len() * row_len);
    push_trailer_header(
        &mut out,
        trailer,
        SingleVersion::V1.wire(),
        row_len,
        0,
        row_count,
    );
    out.extend_from_slice(&counts.outgoing_rows.to_le_bytes());
    out.extend_from_slice(&counts.incoming_rows.to_le_bytes());
    out.extend_from_slice(&counts.amount_valid_rows.to_le_bytes());
    out.extend_from_slice(&counts.price_valid_rows.to_le_bytes());
    for row in rows {
        let crate::LendLeaseRow {
            contract_id,
            perspective_tag_db_id,
            counterpart_tag_db_id,
            direction,
            equipment_variant_id,
            flags,
            amount_raw,
            price_raw,
            source_record_kind,
            reserved,
        } = *row;
        out.extend_from_slice(&contract_id.to_le_bytes());
        out.extend_from_slice(&perspective_tag_db_id.to_le_bytes());
        out.extend_from_slice(&counterpart_tag_db_id.to_le_bytes());
        out.extend_from_slice(&direction.to_le_bytes());
        out.extend_from_slice(&equipment_variant_id.to_le_bytes());
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend_from_slice(&amount_raw.to_le_bytes());
        out.extend_from_slice(&price_raw.to_le_bytes());
        out.extend_from_slice(&source_record_kind.to_le_bytes());
        out.extend_from_slice(&reserved.to_le_bytes());
    }
    Ok(out)
}
