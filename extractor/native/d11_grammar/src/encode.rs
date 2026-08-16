//! The encoder for the base payload and every outer trailer family.
//!
//! Every function preflights every count, conversion and exact byte length,
//! calls `try_reserve_exact` once, and only then starts writing. No encoder can
//! mutate its output and later discover a fallible conversion.

use alloc::vec::Vec;
use core::convert::TryFrom;

use crate::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EncodeError {
    CountOverflow { count: usize },
    LengthOverflow { left: usize, right: usize },
    LengthDoesNotFitU32 { len: usize },
    Allocation { requested: usize },
}

fn count32(count: usize) -> Result<u32, EncodeError> {
    u32::try_from(count).map_err(|_| EncodeError::CountOverflow { count })
}

fn len32(len: usize) -> Result<u32, EncodeError> {
    u32::try_from(len).map_err(|_| EncodeError::LengthDoesNotFitU32 { len })
}

fn add(left: usize, right: usize) -> Result<usize, EncodeError> {
    left.checked_add(right)
        .ok_or(EncodeError::LengthOverflow { left, right })
}

fn mul(left: usize, right: usize) -> Result<usize, EncodeError> {
    left.checked_mul(right)
        .ok_or(EncodeError::LengthOverflow { left, right })
}

fn exact_vec(len: usize) -> Result<Vec<u8>, EncodeError> {
    let mut out = Vec::new();
    out.try_reserve_exact(len)
        .map_err(|_| EncodeError::Allocation { requested: len })?;
    Ok(out)
}

