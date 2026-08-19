use crate::record_semantics::KnownRecordType;
use std::collections::BTreeMap;

pub const CONTENT_IDENTITY_CENSUS_MAGIC: &[u8; 4] = b"H4CI";
pub const CONTENT_IDENTITY_CENSUS_VERSION: u16 = 1;
pub const CONTENT_IDENTITY_CENSUS_HEADER_LEN: usize = 40;
pub const CONTENT_IDENTITY_CENSUS_ROW_LEN: usize = 12;

pub const CONTENT_IDENTITY_CENSUS_MAX_ROWS_PER_NAMESPACE: usize = 8_192;
pub const CONTENT_IDENTITY_CENSUS_MAX_TOKEN_LEN: usize = 256;
pub const CONTENT_IDENTITY_CENSUS_MAX_BLOB_LEN: usize =
    2 * CONTENT_IDENTITY_CENSUS_MAX_ROWS_PER_NAMESPACE * CONTENT_IDENTITY_CENSUS_MAX_TOKEN_LEN;
pub const CONTENT_IDENTITY_CENSUS_MAX_PAYLOAD_LEN: usize = CONTENT_IDENTITY_CENSUS_HEADER_LEN
    + 2 * CONTENT_IDENTITY_CENSUS_MAX_ROWS_PER_NAMESPACE * CONTENT_IDENTITY_CENSUS_ROW_LEN
    + CONTENT_IDENTITY_CENSUS_MAX_BLOB_LEN;

pub const CONTENT_IDENTITY_STATUS_BUILDINGS_UNREADABLE: u16 = 1 << 0;
pub const CONTENT_IDENTITY_STATUS_RESOURCES_UNREADABLE: u16 = 1 << 1;
pub const CONTENT_IDENTITY_STATUS_BOUND_TRUNCATED: u16 = 1 << 2;
pub const CONTENT_IDENTITY_STATUS_CONFLICT_DROPPED: u16 = 1 << 3;
pub const CONTENT_IDENTITY_STATUS_KNOWN_MASK: u16 = CONTENT_IDENTITY_STATUS_BUILDINGS_UNREADABLE
    | CONTENT_IDENTITY_STATUS_RESOURCES_UNREADABLE
    | CONTENT_IDENTITY_STATUS_BOUND_TRUNCATED
    | CONTENT_IDENTITY_STATUS_CONFLICT_DROPPED;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u16)]
pub enum ContentIdentityNamespace {
    Building = 1,
    StrategicResource = 2,
}

impl ContentIdentityNamespace {
    pub const fn code(self) -> u16 {
        self as u16
    }

