use std::collections::BTreeSet;

pub const HEADER_LEN: usize = 32;
pub const COUNTRY_LEN: usize = 24;
pub const SLOT_LEN: usize = 16;
pub const MAX_COUNTRIES: usize = 65_536;
pub const MAX_BLOCK_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum Kind { Division = 1, Fleet = 2 }

impl Kind {
    fn max_country_slots(self) -> usize {
        match self { Self::Division => 20_000, Self::Fleet => 10_000 }
    }
    fn max_total_slots(self) -> usize {
        match self { Self::Division => 20_000, Self::Fleet => 250_000 }
    }
    fn accepts(self, native_type: u32) -> bool {
        match self { Self::Division => matches!(native_type, 4713 | 51), Self::Fleet => native_type == 61 }
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
}

impl Cause {
    fn from_word(raw: u32) -> Result<Option<Self>, String> {
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
                if let Ok(reference) = slot.reference {
                    if reference.id == 0 || !self.kind.accepts(reference.native_type)
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
