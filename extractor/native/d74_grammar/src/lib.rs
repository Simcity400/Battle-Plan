#![no_std]
#![forbid(unsafe_code)]

extern crate alloc;

use alloc::collections::BTreeSet;
use alloc::string::String;
use alloc::vec::Vec;

pub const DOMAIN_ID: u16 = 74;
pub const MAGIC: &[u8; 4] = b"PBS1";
pub const FORMAT_VERSION: u16 = 1;
pub const HEADER_LEN: u16 = 68;
pub const MAX_PAYLOAD_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_COUNTRIES: usize = 1_024;
pub const MAX_RECORDS: usize = 4_096;
pub const MAX_MEMBERS_PER_RECORD: usize = 1_024;
pub const MAX_OVERRIDES_PER_RECORD: usize = 4_096;
pub const MAX_TOTAL_MEMBERS: usize = 1_048_576;
pub const MAX_TOTAL_OVERRIDES: usize = 262_144;
pub const MAX_TOKEN_BYTES: usize = 65_536;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Country {
    pub tag_db_id: u32,
    pub tag: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemberAccumulator {
    pub tag_db_id: u32,
    pub daily_raw_fixed5: i64,
    pub weekly_raw_fixed5: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SideGfxOverride {
    pub side_id: String,
    pub gfx: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Record {
    pub manager_ordinal: u32,
    pub balance_id: String,
    pub left_side_id: String,
    pub right_side_id: String,
    pub trending_side_id: String,
    pub active_range_id: String,
    pub value_raw_fixed5: i64,
    pub members: Vec<MemberAccumulator>,
    pub side_gfx_overrides: Vec<SideGfxOverride>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Payload {
    pub frame_hour: u64,
    pub status_flags: u32,
    pub elapsed_wall_ms: u64,
    pub roster_identity_sha256: [u8; 32],
    pub countries: Vec<Country>,
    pub records: Vec<Record>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    PayloadTooLarge,
    Truncated,
    BadMagic,
    WrongVersion,
    WrongHeaderLength,
    TrailingBytes,
    CountryCountLimit,
    RecordCountLimit,
    MemberCountLimit,
    OverrideCountLimit,
    TotalMemberCountLimit,
    TotalOverrideCountLimit,
    StringLengthLimit,
    InvalidUtf8,
    EmptyToken,
    ControlCharacter,
    InvalidCountry,
    DuplicateCountryId,
    DuplicateCountryTag,
    UnsortedCountryIds,
    ManagerOrdinalMismatch,
    ValueOutOfRange,
    UnsortedMemberIds,
    DuplicateMemberId,
    UnknownMemberId,
    DuplicateCountryBalancePair,
    DuplicateSideOverride,
    CountOverflow,
}

fn valid_token(value: &str) -> Result<(), Error> {
    if value.len() > MAX_TOKEN_BYTES {
        return Err(Error::StringLengthLimit);
    }
    if value.trim().is_empty() {
        return Err(Error::EmptyToken);
    }
    if value.chars().any(char::is_control) {
        return Err(Error::ControlCharacter);
    }
    Ok(())
}

fn valid_country_tag(tag: &str) -> bool {
    tag.len() == 3
        && tag != "---"
        && tag
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
        && tag.bytes().any(|byte| byte.is_ascii_uppercase())
}

fn validate(payload: &Payload) -> Result<(), Error> {
    if payload.countries.len() > MAX_COUNTRIES {
        return Err(Error::CountryCountLimit);
    }
    if payload.records.len() > MAX_RECORDS {
        return Err(Error::RecordCountLimit);
    }
    if payload.status_flags != 0 {
        return if payload.countries.is_empty() && payload.records.is_empty() {
            Ok(())
        } else {
            Err(Error::InvalidCountry)
        };
    }
    if payload.countries.is_empty() {
        return Err(Error::InvalidCountry);
    }
    let mut country_ids = BTreeSet::new();
    let mut country_tags = BTreeSet::new();
    let mut prior_country = None;
    for country in &payload.countries {
        if country.tag_db_id == 0 || !valid_country_tag(&country.tag) {
            return Err(Error::InvalidCountry);
        }
        if prior_country.is_some_and(|prior| prior >= country.tag_db_id) {
            return Err(if prior_country == Some(country.tag_db_id) {
                Error::DuplicateCountryId
            } else {
                Error::UnsortedCountryIds
            });
        }
        prior_country = Some(country.tag_db_id);
        if !country_ids.insert(country.tag_db_id) {
            return Err(Error::DuplicateCountryId);
        }
        if !country_tags.insert(country.tag.as_str()) {
            return Err(Error::DuplicateCountryTag);
        }
    }
    let mut total_members = 0usize;
    let mut total_overrides = 0usize;
    let mut pairs = BTreeSet::new();
    for (ordinal, record) in payload.records.iter().enumerate() {
        if record.manager_ordinal as usize != ordinal {
            return Err(Error::ManagerOrdinalMismatch);
        }
        for token in [
            &record.balance_id,
            &record.left_side_id,
            &record.right_side_id,
            &record.trending_side_id,
            &record.active_range_id,
        ] {
            valid_token(token)?;
        }
        if !(-100_000..=100_000).contains(&record.value_raw_fixed5) {
            return Err(Error::ValueOutOfRange);
        }
        if record.members.len() > MAX_MEMBERS_PER_RECORD {
            return Err(Error::MemberCountLimit);
        }
        if record.side_gfx_overrides.len() > MAX_OVERRIDES_PER_RECORD {
            return Err(Error::OverrideCountLimit);
        }
        total_members = total_members
            .checked_add(record.members.len())
            .ok_or(Error::CountOverflow)?;
        total_overrides = total_overrides
            .checked_add(record.side_gfx_overrides.len())
            .ok_or(Error::CountOverflow)?;
        if total_members > MAX_TOTAL_MEMBERS {
            return Err(Error::TotalMemberCountLimit);
        }
        if total_overrides > MAX_TOTAL_OVERRIDES {
            return Err(Error::TotalOverrideCountLimit);
        }
        let mut prior_member = None;
        for member in &record.members {
            if prior_member.is_some_and(|prior| prior >= member.tag_db_id) {
                return Err(if prior_member == Some(member.tag_db_id) {
                    Error::DuplicateMemberId
                } else {
                    Error::UnsortedMemberIds
                });
            }
            prior_member = Some(member.tag_db_id);
            if !country_ids.contains(&member.tag_db_id) {
                return Err(Error::UnknownMemberId);
            }
            if !pairs.insert((member.tag_db_id, record.balance_id.as_str())) {
                return Err(Error::DuplicateCountryBalancePair);
            }
        }
        let mut override_ids = BTreeSet::new();
        for row in &record.side_gfx_overrides {
            valid_token(&row.side_id)?;
            valid_token(&row.gfx)?;
            if !override_ids.insert(row.side_id.as_str()) {
                return Err(Error::DuplicateSideOverride);
            }
        }
    }
    Ok(())
}

fn push_string(out: &mut Vec<u8>, value: &str) -> Result<(), Error> {
    valid_token(value)?;
    let len = u32::try_from(value.len()).map_err(|_| Error::StringLengthLimit)?;
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(value.as_bytes());
    Ok(())
}

pub fn encode(payload: &Payload) -> Result<Vec<u8>, Error> {
    validate(payload)?;
    let country_count = u32::try_from(payload.countries.len()).map_err(|_| Error::CountOverflow)?;
    let record_count = u32::try_from(payload.records.len()).map_err(|_| Error::CountOverflow)?;
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    out.extend_from_slice(&HEADER_LEN.to_le_bytes());
    out.extend_from_slice(&payload.frame_hour.to_le_bytes());
    out.extend_from_slice(&payload.status_flags.to_le_bytes());
    out.extend_from_slice(&payload.elapsed_wall_ms.to_le_bytes());
    out.extend_from_slice(&country_count.to_le_bytes());
    out.extend_from_slice(&payload.roster_identity_sha256);
    out.extend_from_slice(&record_count.to_le_bytes());
    for country in &payload.countries {
        out.extend_from_slice(&country.tag_db_id.to_le_bytes());
        push_string(&mut out, &country.tag)?;
    }
    for record in &payload.records {
        out.extend_from_slice(&record.manager_ordinal.to_le_bytes());
        out.extend_from_slice(&record.value_raw_fixed5.to_le_bytes());
        for token in [
            &record.balance_id,
            &record.left_side_id,
            &record.right_side_id,
            &record.trending_side_id,
            &record.active_range_id,
        ] {
            push_string(&mut out, token)?;
        }
        out.extend_from_slice(
            &u32::try_from(record.members.len())
                .map_err(|_| Error::CountOverflow)?
                .to_le_bytes(),
        );
        for member in &record.members {
            out.extend_from_slice(&member.tag_db_id.to_le_bytes());
            out.extend_from_slice(&member.daily_raw_fixed5.to_le_bytes());
            out.extend_from_slice(&member.weekly_raw_fixed5.to_le_bytes());
        }
        out.extend_from_slice(
            &u32::try_from(record.side_gfx_overrides.len())
                .map_err(|_| Error::CountOverflow)?
                .to_le_bytes(),
        );
        for row in &record.side_gfx_overrides {
            push_string(&mut out, &row.side_id)?;
            push_string(&mut out, &row.gfx)?;
        }
    }
    if out.len() > MAX_PAYLOAD_BYTES {
        return Err(Error::PayloadTooLarge);
    }
    Ok(out)
}

pub fn encode_unavailable(
    frame_hour: u64,
    status_flags: u32,
    elapsed_wall_ms: u64,
) -> Result<Vec<u8>, Error> {
    encode(&Payload {
        frame_hour,
        status_flags,
        elapsed_wall_ms,
        roster_identity_sha256: [0; 32],
        countries: Vec::new(),
        records: Vec::new(),
    })
}

struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, len: usize) -> Result<&'a [u8], Error> {
        let end = self.at.checked_add(len).ok_or(Error::Truncated)?;
        let value = self.bytes.get(self.at..end).ok_or(Error::Truncated)?;
        self.at = end;
        Ok(value)
    }

    fn u16(&mut self) -> Result<u16, Error> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }

    fn u32(&mut self) -> Result<u32, Error> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn u64(&mut self) -> Result<u64, Error> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn i64(&mut self) -> Result<i64, Error> {
        Ok(i64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }

    fn string(&mut self) -> Result<String, Error> {
        let len = usize::try_from(self.u32()?).map_err(|_| Error::StringLengthLimit)?;
        if len > MAX_TOKEN_BYTES {
            return Err(Error::StringLengthLimit);
        }
        let value = core::str::from_utf8(self.take(len)?).map_err(|_| Error::InvalidUtf8)?;
        valid_token(value)?;
        Ok(String::from(value))
    }
}

pub fn decode(payload: &[u8]) -> Result<Payload, Error> {
    if payload.len() > MAX_PAYLOAD_BYTES {
        return Err(Error::PayloadTooLarge);
    }
    let mut cursor = Cursor {
        bytes: payload,
        at: 0,
    };
    if cursor.take(4)? != MAGIC {
        return Err(Error::BadMagic);
    }
    if cursor.u16()? != FORMAT_VERSION {
        return Err(Error::WrongVersion);
    }
    if cursor.u16()? != HEADER_LEN {
        return Err(Error::WrongHeaderLength);
    }
    let frame_hour = cursor.u64()?;
    let status_flags = cursor.u32()?;
    let elapsed_wall_ms = cursor.u64()?;
    let country_count = usize::try_from(cursor.u32()?).map_err(|_| Error::CountryCountLimit)?;
    if country_count > MAX_COUNTRIES {
        return Err(Error::CountryCountLimit);
    }
    let mut roster_identity_sha256 = [0u8; 32];
    roster_identity_sha256.copy_from_slice(cursor.take(32)?);
    let record_count = usize::try_from(cursor.u32()?).map_err(|_| Error::RecordCountLimit)?;
    if record_count > MAX_RECORDS {
        return Err(Error::RecordCountLimit);
    }
    let mut countries = Vec::with_capacity(country_count);
    for _ in 0..country_count {
        countries.push(Country {
            tag_db_id: cursor.u32()?,
            tag: cursor.string()?,
        });
    }
    let mut records = Vec::with_capacity(record_count);
    let mut total_members = 0usize;
    let mut total_overrides = 0usize;
    for _ in 0..record_count {
        let manager_ordinal = cursor.u32()?;
        let value_raw_fixed5 = cursor.i64()?;
        let balance_id = cursor.string()?;
        let left_side_id = cursor.string()?;
        let right_side_id = cursor.string()?;
        let trending_side_id = cursor.string()?;
        let active_range_id = cursor.string()?;
        let member_count = usize::try_from(cursor.u32()?).map_err(|_| Error::MemberCountLimit)?;
        if member_count > MAX_MEMBERS_PER_RECORD {
            return Err(Error::MemberCountLimit);
        }
        total_members = total_members
            .checked_add(member_count)
            .ok_or(Error::CountOverflow)?;
        if total_members > MAX_TOTAL_MEMBERS {
            return Err(Error::TotalMemberCountLimit);
        }
        let member_bytes = member_count.checked_mul(20).ok_or(Error::CountOverflow)?;
        if cursor.bytes.len().saturating_sub(cursor.at) < member_bytes {
            return Err(Error::Truncated);
        }
        let mut members = Vec::with_capacity(member_count);
        for _ in 0..member_count {
            members.push(MemberAccumulator {
                tag_db_id: cursor.u32()?,
                daily_raw_fixed5: cursor.i64()?,
                weekly_raw_fixed5: cursor.i64()?,
            });
        }
        let override_count =
            usize::try_from(cursor.u32()?).map_err(|_| Error::OverrideCountLimit)?;
        if override_count > MAX_OVERRIDES_PER_RECORD {
            return Err(Error::OverrideCountLimit);
        }
        total_overrides = total_overrides
            .checked_add(override_count)
            .ok_or(Error::CountOverflow)?;
        if total_overrides > MAX_TOTAL_OVERRIDES {
            return Err(Error::TotalOverrideCountLimit);
        }
        let minimum_override_bytes = override_count.checked_mul(8).ok_or(Error::CountOverflow)?;
        if cursor.bytes.len().saturating_sub(cursor.at) < minimum_override_bytes {
            return Err(Error::Truncated);
        }
        let mut side_gfx_overrides = Vec::with_capacity(override_count);
        for _ in 0..override_count {
            side_gfx_overrides.push(SideGfxOverride {
                side_id: cursor.string()?,
                gfx: cursor.string()?,
            });
        }
        records.push(Record {
            manager_ordinal,
            balance_id,
            left_side_id,
            right_side_id,
            trending_side_id,
            active_range_id,
            value_raw_fixed5,
            members,
            side_gfx_overrides,
        });
    }
    if cursor.at != payload.len() {
        return Err(Error::TrailingBytes);
    }
    let decoded = Payload {
        frame_hour,
        status_flags,
        elapsed_wall_ms,
        roster_identity_sha256,
        countries,
        records,
    };
    validate(&decoded)?;
    Ok(decoded)
}

#[cfg(test)]
mod tests;