fn push_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn push_i32(out: &mut Vec<u8>, value: i32) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn push_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}
fn push_i64(out: &mut Vec<u8>, value: i64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn fixed_preflight(row_count: usize, row_len: usize) -> Result<(usize, u32, u32), EncodeError> {
    let rows = mul(row_count, row_len)?;
    let len = add(FIXED_HEADER_LEN, rows)?;
    Ok((len, count32(row_count)?, len32(len)?))
}

fn push_fixed_header(out: &mut Vec<u8>, trailer: Trailer, len: u32, rows: u32) {
    out.extend_from_slice(trailer.magic());
    push_u16(out, 1);
    push_u16(out, FIXED_HEADER_LEN as u16);
    push_u32(out, len);
    push_u32(out, rows);
    push_u32(out, 0);
}

fn push_line_key(out: &mut Vec<u8>, key: LineKey) {
    let LineKey {
        tag_db_id,
        kind,
        line_id,
    } = key;
    push_u32(out, tag_db_id);
    push_u32(out, kind);
    push_i32(out, line_id);
}

// =============================================================================
// Base
// =============================================================================

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BaseWords {
    pub country_count: u32,
    pub changed_rows: u32,
    pub status_flags: u32,
    pub scale_raw: u32,
}

pub fn encode_base(
    elapsed_wall_ms: u64,
    words: BaseWords,
    stockpiles: &[StockpileRow],
    military_lines: &[LineRow],
    naval_lines: &[LineRow],
    construction: &[ConstructionRow],
) -> Result<Vec<u8>, EncodeError> {
    let stockpile_count = count32(stockpiles.len())?;
    let military_count = count32(military_lines.len())?;
    let naval_count = count32(naval_lines.len())?;
    let construction_count = count32(construction.len())?;
    let line_count = add(military_lines.len(), naval_lines.len())?;
    let stockpile_bytes = mul(stockpiles.len(), STOCKPILE_ROW_LEN)?;
    let line_bytes = mul(line_count, LINE_ROW_LEN)?;
    let construction_bytes = mul(construction.len(), CONSTRUCTION_ROW_LEN)?;
    let len = add(
        add(add(BASE_HEADER_LEN, stockpile_bytes)?, line_bytes)?,
        construction_bytes,
    )?;
    let mut out = exact_vec(len)?;
    push_u64(&mut out, elapsed_wall_ms);
    push_u32(&mut out, words.country_count);
    push_u32(&mut out, words.changed_rows);
    push_u32(&mut out, words.status_flags);
    push_u32(&mut out, words.scale_raw);
    push_u32(&mut out, stockpile_count);
    push_u32(&mut out, military_count);
    push_u32(&mut out, naval_count);
    push_u32(&mut out, construction_count);
    for row in stockpiles {
        let StockpileRow {
            tag_db_id,
            variant_id,
            amount_raw,
        } = *row;
        push_u32(&mut out, tag_db_id);
        push_u32(&mut out, variant_id);
        push_i64(&mut out, amount_raw);
    }
    for row in military_lines.iter().chain(naval_lines.iter()) {
        let LineRow {
            tag_db_id,
            kind,
            line_id,
            active_factories,
            cost_raw,
            speed_raw,
            produced_raw,
            amount,
        } = *row;
        push_u32(&mut out, tag_db_id);
        push_u32(&mut out, kind);
        push_i32(&mut out, line_id);
        push_i32(&mut out, active_factories);
        push_i32(&mut out, cost_raw);
        push_i32(&mut out, speed_raw);
        push_i32(&mut out, produced_raw);
        push_i32(&mut out, amount);
    }
    for row in construction {
        let ConstructionRow {
            tag_db_id,
            line_id,
            template_id,
            active_factories,
            cost_raw,
            produced_raw,
            priority,
            reserved,
        } = *row;
        push_u32(&mut out, tag_db_id);
        push_u32(&mut out, line_id);
        push_u32(&mut out, template_id);
        push_i32(&mut out, active_factories);
        push_i32(&mut out, cost_raw);
        push_i32(&mut out, produced_raw);
        push_u32(&mut out, priority);
        push_u32(&mut out, reserved);
    }
    Ok(out)
}

// =============================================================================
// Seven fixed-row trailers
// =============================================================================

pub fn encode_pvj1(rows: &[PvjRow]) -> Result<Vec<u8>, EncodeError> {
    let (len, count, declared) = fixed_preflight(rows.len(), 16)?;
    let mut out = exact_vec(len)?;
    push_fixed_header(&mut out, Trailer::Pvj1, declared, count);
    for row in rows {
        push_line_key(&mut out, row.key);
        push_u32(&mut out, row.equipment_variant_id);
    }
    Ok(out)
}

pub fn encode_pfr1(rows: &[PfrRow]) -> Result<Vec<u8>, EncodeError> {
    let (len, count, declared) = fixed_preflight(rows.len(), 16)?;
    let mut out = exact_vec(len)?;
    push_fixed_header(&mut out, Trailer::Pfr1, declared, count);
    for row in rows {
        push_line_key(&mut out, row.key);
        push_i32(&mut out, row.requested_factories);
    }
    Ok(out)
}

pub fn encode_ppr1(rows: &[PprRow]) -> Result<Vec<u8>, EncodeError> {
    let (len, count, declared) = fixed_preflight(rows.len(), 16)?;
    let mut out = exact_vec(len)?;
    push_fixed_header(&mut out, Trailer::Ppr1, declared, count);
    for row in rows {
        push_line_key(&mut out, row.key);
        push_i32(&mut out, row.priority);
    }
    Ok(out)
}

pub fn encode_pls1(rows: &[PlsRow]) -> Result<Vec<u8>, EncodeError> {
    let (len, count, declared) = fixed_preflight(rows.len(), 16)?;
    let mut out = exact_vec(len)?;
    push_fixed_header(&mut out, Trailer::Pls1, declared, count);
    for row in rows {
        push_line_key(&mut out, row.key);
        push_u32(&mut out, row.subtype);
    }
    Ok(out)
}

pub fn encode_pmi1(rows: &[PmiRow]) -> Result<Vec<u8>, EncodeError> {
    let (len, count, declared) = fixed_preflight(rows.len(), 16)?;
    let mut out = exact_vec(len)?;
    push_fixed_header(&mut out, Trailer::Pmi1, declared, count);
    for row in rows {
        push_line_key(&mut out, row.key);
        push_u32(&mut out, row.mio_id);
    }
    Ok(out)
}

pub fn encode_pef1(rows: &[PefRow]) -> Result<Vec<u8>, EncodeError> {
    let (len, count, declared) = fixed_preflight(rows.len(), 36)?;
    let mut out = exact_vec(len)?;
    push_fixed_header(&mut out, Trailer::Pef1, declared, count);
    for row in rows {
        push_line_key(&mut out, row.key);
        push_i64(&mut out, row.efficiency_raw);
        push_i64(&mut out, row.efficiency_cap_raw);
        push_i64(&mut out, row.efficiency_growth_raw);
    }
    Ok(out)
}

pub fn encode_pcp1(rows: &[PcpRow]) -> Result<Vec<u8>, EncodeError> {
    let (len, count, declared) = fixed_preflight(rows.len(), 8)?;
    let mut out = exact_vec(len)?;
    push_fixed_header(&mut out, Trailer::Pcp1, declared, count);
    for row in rows {
        push_u32(&mut out, row.tag_db_id);
        push_u32(&mut out, row.flags);
    }
    Ok(out)
}

// =============================================================================
// PLIC -- grouping is an input shape, so sum(count) == id_rows is unbreakable
// =============================================================================

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlicCountryV1<'a> {
    pub tag_db_id: u32,
    pub items: &'a [PlicV1Item],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlicCountryV2<'a> {
    pub tag_db_id: u32,
    pub items: &'a [PlicV2Item],
}

fn plic_preflight(
    countries: usize,
    items: usize,
    item_len: usize,
) -> Result<(usize, u32, u32), EncodeError> {
    let country_bytes = mul(countries, PLIC_COUNTRY_ROW_LEN)?;
    let item_bytes = mul(items, item_len)?;
    let len = add(add(FIXED_HEADER_LEN, country_bytes)?, item_bytes)?;
    Ok((len, count32(countries)?, count32(items)?))
}

pub fn encode_plic_v1(countries: &[PlicCountryV1<'_>]) -> Result<Vec<u8>, EncodeError> {
    let mut items = 0usize;
    for country in countries {
        count32(country.items.len())?;
        items = add(items, country.items.len())?;
    }
    let (len, country_count, item_count) =
        plic_preflight(countries.len(), items, PLIC_V1_ITEM_LEN)?;
    let mut out = exact_vec(len)?;
    out.extend_from_slice(Trailer::Plic.magic());
    push_u16(&mut out, PlicVersion::V1.wire());
    push_u16(&mut out, FIXED_HEADER_LEN as u16);
    push_u32(&mut out, country_count);
    push_u32(&mut out, item_count);
    push_u32(&mut out, 0);
    for country in countries {
        push_u32(&mut out, country.tag_db_id);
        push_u32(&mut out, country.items.len() as u32);
    }
    for country in countries {
        for item in country.items {
            push_u32(&mut out, item.variant_id);
        }
    }
    Ok(out)
}

pub fn encode_plic_v2(countries: &[PlicCountryV2<'_>]) -> Result<Vec<u8>, EncodeError> {
    let mut items = 0usize;
    for country in countries {
        count32(country.items.len())?;
        items = add(items, country.items.len())?;
    }
    let (len, country_count, item_count) =
        plic_preflight(countries.len(), items, PLIC_V2_ITEM_LEN)?;
    let mut out = exact_vec(len)?;
    out.extend_from_slice(Trailer::Plic.magic());
    push_u16(&mut out, PlicVersion::V2.wire());
    push_u16(&mut out, FIXED_HEADER_LEN as u16);
    push_u32(&mut out, country_count);
    push_u32(&mut out, item_count);
    push_u32(&mut out, 0);
    for country in countries {
        push_u32(&mut out, country.tag_db_id);
        push_u32(&mut out, country.items.len() as u32);
    }
    for country in countries {
        for item in country.items {
            let PlicV2Item {
                variant_id,
                visible_order,
                creator_tag_db_id,
                origin_tag_db_id,
            } = *item;
            push_u32(&mut out, variant_id);
            push_u32(&mut out, visible_order);
            push_u32(&mut out, creator_tag_db_id);
            push_u32(&mut out, origin_tag_db_id);
        }
    }
    Ok(out)
}

// =============================================================================
// PCV1 -- nested slices make canonical ranges and exact coverage unbreakable
// =============================================================================

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PcvCountryInput<'a> {
    pub tag_db_id: u32,
    pub flags: u32,
    pub owned_raw: i64,
    pub assigned: i32,
    pub available: i32,
    pub variants: &'a [PcvVariantRow],
    pub reservations: &'a [PcvReservationRow],
}

pub fn encode_pcv1(countries: &[PcvCountryInput<'_>]) -> Result<Vec<u8>, EncodeError> {
    let country_count = count32(countries.len())?;
    let mut variants = 0usize;
    let mut reservations = 0usize;
    for country in countries {
        count32(country.variants.len())?;
        count32(country.reservations.len())?;
        variants = add(variants, country.variants.len())?;
        reservations = add(reservations, country.reservations.len())?;
    }
    let variant_count = count32(variants)?;
    let reservation_count = count32(reservations)?;
    let country_bytes = mul(countries.len(), PCV_COUNTRY_ROW_LEN)?;
    let variant_bytes = mul(variants, PCV_VARIANT_ROW_LEN)?;
    let reservation_bytes = mul(reservations, PCV_RESERVATION_ROW_LEN)?;
    let len = add(
        add(add(PCV_HEADER_LEN, country_bytes)?, variant_bytes)?,
        reservation_bytes,
    )?;
    let declared = len32(len)?;
    let mut out = exact_vec(len)?;
    out.extend_from_slice(Trailer::Pcv1.magic());
    push_u16(&mut out, 1);
    push_u16(&mut out, PCV_HEADER_LEN as u16);
    push_u32(&mut out, declared);
    push_u32(&mut out, country_count);
    push_u32(&mut out, variant_count);
    push_u32(&mut out, reservation_count);
    push_u32(&mut out, 0);
    let mut variant_start = 0u32;
    let mut reservation_start = 0u32;
    for country in countries {
        let variant_len = country.variants.len() as u32;
        let reservation_len = country.reservations.len() as u32;
        push_u32(&mut out, country.tag_db_id);
        push_u32(&mut out, country.flags);
        push_i64(&mut out, country.owned_raw);
        push_i32(&mut out, country.assigned);
        push_i32(&mut out, country.available);
        push_u32(&mut out, variant_start);
        push_u32(&mut out, variant_len);
        push_u32(&mut out, reservation_start);
        push_u32(&mut out, reservation_len);
        variant_start += variant_len;
        reservation_start += reservation_len;
    }
    for country in countries {
        for row in country.variants {
            push_u32(&mut out, row.variant_id);
            push_u32(&mut out, row.equip_type);
            push_i64(&mut out, row.amount_raw);
        }
    }
    for country in countries {
        for row in country.reservations {
            push_i32(&mut out, row.priority);
            push_i32(&mut out, row.count);
        }
    }
    Ok(out)
}

// =============================================================================
// PDQ1 outer carrier; the inner codec bytes are opaque here
// =============================================================================

pub fn encode_pdq1(inner_codec_payload: &[u8]) -> Result<Vec<u8>, EncodeError> {
    let codec_len = count32(inner_codec_payload.len())?;
    let len = add(FIXED_HEADER_LEN, inner_codec_payload.len())?;
    let declared = len32(len)?;
    let mut out = exact_vec(len)?;
    out.extend_from_slice(Trailer::Pdq1.magic());
    push_u16(&mut out, 1);
    push_u16(&mut out, FIXED_HEADER_LEN as u16);
    push_u32(&mut out, declared);
    push_u32(&mut out, codec_len);
    push_u32(&mut out, 0);
    out.extend_from_slice(inner_codec_payload);
    Ok(out)
}
