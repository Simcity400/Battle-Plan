use std::collections::BTreeSet;

pub const HEADER_LEN: usize = 32;
pub const COUNTRY_LEN: usize = 24;
pub const SLOT_LEN: usize = 16;
pub const MAX_COUNTRIES: usize = 65_536;
pub const MAX_BLOCK_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum Kind { Division = 1, Fleet = 2, Railway = 3 }

impl Kind {
    fn max_country_slots(self) -> usize {
        match self { Self::Division => 20_000, Self::Fleet | Self::Railway => 10_000 }
    }
    fn max_total_slots(self) -> usize {
        match self { Self::Division | Self::Railway => 20_000, Self::Fleet => 250_000 }
    }
    pub fn accepts_reference(self, reference: MemberRef) -> bool {
        match self {
            Self::Division => reference.id != 0 && matches!(reference.native_type, 4713 | 51),
            Self::Fleet => reference.id != 0 && reference.native_type == 61,
            Self::Railway => reference.native_type != 0 || reference.id != 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Cause {
    CountrySlotUnreadable = 1,
    CountryClassInvalid = 2,
    OwnerIdentityUnavailable = 3,
    HeaderUnreadable = 4,
    HeaderChanged = 5,
    CountExceedsCapacity = 6,
    BoundExceeded = 7,
    VectorUnreadable = 8,
    VectorChanged = 9,
    SlotPointerInvalid = 10,
    WrapperClassInvalid = 11,
    BodyClassInvalid = 12,
    IdentityInvalid = 13,
    DuplicateIdentity = 14,
    AliasedBody = 15,
    OwnerIdentityChanged = 16,
    ClockUnavailable = 17,
    TickChanged = 18,
    HourMismatch = 19,
    ExternalBridge = 20,
    RailwayNotRead = 21,
    RailwayAddressOverflow = 22,
    RailwayNegativeCapacity = 23,
    RailwayNegativeCount = 24,
    RailwayCountExceedsCapacity = 25,
    RailwayCountryBound = 26,
    RailwaySlotBound = 27,
    RailwayReadBound = 28,
    RailwayRetainedBound = 29,
    RailwayAllocationFailed = 30,
    RailwayPrimaryVtable = 31,
    RailwayClass = 32,
    RailwayReferenceVtable = 33,
    RailwayOwnerMismatch = 34,
    RailwayUninitializedReference = 35,
    RailwayNullRegistry = 36,
    RailwayMissingKey = 37,
    RailwaySentinel = 38,
    RailwayNullPayload = 39,
    RailwayDifferentPayload = 40,
    RailwayProbeBound = 41,
    RailwayDuplicatePointer = 42,
    RailwayDuplicateReference = 43,
    RailwayNullPointerHeaderPointer = 100,
    RailwayNullPointerHeaderCapacity = 101,
    RailwayNullPointerHeaderCount = 102,
    RailwayNullPointerVectorSlot = 103,
    RailwayNullPointerPrimaryVtable = 104,
    RailwayNullPointerClass = 105,
    RailwayNullPointerReferenceVtable = 106,
    RailwayNullPointerReference = 107,
    RailwayNullPointerOwner = 108,
    RailwayNullPointerRegistryRoot = 109,
    RailwayNullPointerRegistrySlots = 110,
    RailwayNullPointerRegistryMask = 111,
    RailwayNullPointerRegistryExtra = 112,
    RailwayNullPointerProbeDistance = 113,
    RailwayNullPointerProbeId = 114,
    RailwayNullPointerProbeType = 115,
    RailwayNullPointerPayload = 116,
    RailwayUnreadableHeaderPointer = 200,
    RailwayUnreadableHeaderCapacity = 201,
    RailwayUnreadableHeaderCount = 202,
    RailwayUnreadableVectorSlot = 203,
    RailwayUnreadablePrimaryVtable = 204,
    RailwayUnreadableClass = 205,
    RailwayUnreadableReferenceVtable = 206,
    RailwayUnreadableReference = 207,
    RailwayUnreadableOwner = 208,
    RailwayUnreadableRegistryRoot = 209,
    RailwayUnreadableRegistrySlots = 210,
    RailwayUnreadableRegistryMask = 211,
    RailwayUnreadableRegistryExtra = 212,
    RailwayUnreadableProbeDistance = 213,
    RailwayUnreadableProbeId = 214,
    RailwayUnreadableProbeType = 215,
    RailwayUnreadablePayload = 216,
    RailwayChangedHeaderPointer = 300,
    RailwayChangedHeaderCapacity = 301,
    RailwayChangedHeaderCount = 302,
    RailwayChangedVectorSlot = 303,
    RailwayChangedPrimaryVtable = 304,
    RailwayChangedClass = 305,
    RailwayChangedReferenceVtable = 306,
    RailwayChangedReference = 307,
    RailwayChangedOwner = 308,
    RailwayChangedRegistryRoot = 309,
    RailwayChangedRegistrySlots = 310,
    RailwayChangedRegistryMask = 311,
    RailwayChangedRegistryExtra = 312,
    RailwayChangedProbeDistance = 313,
    RailwayChangedProbeId = 314,
    RailwayChangedProbeType = 315,
    RailwayChangedPayload = 316,
}

impl Cause {
    pub fn valid_for(self, kind: Kind) -> bool {
        self as u32 <= 20 || kind == Kind::Railway
    }

    pub fn from_word(raw: u32) -> Result<Option<Self>, String> {
        Ok(match raw {
            0 => None,
            1 => Some(Self::CountrySlotUnreadable), 2 => Some(Self::CountryClassInvalid),
            3 => Some(Self::OwnerIdentityUnavailable), 4 => Some(Self::HeaderUnreadable),
            5 => Some(Self::HeaderChanged), 6 => Some(Self::CountExceedsCapacity),
            7 => Some(Self::BoundExceeded), 8 => Some(Self::VectorUnreadable),
            9 => Some(Self::VectorChanged), 10 => Some(Self::SlotPointerInvalid),
            11 => Some(Self::WrapperClassInvalid), 12 => Some(Self::BodyClassInvalid),
            13 => Some(Self::IdentityInvalid), 14 => Some(Self::DuplicateIdentity),
            15 => Some(Self::AliasedBody), 16 => Some(Self::OwnerIdentityChanged),
            17 => Some(Self::ClockUnavailable), 18 => Some(Self::TickChanged),
            19 => Some(Self::HourMismatch), 20 => Some(Self::ExternalBridge),
            21 => Some(Self::RailwayNotRead),
            22 => Some(Self::RailwayAddressOverflow),
            23 => Some(Self::RailwayNegativeCapacity),
            24 => Some(Self::RailwayNegativeCount),
            25 => Some(Self::RailwayCountExceedsCapacity),
            26 => Some(Self::RailwayCountryBound),
            27 => Some(Self::RailwaySlotBound),
            28 => Some(Self::RailwayReadBound),
            29 => Some(Self::RailwayRetainedBound),
            30 => Some(Self::RailwayAllocationFailed),
            31 => Some(Self::RailwayPrimaryVtable),
            32 => Some(Self::RailwayClass),
            33 => Some(Self::RailwayReferenceVtable),
            34 => Some(Self::RailwayOwnerMismatch),
            35 => Some(Self::RailwayUninitializedReference),
            36 => Some(Self::RailwayNullRegistry),
            37 => Some(Self::RailwayMissingKey),
            38 => Some(Self::RailwaySentinel),
            39 => Some(Self::RailwayNullPayload),
            40 => Some(Self::RailwayDifferentPayload),
            41 => Some(Self::RailwayProbeBound),
            42 => Some(Self::RailwayDuplicatePointer),
            43 => Some(Self::RailwayDuplicateReference),
            100 => Some(Self::RailwayNullPointerHeaderPointer),
            101 => Some(Self::RailwayNullPointerHeaderCapacity),
            102 => Some(Self::RailwayNullPointerHeaderCount),
            103 => Some(Self::RailwayNullPointerVectorSlot),
            104 => Some(Self::RailwayNullPointerPrimaryVtable),
            105 => Some(Self::RailwayNullPointerClass),
            106 => Some(Self::RailwayNullPointerReferenceVtable),
            107 => Some(Self::RailwayNullPointerReference),
            108 => Some(Self::RailwayNullPointerOwner),
            109 => Some(Self::RailwayNullPointerRegistryRoot),
            110 => Some(Self::RailwayNullPointerRegistrySlots),
            111 => Some(Self::RailwayNullPointerRegistryMask),
            112 => Some(Self::RailwayNullPointerRegistryExtra),
            113 => Some(Self::RailwayNullPointerProbeDistance),
            114 => Some(Self::RailwayNullPointerProbeId),
            115 => Some(Self::RailwayNullPointerProbeType),
            116 => Some(Self::RailwayNullPointerPayload),
            200 => Some(Self::RailwayUnreadableHeaderPointer),
            201 => Some(Self::RailwayUnreadableHeaderCapacity),
            202 => Some(Self::RailwayUnreadableHeaderCount),
            203 => Some(Self::RailwayUnreadableVectorSlot),
            204 => Some(Self::RailwayUnreadablePrimaryVtable),
            205 => Some(Self::RailwayUnreadableClass),
            206 => Some(Self::RailwayUnreadableReferenceVtable),
            207 => Some(Self::RailwayUnreadableReference),
            208 => Some(Self::RailwayUnreadableOwner),
            209 => Some(Self::RailwayUnreadableRegistryRoot),
            210 => Some(Self::RailwayUnreadableRegistrySlots),
            211 => Some(Self::RailwayUnreadableRegistryMask),
            212 => Some(Self::RailwayUnreadableRegistryExtra),
            213 => Some(Self::RailwayUnreadableProbeDistance),
            214 => Some(Self::RailwayUnreadableProbeId),
            215 => Some(Self::RailwayUnreadableProbeType),
            216 => Some(Self::RailwayUnreadablePayload),
            300 => Some(Self::RailwayChangedHeaderPointer),
            301 => Some(Self::RailwayChangedHeaderCapacity),
            302 => Some(Self::RailwayChangedHeaderCount),
            303 => Some(Self::RailwayChangedVectorSlot),
            304 => Some(Self::RailwayChangedPrimaryVtable),
            305 => Some(Self::RailwayChangedClass),
            306 => Some(Self::RailwayChangedReferenceVtable),
            307 => Some(Self::RailwayChangedReference),
            308 => Some(Self::RailwayChangedOwner),
            309 => Some(Self::RailwayChangedRegistryRoot),
            310 => Some(Self::RailwayChangedRegistrySlots),
            311 => Some(Self::RailwayChangedRegistryMask),
            312 => Some(Self::RailwayChangedRegistryExtra),
            313 => Some(Self::RailwayChangedProbeDistance),
            314 => Some(Self::RailwayChangedProbeId),
            315 => Some(Self::RailwayChangedProbeType),
            316 => Some(Self::RailwayChangedPayload),
            _ => return Err("CRO1 unknown refusal cause".into()),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Acquisition { Uncovered = 0, Bracketed = 1, Refused = 2 }

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct MemberRef {
    pub native_type: u32,
    pub id: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Slot {
    pub ordinal: u32,
    pub reference: Result<MemberRef, Cause>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Country {
    pub physical_slot: u32,
    pub owner_tag_db_id: Option<u32>,
    pub acquisition: Acquisition,
    pub observed_count: Option<u32>,
    pub failure: Option<Cause>,
    pub slots: Vec<Slot>,
}

impl Country {
    pub fn complete(&self) -> bool {
        self.owner_tag_db_id.is_some()
            && self.acquisition == Acquisition::Bracketed
            && self.failure.is_none()
            && self.observed_count == u32::try_from(self.slots.len()).ok()
            && self.slots.iter().all(|slot| slot.reference.is_ok())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Observation {
    pub kind: Kind,
    pub country_count: Option<u32>,
    pub failure: Option<Cause>,
    pub countries: Vec<Country>,
}

impl Observation {
    pub fn unavailable(kind: Kind, cause: Cause) -> Self {
        Self { kind, country_count: None, failure: Some(cause), countries: Vec::new() }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.failure.is_some_and(|cause| !cause.valid_for(self.kind)) {
            return Err("CRO1 refusal kind".into());
        }
        if self.countries.len() > MAX_COUNTRIES
            || self.country_count.is_some_and(|count| count as usize > MAX_COUNTRIES) {
            return Err("CRO1 country bound".into());
        }
        match self.country_count {
            None if self.failure.is_none() || !self.countries.is_empty() =>
                return Err("CRO1 unavailable country census contradiction".into()),
            Some(count) if count as usize != self.countries.len() =>
                return Err("CRO1 physical country cardinality".into()),
            _ => {}
        }
        let mut owners = BTreeSet::new();
        let mut identities = BTreeSet::new();
        let mut total = 0usize;
        for (index, country) in self.countries.iter().enumerate() {
            if country.failure.is_some_and(|cause| !cause.valid_for(self.kind)) {
                return Err("CRO1 country refusal kind".into());
            }
            if country.physical_slot as usize != index {
                return Err("CRO1 physical country order".into());
            }
            if let Some(owner) = country.owner_tag_db_id {
                if owner == 0 || !owners.insert(owner) {
                    return Err("CRO1 ambiguous owner identity".into());
                }
            }
            if (country.owner_tag_db_id.is_none()
                || country.acquisition != Acquisition::Bracketed
                || country.observed_count.is_none()) && country.failure.is_none() {
                return Err("CRO1 unavailable authority without cause".into());
            }
            match country.observed_count {
                None if !country.slots.is_empty() =>
                    return Err("CRO1 unknown count with slots".into()),
                Some(count) if count as usize != country.slots.len() =>
                    if country.failure.is_none() || !country.slots.is_empty() {
                        return Err("CRO1 slot cardinality".into());
                    },
                _ => {}
            }
            if country.slots.len() > self.kind.max_country_slots() {
                return Err("CRO1 per-country slot bound".into());
            }
            total = total.checked_add(country.slots.len()).ok_or("CRO1 slot overflow")?;
            if total > self.kind.max_total_slots() { return Err("CRO1 total slot bound".into()); }
            for (ordinal, slot) in country.slots.iter().enumerate() {
                if slot.ordinal as usize != ordinal { return Err("CRO1 slot order".into()); }
                if slot.reference.is_err_and(|cause| !cause.valid_for(self.kind)) {
                    return Err("CRO1 slot refusal kind".into());
                }
                if let Ok(reference) = slot.reference {
                    if !self.kind.accepts_reference(reference)
                        || !identities.insert(reference) {
                        return Err("CRO1 ambiguous member identity".into());
                    }
                }
            }
        }
        let bytes = HEADER_LEN.checked_add(self.countries.len().checked_mul(COUNTRY_LEN)
            .ok_or("CRO1 size overflow")?).and_then(|n| n.checked_add(total.checked_mul(SLOT_LEN)?))
            .ok_or("CRO1 size overflow")?;
        if bytes > MAX_BLOCK_BYTES { return Err("CRO1 byte bound".into()); }
        Ok(())
    }

    pub fn encode(&self) -> Result<Vec<u8>, String> {
        self.validate()?;
        let total = self.countries.iter().map(|country| country.slots.len()).sum::<usize>();
        let len = HEADER_LEN + self.countries.len() * COUNTRY_LEN + total * SLOT_LEN;
        let mut out = Vec::with_capacity(len);
        out.extend_from_slice(b"CRO1");
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&(self.kind as u16).to_le_bytes());
        for word in [len as u32, self.country_count.unwrap_or(0),
            self.failure.map_or(0, |cause| cause as u32), self.countries.len() as u32,
            total as u32, u32::from(self.country_count.is_some())] { out.extend_from_slice(&word.to_le_bytes()); }
        for country in &self.countries {
            out.extend_from_slice(&country.physical_slot.to_le_bytes());
            out.extend_from_slice(&country.owner_tag_db_id.unwrap_or(0).to_le_bytes());
            out.extend_from_slice(&[u8::from(country.owner_tag_db_id.is_some()), country.acquisition as u8, u8::from(country.observed_count.is_some()), 0]);
            for word in [country.observed_count.unwrap_or(0), country.slots.len() as u32,
                country.failure.map_or(0, |cause| cause as u32)] {
                out.extend_from_slice(&word.to_le_bytes());
            }
            for slot in &country.slots {
                let (kind, id, cause) = match slot.reference {
                    Ok(reference) => (reference.native_type, reference.id, 0),
                    Err(cause) => (0, 0, cause as u32),
                };
                for word in [slot.ordinal, kind, id, cause] { out.extend_from_slice(&word.to_le_bytes()); }
            }
        }
        Ok(out)
    }

    pub fn parse_prefix(bytes: &[u8], expected: Kind) -> Result<(Self, usize), String> {
        let header = bytes.get(..HEADER_LEN).ok_or("CRO1 truncated header")?;
        let len = u32::from_le_bytes(header[8..12].try_into().unwrap()) as usize;
        if !(HEADER_LEN..=MAX_BLOCK_BYTES).contains(&len) { return Err("CRO1 block bound".into()); }
        let block = bytes.get(..len).ok_or("CRO1 truncated block")?;
        Ok((Self::parse(block, expected)?, len))
    }

    pub fn parse(bytes: &[u8], expected: Kind) -> Result<Self, String> {
        if bytes.len() < HEADER_LEN || bytes.len() > MAX_BLOCK_BYTES || &bytes[..4] != b"CRO1"
            || u16::from_le_bytes(bytes[4..6].try_into().unwrap()) != 1
            || u16::from_le_bytes(bytes[6..8].try_into().unwrap()) != expected as u16 {
            return Err("CRO1 header/schema/kind".into());
        }
        let word = |offset: usize| u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
        let country_count = match word(28) {
            0 if word(12) == 0 => None,
            1 => Some(word(12)),
            _ => return Err("CRO1 country count presence".into()),
        };
        let countries_len = word(20) as usize;
        let total = word(24) as usize;
        let expected_len = HEADER_LEN.checked_add(countries_len.checked_mul(COUNTRY_LEN)
            .ok_or("CRO1 size overflow")?).and_then(|n| n.checked_add(total.checked_mul(SLOT_LEN)?))
            .ok_or("CRO1 size overflow")?;
        if word(8) as usize != bytes.len() || expected_len != bytes.len()
            || countries_len > MAX_COUNTRIES || total > expected.max_total_slots() {
            return Err("CRO1 length/bound/reserved".into());
        }
        let failure = Cause::from_word(word(16))?;
        let mut countries = Vec::with_capacity(countries_len);
        let mut cursor = HEADER_LEN;
        let mut consumed = 0usize;
        for _ in 0..countries_len {
            let end = cursor.checked_add(COUNTRY_LEN).filter(|end| *end <= bytes.len())
                .ok_or("CRO1 truncated country")?;
            let owner = word(cursor + 4);
            let owner_tag_db_id = match bytes[cursor + 8] {
                0 if owner == 0 => None,
                1 if owner != 0 => Some(owner),
                _ => return Err("CRO1 owner presence".into()),
            };
            let acquisition = match bytes[cursor + 9] {
                0 => Acquisition::Uncovered, 1 => Acquisition::Bracketed, 2 => Acquisition::Refused,
                _ => return Err("CRO1 acquisition".into()),
            };
            if bytes[cursor + 10] > 1 || bytes[cursor + 11] != 0 { return Err("CRO1 reserved".into()); }
            let count = match bytes[cursor + 10] {
                0 if word(cursor + 12) == 0 => None,
                1 => Some(word(cursor + 12)),
                _ => return Err("CRO1 slot count presence".into()),
            };
            let slot_count = word(cursor + 16) as usize;
            consumed = consumed.checked_add(slot_count).ok_or("CRO1 slot overflow")?;
            let slot_end = end.checked_add(slot_count.checked_mul(SLOT_LEN).ok_or("CRO1 slot overflow")?)
                .filter(|end| *end <= bytes.len()).ok_or("CRO1 truncated slots")?;
            if slot_count > expected.max_country_slots() || consumed > total {
                return Err("CRO1 slot bound".into());
            }
            let mut country = Country { physical_slot: word(cursor), owner_tag_db_id, acquisition,
                observed_count: count,
                failure: Cause::from_word(word(cursor + 20))?, slots: Vec::with_capacity(slot_count) };
            cursor = end;
            while cursor < slot_end {
                let kind = word(cursor + 4);
                let id = word(cursor + 8);
                let reference = match Cause::from_word(word(cursor + 12))? {
                    None => Ok(MemberRef { native_type: kind, id }),
                    Some(cause) if kind == 0 && id == 0 => Err(cause),
                    _ => return Err("CRO1 refused slot has identity".into()),
                };
                country.slots.push(Slot { ordinal: word(cursor), reference });
                cursor += SLOT_LEN;
            }
            countries.push(country);
        }
        if cursor != bytes.len() || consumed != total { return Err("CRO1 trailing/cardinality".into()); }
        let observation = Self { kind: expected, country_count, failure, countries };
        observation.validate()?;
        Ok(observation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn country(owner: u32, ids: &[u32]) -> Country {
        Country {
            physical_slot: 0, owner_tag_db_id: Some(owner), acquisition: Acquisition::Bracketed,
            observed_count: Some(ids.len() as u32), failure: None,
            slots: ids.iter().enumerate().map(|(i, id)| Slot {
                ordinal: i as u32, reference: Ok(MemberRef { native_type: 4713, id: *id }),
            }).collect(),
        }
    }
    fn observed(country: Country) -> Observation {
        Observation { kind: Kind::Division, country_count: Some(1), failure: None, countries: vec![country] }
    }

    #[test]
    fn order_empty_unknown_and_slot_failure_roundtrip_distinctly() {
        let ordered = observed(country(7, &[9, 2]));
        let changed = observed(country(7, &[2, 9]));
        assert_ne!(ordered.encode().unwrap(), changed.encode().unwrap());
        for value in [&ordered, &changed] {
            let decoded = Observation::parse(&value.encode().unwrap(), Kind::Division).unwrap();
            assert_eq!(&decoded, value);
            assert!(decoded.countries[0].complete());
        }
        let empty = observed(country(7, &[]));
        assert!(Observation::parse(&empty.encode().unwrap(), Kind::Division).unwrap().countries[0].complete());
        let unknown = Observation::unavailable(Kind::Division, Cause::HeaderUnreadable);
        assert_ne!(unknown.encode().unwrap(), empty.encode().unwrap());
        assert_eq!(Observation::parse(&unknown.encode().unwrap(), Kind::Division).unwrap(), unknown);
        let mut partial = country(7, &[9, 2]);
        partial.slots[1].reference = Err(Cause::IdentityInvalid);
        let partial = observed(partial);
        let decoded = Observation::parse(&partial.encode().unwrap(), Kind::Division).unwrap();
        assert_eq!(decoded, partial);
        assert!(!decoded.countries[0].complete());
        assert_eq!(decoded.countries[0].observed_count, Some(2));
    }

    #[test]
    fn strict_bytes_reject_truncation_unknown_flags_and_certified_duplicates() {
        let bytes = observed(country(7, &[9, 2])).encode().unwrap();
        for end in 0..bytes.len() {
            assert!(Observation::parse(&bytes[..end], Kind::Division).is_err(), "{end}");
        }
        for offset in [4usize, 6, 28, HEADER_LEN + 8, HEADER_LEN + 9, HEADER_LEN + 11] {
            let mut bad = bytes.clone();
            bad[offset] = 255;
            assert!(Observation::parse(&bad, Kind::Division).is_err(), "{offset}");
        }
        let mut bad = observed(country(7, &[9, 9]));
        assert!(bad.encode().is_err());
        bad.countries[0].slots[1].reference = Err(Cause::DuplicateIdentity);
        assert!(bad.encode().is_ok());
        bad.countries[0].slots[0].reference = Ok(MemberRef { native_type: 13, id: 9 });
        assert!(bad.encode().is_err());
        let mut refused = observed(country(7, &[9]));
        refused.countries[0].slots[0].reference = Err(Cause::VectorChanged);
        let mut bytes = refused.encode().unwrap();
        bytes[HEADER_LEN + COUNTRY_LEN + 8] = 9;
        assert!(Observation::parse(&bytes, Kind::Division).is_err());
    }

    #[test]
    fn unavailable_count_does_not_consume_a_raw_unsigned_value() {
        let mut value = observed(country(7, &[]));
        value.countries[0].observed_count = Some(u32::MAX);
        value.countries[0].failure = Some(Cause::BoundExceeded);
        let bytes = value.encode().unwrap();
        assert_eq!(Observation::parse(&bytes, Kind::Division).unwrap(), value);
        value.countries[0].observed_count = None;
        assert_ne!(value.encode().unwrap(), bytes);
        assert_eq!(Observation::parse(&value.encode().unwrap(), Kind::Division).unwrap(), value);
    }

    #[test]
    fn physical_country_and_aggregate_slot_limits_are_actual_limits() {
        let mut value = Observation {
            kind: Kind::Division, country_count: Some(MAX_COUNTRIES as u32), failure: None,
            countries: (0..MAX_COUNTRIES).map(|i| Country {
                physical_slot: i as u32, owner_tag_db_id: None, acquisition: Acquisition::Uncovered,
                observed_count: None, failure: Some(Cause::CountrySlotUnreadable), slots: Vec::new(),
            }).collect(),
        };
        assert_eq!(Observation::parse(&value.encode().unwrap(), Kind::Division).unwrap(), value);
        value.countries.push(value.countries[0].clone());
        value.country_count = Some(MAX_COUNTRIES as u32 + 1);
        assert!(value.encode().is_err());

        let mut fleets = Observation { kind: Kind::Fleet, country_count: Some(25), failure: None,
            countries: (0..25u32).map(|owner| Country {
                physical_slot: owner, owner_tag_db_id: Some(owner + 1), acquisition: Acquisition::Bracketed,
                observed_count: Some(10_000), failure: None,
                slots: (0..10_000u32).map(|ordinal| Slot {
                    ordinal, reference: Ok(MemberRef { native_type: 61, id: owner * 10_000 + ordinal + 1 }),
                }).collect(),
            }).collect(),
        };
        assert_eq!(Observation::parse(&fleets.encode().unwrap(), Kind::Fleet).unwrap(), fleets);
        fleets.country_count = Some(26);
        fleets.countries.push(Country {
            physical_slot: 25, owner_tag_db_id: Some(26), acquisition: Acquisition::Bracketed,
            observed_count: Some(1), failure: None,
            slots: vec![Slot { ordinal: 0, reference: Ok(MemberRef { native_type: 61, id: 250_001 }) }],
        });
        assert!(fleets.encode().is_err());
        assert!(Observation::parse(&vec![0; MAX_BLOCK_BYTES + 1], Kind::Fleet).is_err());
    }
}

#[cfg(test)]
mod railway_tests {
    use super::*;
    fn country(physical: u32, start: u32, count: u32) -> Country {
        Country { physical_slot: physical, owner_tag_db_id: Some(physical+1),
            acquisition: Acquisition::Bracketed, observed_count: Some(count), failure: None,
            slots: (0..count).map(|ordinal| Slot { ordinal,
                reference: Ok(MemberRef { native_type: 0, id: start+ordinal }) }).collect() }
    }
    fn observed(countries: Vec<Country>) -> Observation {
        Observation { kind: Kind::Railway, country_count: Some(countries.len() as u32), failure: None, countries }
    }
    #[test]
    fn railway_all_reference_bits_and_local_refusals_roundtrip_without_division_whitelist() {
        let mut row=country(0,1,4);
        let refs=[MemberRef { native_type:0,id:u32::MAX },MemberRef { native_type:u32::MAX,id:0 },
            MemberRef { native_type:51,id:9 },MemberRef { native_type:4713,id:1 }];
        for (slot,reference) in row.slots.iter_mut().zip(refs) { slot.reference=Ok(reference); }
        let value=observed(vec![row]);let bytes=value.encode().unwrap();
        assert_eq!(Observation::parse(&bytes,Kind::Railway).unwrap(),value);
        assert!(Observation::parse(&bytes,Kind::Division).is_err());
        let mut bad=value.clone();bad.countries[0].slots[0].reference=Ok(MemberRef {native_type:0,id:0});
        assert!(bad.encode().is_err());
        for raw in 1..=400 {
            if let Ok(Some(cause))=Cause::from_word(raw) {
                let mut changed=value.clone();changed.countries[0].slots[0].reference=Err(cause);
                assert_eq!(Observation::parse(&changed.encode().unwrap(),Kind::Railway).unwrap(),changed);
                if raw>20 { assert!(!cause.valid_for(Kind::Division));assert!(!cause.valid_for(Kind::Fleet)); }
            }
        }
    }
    #[test]
    fn railway_actual_per_owner_and_total_slot_bounds_do_not_certify_truncation() {
        let mut value=observed(vec![country(0,1,10_000),country(1,10_001,10_000)]);
        let bytes=value.encode().unwrap();assert_eq!(Observation::parse(&bytes,Kind::Railway).unwrap(),value);
        value.countries.push(country(2,20_001,1));value.country_count=Some(3);
        assert!(value.encode().unwrap_err().contains("total slot bound"));
        let value=observed(vec![country(0,1,10_001)]);
        assert!(value.encode().unwrap_err().contains("per-country slot bound"));
        for value in [observed(vec![country(0,1,0)]),Observation::unavailable(Kind::Railway,Cause::RailwayNegativeCount)] {
            assert_eq!(Observation::parse(&value.encode().unwrap(),Kind::Railway).unwrap(),value);
        }
    }
}