    pub const fn canonical_name(self) -> &'static str {
        match self {
            Self::Building => "building",
            Self::StrategicResource => "strategic_resource",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentIdentityRow {
    pub token_id: u32,
    pub token: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentIdentityCensusV1 {
    pub buildings: Vec<ContentIdentityRow>,
    pub resources: Vec<ContentIdentityRow>,
    pub buildings_declared: u32,
    pub resources_declared: u32,
    pub status: u16,
}

impl ContentIdentityCensusV1 {
    pub fn is_complete(&self) -> bool {
        self.status == 0
            && self.buildings.len() as u64 == u64::from(self.buildings_declared)
            && self.resources.len() as u64 == u64::from(self.resources_declared)
    }

    pub fn rows(&self, namespace: ContentIdentityNamespace) -> &[ContentIdentityRow] {
        match namespace {
            ContentIdentityNamespace::Building => &self.buildings,
            ContentIdentityNamespace::StrategicResource => &self.resources,
        }
    }

    pub fn token(&self, namespace: ContentIdentityNamespace, token_id: u32) -> Option<&str> {
        self.rows(namespace)
            .binary_search_by_key(&token_id, |row| row.token_id)
            .ok()
            .map(|index| self.rows(namespace)[index].token.as_str())
    }

    pub fn status_names(&self) -> Vec<&'static str> {
        let mut names = Vec::new();
        if self.status & CONTENT_IDENTITY_STATUS_BUILDINGS_UNREADABLE != 0 {
            names.push("buildings_unreadable");
        }
        if self.status & CONTENT_IDENTITY_STATUS_RESOURCES_UNREADABLE != 0 {
            names.push("resources_unreadable");
        }
        if self.status & CONTENT_IDENTITY_STATUS_BOUND_TRUNCATED != 0 {
            names.push("bound_truncated");
        }
        if self.status & CONTENT_IDENTITY_STATUS_CONFLICT_DROPPED != 0 {
            names.push("conflict_dropped");
        }
        names
    }

    pub fn encode(&self) -> Result<Vec<u8>, String> {
        if self.status & !CONTENT_IDENTITY_STATUS_KNOWN_MASK != 0 {
            return Err(format!(
                "content identity census status {:#06x} contains unknown bits",
                self.status
            ));
        }
        for namespace in [
            ContentIdentityNamespace::Building,
            ContentIdentityNamespace::StrategicResource,
        ] {
            let rows = self.rows(namespace);
            if rows.len() > CONTENT_IDENTITY_CENSUS_MAX_ROWS_PER_NAMESPACE {
                return Err(format!(
                    "content identity census {} namespace has {} rows, more than {CONTENT_IDENTITY_CENSUS_MAX_ROWS_PER_NAMESPACE}",
                    namespace.canonical_name(),
                    rows.len()
                ));
            }
            let mut previous: Option<u32> = None;
            for row in rows {
                validate_token(&row.token, row.token_id)?;
                if let Some(previous) = previous {
                    if row.token_id <= previous {
                        return Err(format!(
                            "content identity census {} rows are not strictly increasing at id {:#010x}",
                            namespace.canonical_name(),
                            row.token_id
                        ));
                    }
                }
                previous = Some(row.token_id);
            }
        }
        identity_is_a_bijection(&self.buildings, &self.resources)?;

        let mut blob: Vec<u8> = Vec::new();
        let mut rows_bytes: Vec<u8> = Vec::new();
        for namespace in [
            ContentIdentityNamespace::Building,
            ContentIdentityNamespace::StrategicResource,
        ] {
            for row in self.rows(namespace) {
                let offset = u32::try_from(blob.len())
                    .map_err(|_| "content identity census token blob is too large".to_string())?;
                rows_bytes.extend_from_slice(&row.token_id.to_le_bytes());
                rows_bytes.extend_from_slice(&offset.to_le_bytes());
                rows_bytes.extend_from_slice(&(row.token.len() as u16).to_le_bytes());
                rows_bytes.extend_from_slice(&0u16.to_le_bytes());
                blob.extend_from_slice(row.token.as_bytes());
            }
        }
        if blob.len() > CONTENT_IDENTITY_CENSUS_MAX_BLOB_LEN {
            return Err(format!(
                "content identity census token blob is {} bytes, more than {CONTENT_IDENTITY_CENSUS_MAX_BLOB_LEN}",
                blob.len()
            ));
        }

        let mut out =
            Vec::with_capacity(CONTENT_IDENTITY_CENSUS_HEADER_LEN + rows_bytes.len() + blob.len());
        out.extend_from_slice(CONTENT_IDENTITY_CENSUS_MAGIC);
        out.extend_from_slice(&CONTENT_IDENTITY_CENSUS_VERSION.to_le_bytes());
        out.extend_from_slice(&(CONTENT_IDENTITY_CENSUS_HEADER_LEN as u16).to_le_bytes());
        out.extend_from_slice(&self.status.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&(self.buildings.len() as u32).to_le_bytes());
        out.extend_from_slice(&(self.resources.len() as u32).to_le_bytes());
        out.extend_from_slice(&(blob.len() as u32).to_le_bytes());
        out.extend_from_slice(&self.buildings_declared.to_le_bytes());
        out.extend_from_slice(&self.resources_declared.to_le_bytes());
        out.extend_from_slice(&0u64.to_le_bytes());
        debug_assert_eq!(out.len(), CONTENT_IDENTITY_CENSUS_HEADER_LEN);
        out.extend_from_slice(&rows_bytes);
        out.extend_from_slice(&blob);
        Ok(out)
    }

    pub fn decode(payload: &[u8]) -> Result<Self, String> {
        if payload.len() > CONTENT_IDENTITY_CENSUS_MAX_PAYLOAD_LEN {
            return Err(format!(
                "content identity census payload has {} byte(s), more than {CONTENT_IDENTITY_CENSUS_MAX_PAYLOAD_LEN}",
                payload.len()
            ));
        }
        if payload.len() < CONTENT_IDENTITY_CENSUS_HEADER_LEN {
            return Err(format!(
                "content identity census payload has {} byte(s), fewer than header {CONTENT_IDENTITY_CENSUS_HEADER_LEN}",
                payload.len()
            ));
        }
        if &payload[..4] != CONTENT_IDENTITY_CENSUS_MAGIC {
            return Err("content identity census payload magic is invalid".to_string());
        }
        let version = u16::from_le_bytes([payload[4], payload[5]]);
        if version != CONTENT_IDENTITY_CENSUS_VERSION {
            return Err(format!(
                "content identity census payload version {version} is not {CONTENT_IDENTITY_CENSUS_VERSION}"
            ));
        }
        let header_len = usize::from(u16::from_le_bytes([payload[6], payload[7]]));
        if header_len != CONTENT_IDENTITY_CENSUS_HEADER_LEN {
            return Err(format!(
                "content identity census header length {header_len} is not {CONTENT_IDENTITY_CENSUS_HEADER_LEN}"
            ));
        }
        let status = u16::from_le_bytes([payload[8], payload[9]]);
        let unknown_status = status & !CONTENT_IDENTITY_STATUS_KNOWN_MASK;
        if unknown_status != 0 {
            return Err(format!(
                "content identity census status contains unknown bits {unknown_status:#06x}"
            ));
        }
        if u16::from_le_bytes([payload[10], payload[11]]) != 0
            || u64::from_le_bytes(payload[32..40].try_into().expect("fixed reserved slice")) != 0
        {
            return Err("content identity census reserved field is nonzero".to_string());
        }
        let building_count = read_u32(payload, 12) as usize;
        let resource_count = read_u32(payload, 16) as usize;
        let blob_len = read_u32(payload, 20) as usize;
        let buildings_declared = read_u32(payload, 24);
        let resources_declared = read_u32(payload, 28);
        for (count, namespace) in [
            (building_count, ContentIdentityNamespace::Building),
            (resource_count, ContentIdentityNamespace::StrategicResource),
        ] {
            if count > CONTENT_IDENTITY_CENSUS_MAX_ROWS_PER_NAMESPACE {
                return Err(format!(
                    "content identity census {} namespace declares {count} rows, more than {CONTENT_IDENTITY_CENSUS_MAX_ROWS_PER_NAMESPACE}",
                    namespace.canonical_name()
                ));
            }
        }
        if building_count as u64 > u64::from(buildings_declared)
            || resource_count as u64 > u64::from(resources_declared)
        {
            return Err(
                "content identity census carries more rows than the databases declared".to_string(),
            );
        }
        let rows_len = (building_count + resource_count)
            .checked_mul(CONTENT_IDENTITY_CENSUS_ROW_LEN)
            .ok_or_else(|| "content identity census row table overflows".to_string())?;
        let expected = CONTENT_IDENTITY_CENSUS_HEADER_LEN
            .checked_add(rows_len)
            .and_then(|value| value.checked_add(blob_len))
            .ok_or_else(|| "content identity census payload length overflows".to_string())?;
        if payload.len() != expected {
            return Err(format!(
                "content identity census payload is {} byte(s); its header describes {expected}",
                payload.len()
            ));
        }
        let blob = &payload[CONTENT_IDENTITY_CENSUS_HEADER_LEN + rows_len..];

        let mut cursor = CONTENT_IDENTITY_CENSUS_HEADER_LEN;
        let mut blob_cursor = 0usize;
        let blob_cursor = &mut blob_cursor;
        let mut read_rows = |count: usize,
                             namespace: ContentIdentityNamespace|
         -> Result<Vec<ContentIdentityRow>, String> {
            let mut rows = Vec::with_capacity(count);
            let mut previous: Option<u32> = None;
            for _ in 0..count {
                let token_id = read_u32(payload, cursor);
                let offset = read_u32(payload, cursor + 4) as usize;
                let len = usize::from(u16::from_le_bytes([
                    payload[cursor + 8],
                    payload[cursor + 9],
                ]));
                if u16::from_le_bytes([payload[cursor + 10], payload[cursor + 11]]) != 0 {
                    return Err("content identity census row reserved field is nonzero".to_string());
                }
                cursor += CONTENT_IDENTITY_CENSUS_ROW_LEN;
                if offset != *blob_cursor {
                    return Err(format!(
                        "content identity census {} row {token_id:#010x} starts at {offset} in its token blob, not at {}",
                        namespace.canonical_name(),
                        *blob_cursor
                    ));
                }
                let end = offset
                    .checked_add(len)
                    .ok_or_else(|| "content identity census row span overflows".to_string())?;
                *blob_cursor = end;
                if end > blob.len() {
                    return Err(format!(
                        "content identity census {} row {token_id:#010x} spans past its token blob",
                        namespace.canonical_name()
                    ));
                }
                let token = core::str::from_utf8(&blob[offset..end])
                    .map_err(|_| {
                        format!(
                            "content identity census {} row {token_id:#010x} token is not UTF-8",
                            namespace.canonical_name()
                        )
                    })?
                    .to_string();
                validate_token(&token, token_id)?;
                if let Some(previous) = previous {
                    if token_id <= previous {
                        return Err(format!(
                            "content identity census {} rows are not strictly increasing at id {token_id:#010x}",
                            namespace.canonical_name()
                        ));
                    }
                }
                previous = Some(token_id);
                rows.push(ContentIdentityRow { token_id, token });
            }
            Ok(rows)
        };
        let buildings = read_rows(building_count, ContentIdentityNamespace::Building)?;
        let resources = read_rows(resource_count, ContentIdentityNamespace::StrategicResource)?;
        if *blob_cursor != blob.len() {
            return Err(format!(
                "content identity census token blob is {} bytes; its rows name {}",
                blob.len(),
                *blob_cursor
            ));
        }
        identity_is_a_bijection(&buildings, &resources)?;

        Ok(Self {
            buildings,
            resources,
            buildings_declared,
            resources_declared,
            status,
        })
    }
}

pub fn validate_content_identity_census_placement(
    ordinary_records_before: u64,
    previous_record_type: Option<KnownRecordType>,
    already_seen_in_this_stream: bool,
) -> Result<(), &'static str> {
    if already_seen_in_this_stream {
        return Err("content_identity_census_already_seen_in_this_stream");
    }
    if ordinary_records_before == 0 {
        return Ok(());
    }
    match previous_record_type {
        // world record, which is the property that matters. (A release
        Some(KnownRecordType::FreezeCensus)
        | Some(KnownRecordType::FrameEnd)
        | Some(KnownRecordType::FrameRefused) => Ok(()),
        _ => Err("content_identity_census_is_not_at_a_frame_boundary"),
    }
}

fn read_u32(payload: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(
        payload[at..at + 4]
            .try_into()
            .expect("bounds checked by caller"),
    )
}

fn validate_token(token: &str, token_id: u32) -> Result<(), String> {
    if token_id == 0 {
        return Err("content identity census row has the zero token id".to_string());
    }
    if token.is_empty() {
        return Err(format!(
            "content identity census row {token_id:#010x} has an empty token"
        ));
    }
    if token.len() > CONTENT_IDENTITY_CENSUS_MAX_TOKEN_LEN {
        return Err(format!(
            "content identity census row {token_id:#010x} token is {} bytes, more than {CONTENT_IDENTITY_CENSUS_MAX_TOKEN_LEN}",
            token.len()
        ));
    }
    if token
        .bytes()
        .any(|byte| byte <= b' ' || byte == 0x7F || byte == b'"' || byte == b'{' || byte == b'}')
    {
        return Err(format!(
            "content identity census row {token_id:#010x} token is not a script token"
        ));
    }
    Ok(())
}

/// The id space is GLOBAL and the interner is a BIJECTION, so the census must
/// be one too.
///
/// Both directions matter and only one of them is obvious:
///
/// * one id, two texts - a building and a resource may legitimately share an id
///   only when they are the same interned token, so different texts under one
///   id means a database was misread;
/// * one text, two ids - `AddDynamicToken` returns the EXISTING id for text it
///   has already interned, so the engine can never have two ids for one string.
///   A census that carries two is a coherent misreading, and it is the
///   dangerous direction: a consumer counting by token would count both.
fn identity_is_a_bijection(
    buildings: &[ContentIdentityRow],
    resources: &[ContentIdentityRow],
) -> Result<(), String> {
    for resource in resources {
        if let Ok(index) = buildings.binary_search_by_key(&resource.token_id, |row| row.token_id) {
            if buildings[index].token != resource.token {
                return Err(format!(
                    "content identity census id {:#010x} is '{}' as a building and '{}' as a strategic resource",
                    resource.token_id, buildings[index].token, resource.token
                ));
            }
        }
    }
    let mut by_text: BTreeMap<&str, u32> = BTreeMap::new();
    for row in buildings.iter().chain(resources) {
        match by_text.get(row.token.as_str()) {
            Some(first) if *first != row.token_id => {
                return Err(format!(
                    "content identity census names '{}' twice, as {:#010x} and {:#010x}; the lexer interns one id per token",
                    row.token, first, row.token_id
                ))
            }
            _ => {
                by_text.insert(row.token.as_str(), row.token_id);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(token_id: u32, token: &str) -> ContentIdentityRow {
        ContentIdentityRow {
            token_id,
            token: token.to_string(),
        }
    }

    fn census() -> ContentIdentityCensusV1 {
        ContentIdentityCensusV1 {
            buildings: vec![
                row(0x4BF6, "dockyard"),
                row(0x4E22, "arms_factory"),
                row(0x4E2D, "industrial_complex"),
            ],
            resources: vec![row(0x3E03, "steel"), row(0x4E1F, "aluminium")],
            buildings_declared: 3,
            resources_declared: 2,
            status: 0,
        }
    }

    // kills: any encoder/decoder disagreement about the layout, and any change
    // that stops the census carrying what it was read for - the id, the text,
    // which database it came from, and how many definitions that database
    // declared.
    #[test]
    fn a_census_round_trips_with_both_namespaces_intact() {
        let original = census();
        let bytes = original.encode().expect("the census encodes");
        let decoded = ContentIdentityCensusV1::decode(&bytes).expect("the census decodes");
        assert_eq!(decoded, original);
        assert!(decoded.is_complete());
        assert_eq!(
            decoded.token(ContentIdentityNamespace::Building, 0x4E2D),
            Some("industrial_complex")
        );
        assert_eq!(
            decoded.token(ContentIdentityNamespace::StrategicResource, 0x3E03),
            Some("steel")
        );
        // The namespaces do not answer for each other.
        assert_eq!(
            decoded.token(ContentIdentityNamespace::Building, 0x3E03),
            None
        );
        assert_eq!(
            decoded.token(ContentIdentityNamespace::StrategicResource, 0x4E2D),
            None
        );
    }

    // kills: treating a census that lost rows as authoritative. Completeness is
    // what the factory derivation asks before it counts anything, so every way
    // of losing a definition has to make `is_complete` false - including the
    // one that sets no status bit at all, where the database declared more
    // definitions than the reader kept.
    #[test]
    fn every_way_of_losing_a_definition_makes_the_census_incomplete() {
        assert!(census().is_complete());
        for status in [
            CONTENT_IDENTITY_STATUS_BUILDINGS_UNREADABLE,
            CONTENT_IDENTITY_STATUS_RESOURCES_UNREADABLE,
            CONTENT_IDENTITY_STATUS_BOUND_TRUNCATED,
            CONTENT_IDENTITY_STATUS_CONFLICT_DROPPED,
        ] {
            let mut injured = census();
            injured.status = status;
            assert!(!injured.is_complete(), "status {status:#06x}");
            let bytes = injured.encode().expect("an injured census still encodes");
            let decoded = ContentIdentityCensusV1::decode(&bytes).expect("and still decodes");
            assert!(!decoded.is_complete());
            assert_eq!(decoded.status_names().len(), 1);
        }
        let mut short = census();
        short.buildings_declared = 4;
        assert!(!short.is_complete(), "three rows for four declarations");
        let bytes = short.encode().expect("it encodes");
        assert!(!ContentIdentityCensusV1::decode(&bytes)
            .expect("it decodes")
            .is_complete());
    }

    // kills: admitting a census whose rows contradict themselves or their own
    // header. Each case here is a way a misread engine object could otherwise
    // become a building name that replay displays as fact.
    #[test]
    fn a_contradictory_census_is_refused_rather_than_half_believed() {
        let mut unsorted = census();
        unsorted.buildings.swap(0, 1);
        assert!(unsorted.encode().is_err(), "rows must be sorted by id");

        let mut repeated = census();
        repeated.buildings[1] = row(0x4BF6, "arms_factory");
        assert!(repeated.encode().is_err(), "an id cannot repeat");

        let mut disagreeing = census();
        disagreeing.resources.push(row(0x4E2D, "steel_ore"));
        disagreeing.resources_declared = 3;
        let error = disagreeing.encode().expect_err("one id, two texts");
        assert!(error.contains("industrial_complex"), "{error}");

        let mut agreeing = census();
        agreeing.resources.push(row(0x4E2D, "industrial_complex"));
        agreeing.resources_declared = 3;
        agreeing.resources.sort_by_key(|row| row.token_id);
        assert!(
            agreeing.encode().is_ok(),
            "the same interned text under one id is not a contradiction"
        );

        for bad in ["", "two words", "quote\"", "brace{"] {
            let mut token = census();
            token.buildings[0] = row(0x4BF6, bad);
            assert!(token.encode().is_err(), "token {bad:?}");
        }
        let mut zero = census();
        zero.buildings[0] = row(0, "dockyard");
        assert!(zero.encode().is_err(), "the zero id is not a token");
    }

    #[test]
    fn the_decoder_refuses_a_payload_that_does_not_describe_itself() {
        let bytes = census().encode().expect("the census encodes");
        assert!(ContentIdentityCensusV1::decode(&bytes).is_ok());

        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(ContentIdentityCensusV1::decode(&trailing).is_err());

        let mut truncated = bytes.clone();
        truncated.pop();
        assert!(ContentIdentityCensusV1::decode(&truncated).is_err());

        let mut magic = bytes.clone();
        magic[0] = b'X';
        assert!(ContentIdentityCensusV1::decode(&magic).is_err());

        let mut version = bytes.clone();
        version[4] = 2;
        assert!(ContentIdentityCensusV1::decode(&version).is_err());

        let mut status = bytes.clone();
        status[8] = 0x80;
        assert!(ContentIdentityCensusV1::decode(&status).is_err());

        let mut reserved = bytes.clone();
        reserved[10] = 1;
        assert!(ContentIdentityCensusV1::decode(&reserved).is_err());

        let mut over_declared = bytes.clone();
        over_declared[24..28].copy_from_slice(&2u32.to_le_bytes());
        assert!(ContentIdentityCensusV1::decode(&over_declared).is_err());

        let mut span = bytes.clone();
        let first_row_len_at = CONTENT_IDENTITY_CENSUS_HEADER_LEN + 8;
        span[first_row_len_at..first_row_len_at + 2].copy_from_slice(&4_096u16.to_le_bytes());
        assert!(ContentIdentityCensusV1::decode(&span).is_err());

        assert!(ContentIdentityCensusV1::decode(&[]).is_err());
        assert!(ContentIdentityCensusV1::decode(&vec![
            0;
            CONTENT_IDENTITY_CENSUS_MAX_PAYLOAD_LEN + 1
        ])
        .is_err());
    }

    #[test]
    fn one_token_may_not_carry_two_ids() {
        let mut twice = census();
        twice.buildings.push(row(0x9000, "arms_factory"));
        twice.buildings_declared = 4;
        let error = twice.encode().expect_err("one text, two ids");
        assert!(error.contains("arms_factory"), "{error}");

        let mut across = census();
        across.resources.push(row(0x9001, "dockyard"));
        across.resources_declared = 3;
        assert!(across.encode().is_err());

        let mut shared = census();
        shared.resources.push(row(0x4BF6, "dockyard"));
        shared.resources_declared = 3;
        shared.resources.sort_by_key(|row| row.token_id);
        assert!(shared.encode().is_ok());
    }

    #[test]
    fn a_row_must_name_the_exact_span_the_encoder_wrote() {
        let bytes = census().encode().expect("the census encodes");
        let second_row_offset_at =
            CONTENT_IDENTITY_CENSUS_HEADER_LEN + CONTENT_IDENTITY_CENSUS_ROW_LEN + 4;

        let mut aliased = bytes.clone();
        aliased[second_row_offset_at..second_row_offset_at + 4]
            .copy_from_slice(&0u32.to_le_bytes());
        let error = ContentIdentityCensusV1::decode(&aliased)
            .expect_err("an aliased span is not this row's token");
        assert!(error.contains("token blob"), "{error}");

        let mut hole = bytes.clone();
        let blob_len_at = 20;
        let blob_len = u32::from_le_bytes(hole[blob_len_at..blob_len_at + 4].try_into().unwrap());
        hole[blob_len_at..blob_len_at + 4].copy_from_slice(&(blob_len + 1).to_le_bytes());
        hole.push(b'x');
        assert!(ContentIdentityCensusV1::decode(&hole).is_err());
    }

    #[test]
    fn the_census_sits_first_or_behind_a_closed_hour_and_only_once() {
        assert_eq!(
            validate_content_identity_census_placement(0, None, false),
            Ok(())
        );
        assert_eq!(
            validate_content_identity_census_placement(
                1,
                Some(KnownRecordType::FreezeCensus),
                false
            ),
            Ok(())
        );
        for closed in [KnownRecordType::FrameEnd, KnownRecordType::FrameRefused] {
            assert_eq!(
                validate_content_identity_census_placement(400, Some(closed), false),
                Ok(())
            );
        }
        for open in [
            KnownRecordType::DomainData,
            KnownRecordType::DomainPerf,
            KnownRecordType::DomainGap,
            KnownRecordType::FrameDegraded,
        ] {
            assert_eq!(
                validate_content_identity_census_placement(400, Some(open), false),
                Err("content_identity_census_is_not_at_a_frame_boundary"),
                "{open:?}"
            );
        }
        assert_eq!(
            validate_content_identity_census_placement(400, Some(KnownRecordType::FrameEnd), true),
            Err("content_identity_census_already_seen_in_this_stream")
        );
        assert_eq!(
            validate_content_identity_census_placement(0, None, true),
            Err("content_identity_census_already_seen_in_this_stream")
        );
    }

    #[test]
    fn a_namespace_past_its_bound_is_refused_by_the_encoder() {
        let mut huge = census();
        huge.buildings = (1..=CONTENT_IDENTITY_CENSUS_MAX_ROWS_PER_NAMESPACE as u32 + 1)
            .map(|id| row(id, "b"))
            .collect();
        huge.buildings_declared = huge.buildings.len() as u32;
        assert!(huge.encode().is_err());

        let mut long_token = census();
        long_token.buildings[0] = row(
            0x4BF6,
            &"b".repeat(CONTENT_IDENTITY_CENSUS_MAX_TOKEN_LEN + 1),
        );
        assert!(long_token.encode().is_err());
    }
}
