//! D22's current, ordered province occupants. No renderer or lifetime identity.
pub const MAGIC: &[u8; 4] = b"POC1";
pub const HEADER_LEN: usize = 48;
pub const PROVINCE_LEN: usize = 24;
pub const VECTOR_LEN: usize = 32;
pub const SLOT_LEN: usize = 24;
pub const MAX_PROVINCES: usize = 65_536;
pub const MAX_SLOTS: usize = 250_000;
pub const MAX_BYTES: usize = 16 * 1024 * 1024;
pub const MAP_HEADER_LEN: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Family {
    Mixed = 0,
    Class0 = 1,
    Class1 = 2,
    Class13 = 3,
}
impl Family {
    pub const ALL: [Self; 4] = [Self::Mixed, Self::Class0, Self::Class1, Self::Class13];
    pub fn from_raw(v: u8) -> Result<Self, String> {
        Self::ALL
            .get(v as usize)
            .copied()
            .ok_or_else(|| error("family"))
    }
    pub fn class(self) -> Option<i32> {
        match self {
            Self::Mixed => None,
            Self::Class0 => Some(0),
            Self::Class1 => Some(1),
            Self::Class13 => Some(13),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Status {
    Measured = 1,
    Partial = 2,
    Failed = 3,
    NotScheduled = 4,
    Vacant = 5,
}
impl Status {
    pub fn from_raw(v: u8) -> Result<Self, String> {
        match v {
            1 => Ok(Self::Measured),
            2 => Ok(Self::Partial),
            3 => Ok(Self::Failed),
            4 => Ok(Self::NotScheduled),
            5 => Ok(Self::Vacant),
            _ => Err(error("status")),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Cause {
    None = 0,
    Unreadable = 1,
    InvalidHeader = 2,
    InvalidIdentity = 3,
    Changed = 4,
    ClockUnreadable = 5,
    ClockChanged = 6,
    BoundExceeded = 7,
    UnsupportedClass = 8,
    UnresolvedReference = 9,
    DependencyUnavailable = 10,
    ClassMismatch = 11,
    UninitializedReference = 12,
    IdentityConflict = 13,
}
impl Cause {
    pub fn from_raw(v: u8) -> Result<Self, String> {
        use Cause::*;
        Ok(match v {
            0 => None,
            1 => Unreadable,
            2 => InvalidHeader,
            3 => InvalidIdentity,
            4 => Changed,
            5 => ClockUnreadable,
            6 => ClockChanged,
            7 => BoundExceeded,
            8 => UnsupportedClass,
            9 => UnresolvedReference,
            10 => DependencyUnavailable,
            11 => ClassMismatch,
            12 => UninitializedReference,
            13 => IdentityConflict,
            _ => return Err(error("cause")),
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Reference {
    pub class_raw: i32,
    pub type_raw: u32,
    pub id_raw: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotValue {
    Measured(Reference),
    Null,
    Failed {
        cause: Cause,
        class_raw: Option<i32>,
    },
}
impl SlotValue {
    /// Read this witness before resolving an identity in a filtered family.
    /// None is unknown (or a null slot), never an inferred class zero.
    pub fn class_witness(self) -> Option<i32> {
        match self {
            Self::Measured(r) => Some(r.class_raw),
            Self::Failed { class_raw, .. } => class_raw,
            Self::Null => None,
        }
    }
}
fn failed_class_cause(cause: Cause) -> bool {
    matches!(
        cause,
        Cause::Unreadable
            | Cause::BoundExceeded
            | Cause::Changed
            | Cause::UnresolvedReference
            | Cause::ClassMismatch
            | Cause::UninitializedReference
            | Cause::IdentityConflict
    )
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slot {
    pub physical_ordinal: u32,
    pub value: SlotValue,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Vector {
    pub family: Family,
    pub status: Status,
    pub cause: Cause,
    pub declared_count: Option<i32>,
    pub covered_prefix: u32,
    pub slots: Vec<Slot>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Province {
    pub physical_ordinal: u32,
    pub province_id: Option<u32>,
    pub status: Status,
    pub cause: Cause,
    pub vectors: Vec<Vector>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Census {
    pub game_hour_raw: i32,
    pub observation_ordinal: u64,
    pub root_slot_count: Option<u32>,
    pub status: Status,
    pub cause: Cause,
    pub root_covered_prefix: u32,
    pub provinces: Vec<Province>,
}
fn error(s: &str) -> String {
    format!("province_occupants {s}")
}
fn status_cause(status: Status, cause: Cause, vacant: bool) -> Result<(), String> {
    match status {
        Status::Measured if cause == Cause::None => Ok(()),
        Status::Vacant if vacant && cause == Cause::None => Ok(()),
        Status::Partial | Status::Failed | Status::NotScheduled if cause != Cause::None => Ok(()),
        _ => Err(error("status/cause contradiction")),
    }
}
impl Census {
    pub fn failed(hour: i32, ordinal: u64, cause: Cause) -> Self {
        Self {
            game_hour_raw: hour,
            observation_ordinal: ordinal,
            root_slot_count: None,
            status: Status::Failed,
            cause,
            root_covered_prefix: 0,
            provinces: Vec::new(),
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        status_cause(self.status, self.cause, false)?;
        if self.provinces.len() > MAX_PROVINCES
            || self.root_covered_prefix as usize != self.provinces.len()
        {
            return Err(error("root prefix"));
        }
        match self.root_slot_count {
            Some(n) if !(2..=MAX_PROVINCES as u32).contains(&n) || self.root_covered_prefix > n => {
                return Err(error("root count"))
            }
            None if !self.provinces.is_empty() || self.status == Status::Measured => {
                return Err(error("unknown root count"))
            }
            _ => {}
        }
        if self.status == Status::Measured && self.root_slot_count != Some(self.root_covered_prefix)
        {
            return Err(error("incomplete measured root"));
        }
        let mut slots = 0usize;
        for (ordinal, p) in self.provinces.iter().enumerate() {
            if p.physical_ordinal as usize != ordinal {
                return Err(error("province ordinal"));
            }
            status_cause(p.status, p.cause, true)?;
            if let Some(id) = p.province_id {
                if id == 0 || id != p.physical_ordinal {
                    return Err(error("province identity"));
                }
            }
            if p.status == Status::Vacant {
                if p.province_id.is_some() || !p.vectors.is_empty() {
                    return Err(error("vacant province"));
                }
                continue;
            }
            if p.vectors.len() != 4 {
                return Err(error("four independent vectors required"));
            }
            if p.status == Status::Measured && p.province_id.is_none() {
                return Err(error("unwitnessed province"));
            }
            if self.status == Status::Measured && p.status != Status::Measured {
                return Err(error("root hides province refusal"));
            }
            for (family, v) in Family::ALL.into_iter().zip(&p.vectors) {
                if v.family != family {
                    return Err(error("family order"));
                }
                status_cause(v.status, v.cause, false)?;
                if v.covered_prefix as usize != v.slots.len() || v.slots.len() > MAX_SLOTS {
                    return Err(error("vector prefix"));
                }
                slots = slots
                    .checked_add(v.slots.len())
                    .ok_or_else(|| error("slot overflow"))?;
                if slots > MAX_SLOTS {
                    return Err(error("total slot bound"));
                }
                match v.declared_count {
                    Some(n) if n < 0 => return Err(error("negative count")),
                    Some(n) if v.covered_prefix > n as u32 => {
                        return Err(error("prefix exceeds count"))
                    }
                    None if !v.slots.is_empty() || v.status == Status::Measured => {
                        return Err(error("unknown vector count"))
                    }
                    _ => {}
                }
                if v.status == Status::Measured && v.declared_count != Some(v.covered_prefix as i32)
                {
                    return Err(error("incomplete measured vector"));
                }
                if p.status == Status::Measured && v.status != Status::Measured {
                    return Err(error("province hides vector refusal"));
                }
                for (i, slot) in v.slots.iter().enumerate() {
                    if slot.physical_ordinal as usize != i {
                        return Err(error("slot ordinal"));
                    }
                    match slot.value {
                        SlotValue::Measured(r) => {
                            if ![0, 1, 13].contains(&r.class_raw)
                                || family.class().is_some_and(|c| c != r.class_raw)
                                || (r.type_raw == 0 && r.id_raw == 0)
                            {
                                return Err(error("typed reference"));
                            }
                        }
                        SlotValue::Null => {}
                        SlotValue::Failed { cause, class_raw }
                            if cause != Cause::None
                                && v.status != Status::Measured
                                && (class_raw.is_none() || failed_class_cause(cause)) => {}
                        SlotValue::Failed { .. } => return Err(error("slot refusal")),
                    }
                }
            }
        }
        if self.encoded_len()? > MAX_BYTES {
            return Err(error("byte bound"));
        }
        Ok(())
    }
    pub fn encoded_len(&self) -> Result<usize, String> {
        let vectors = self
            .provinces
            .iter()
            .try_fold(0usize, |n, p| n.checked_add(p.vectors.len()))
            .ok_or_else(|| error("length"))?;
        let slots = self
            .provinces
            .iter()
            .flat_map(|p| &p.vectors)
            .try_fold(0usize, |n, v| n.checked_add(v.slots.len()))
            .ok_or_else(|| error("length"))?;
        HEADER_LEN
            .checked_add(
                self.provinces
                    .len()
                    .checked_mul(PROVINCE_LEN)
                    .ok_or_else(|| error("length"))?,
            )
            .and_then(|n| {
                vectors
                    .checked_mul(VECTOR_LEN)
                    .and_then(|v| n.checked_add(v))
            })
            .and_then(|n| slots.checked_mul(SLOT_LEN).and_then(|v| n.checked_add(v)))
            .ok_or_else(|| error("length"))
    }
}
fn put32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}
fn get32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}
fn zero(b: &[u8]) -> Result<(), String> {
    if b.iter().any(|v| *v != 0) {
        Err(error("reserved bytes"))
    } else {
        Ok(())
    }
}
fn option(raw: u32, flag: u8) -> Result<Option<u32>, String> {
    match flag {
        1 => Ok(Some(raw)),
        0 if raw == 0 => Ok(None),
        _ => Err(error("optional presence/filler")),
    }
}
/// Caller must charge the returned allocation before invoking this encoder.
pub fn encode(value: &Census) -> Result<Vec<u8>, String> {
    value.validate()?;
    let np = value.provinces.len();
    let nv = value
        .provinces
        .iter()
        .map(|p| p.vectors.len())
        .sum::<usize>();
    let ns = value
        .provinces
        .iter()
        .flat_map(|p| &p.vectors)
        .map(|v| v.slots.len())
        .sum::<usize>();
    let mut b = vec![0; value.encoded_len()?];
    b[..4].copy_from_slice(MAGIC);
    b[4] = 1;
    b[6] = HEADER_LEN as u8;
    put32(&mut b, 8, value.game_hour_raw as u32);
    put32(&mut b, 12, value.root_slot_count.unwrap_or(0));
    b[16..24].copy_from_slice(&value.observation_ordinal.to_le_bytes());
    put32(&mut b, 24, np as u32);
    put32(&mut b, 28, nv as u32);
    put32(&mut b, 32, ns as u32);
    b[36] = value.status as u8;
    b[37] = value.cause as u8;
    b[38] = u8::from(value.root_slot_count.is_some());
    put32(&mut b, 40, value.root_covered_prefix);
    let mut vi = 0;
    let mut si = 0;
    for (i, p) in value.provinces.iter().enumerate() {
        let a = HEADER_LEN + i * PROVINCE_LEN;
        put32(&mut b, a, p.physical_ordinal);
        put32(&mut b, a + 4, p.province_id.unwrap_or(0));
        b[a + 8] = p.status as u8;
        b[a + 9] = p.cause as u8;
        b[a + 10] = u8::from(p.province_id.is_some());
        put32(&mut b, a + 12, vi as u32);
        put32(&mut b, a + 16, p.vectors.len() as u32);
        for v in &p.vectors {
            let a = HEADER_LEN + np * PROVINCE_LEN + vi * VECTOR_LEN;
            vi += 1;
            b[a] = v.family as u8;
            b[a + 1] = v.status as u8;
            b[a + 2] = v.cause as u8;
            b[a + 3] = u8::from(v.declared_count.is_some());
            put32(&mut b, a + 4, v.declared_count.unwrap_or(0) as u32);
            put32(&mut b, a + 8, v.covered_prefix);
            put32(&mut b, a + 12, si as u32);
            put32(&mut b, a + 16, v.slots.len() as u32);
            for s in &v.slots {
                let a = HEADER_LEN + np * PROVINCE_LEN + nv * VECTOR_LEN + si * SLOT_LEN;
                si += 1;
                put32(&mut b, a, s.physical_ordinal);
                match s.value {
                    SlotValue::Measured(r) => {
                        b[a + 4] = 1;
                        put32(&mut b, a + 8, r.class_raw as u32);
                        put32(&mut b, a + 12, r.type_raw);
                        put32(&mut b, a + 16, r.id_raw);
                    }
                    SlotValue::Null => b[a + 4] = 2,
                    SlotValue::Failed { cause, class_raw } => {
                        b[a + 4] = 3;
                        b[a + 5] = cause as u8;
                        b[a + 6] = u8::from(class_raw.is_some());
                        put32(&mut b, a + 8, class_raw.unwrap_or(0) as u32);
                    }
                }
            }
        }
    }
    Ok(b)
}
pub fn decode(b: &[u8]) -> Result<Census, String> {
    if b.len() < HEADER_LEN || b.len() > MAX_BYTES || &b[..4] != MAGIC || b[4..8] != [1, 0, 48, 0] {
        return Err(error("header/version"));
    }
    zero(&b[39..40])?;
    zero(&b[44..48])?;
    let np = get32(b, 24) as usize;
    let nv = get32(b, 28) as usize;
    let ns = get32(b, 32) as usize;
    if np > MAX_PROVINCES || nv > MAX_PROVINCES * 4 || ns > MAX_SLOTS {
        return Err(error("table bounds"));
    }
    if b.len() != HEADER_LEN + np * PROVINCE_LEN + nv * VECTOR_LEN + ns * SLOT_LEN {
        return Err(error("exact length"));
    }
    let mut out = Census {
        game_hour_raw: get32(b, 8) as i32,
        observation_ordinal: u64::from_le_bytes(b[16..24].try_into().unwrap()),
        root_slot_count: option(get32(b, 12), b[38])?,
        status: Status::from_raw(b[36])?,
        cause: Cause::from_raw(b[37])?,
        root_covered_prefix: get32(b, 40),
        provinces: Vec::with_capacity(np),
    };
    let mut vi = 0usize;
    let mut si = 0usize;
    for i in 0..np {
        let a = HEADER_LEN + i * PROVINCE_LEN;
        zero(&b[a + 11..a + 12])?;
        zero(&b[a + 20..a + 24])?;
        let count = get32(b, a + 16) as usize;
        if get32(b, a + 12) as usize != vi || count > 4 || vi + count > nv {
            return Err(error("vector range"));
        }
        let mut p = Province {
            physical_ordinal: get32(b, a),
            province_id: option(get32(b, a + 4), b[a + 10])?,
            status: Status::from_raw(b[a + 8])?,
            cause: Cause::from_raw(b[a + 9])?,
            vectors: Vec::with_capacity(count),
        };
        for _ in 0..count {
            let a = HEADER_LEN + np * PROVINCE_LEN + vi * VECTOR_LEN;
            vi += 1;
            zero(&b[a + 20..a + 32])?;
            let count = get32(b, a + 16) as usize;
            if get32(b, a + 12) as usize != si || count > MAX_SLOTS || si + count > ns {
                return Err(error("slot range"));
            }
            let mut v = Vector {
                family: Family::from_raw(b[a])?,
                status: Status::from_raw(b[a + 1])?,
                cause: Cause::from_raw(b[a + 2])?,
                declared_count: option(get32(b, a + 4), b[a + 3])?.map(|n| n as i32),
                covered_prefix: get32(b, a + 8),
                slots: Vec::with_capacity(count),
            };
            for _ in 0..count {
                let a = HEADER_LEN + np * PROVINCE_LEN + nv * VECTOR_LEN + si * SLOT_LEN;
                si += 1;
                zero(&b[a + 7..a + 8])?;
                zero(&b[a + 20..a + 24])?;
                let cause = Cause::from_raw(b[a + 5])?;
                let value = match b[a + 4] {
                    1 if cause == Cause::None && b[a + 6] == 0 => SlotValue::Measured(Reference {
                        class_raw: get32(b, a + 8) as i32,
                        type_raw: get32(b, a + 12),
                        id_raw: get32(b, a + 16),
                    }),
                    2 if cause == Cause::None && b[a + 6] == 0 => {
                        zero(&b[a + 8..a + 20])?;
                        SlotValue::Null
                    }
                    3 if cause != Cause::None => {
                        zero(&b[a + 12..a + 20])?;
                        SlotValue::Failed {
                            cause,
                            class_raw: option(get32(b, a + 8), b[a + 6])?.map(|v| v as i32),
                        }
                    }
                    _ => return Err(error("slot tag")),
                };
                v.slots.push(Slot {
                    physical_ordinal: get32(b, a),
                    value,
                });
            }
            p.vectors.push(v);
        }
        out.provinces.push(p);
    }
    if vi != nv || si != ns {
        return Err(error("unowned tail"));
    }
    out.validate()?;
    Ok(out)
}
/// Strict MAPC2 envelope. The returned legacy rows have their original offsets.
pub fn split_map_control(payload: &[u8]) -> Result<(&[u8], Census), String> {
    if payload.len() < MAP_HEADER_LEN + 32 + HEADER_LEN
        || payload[..8] != [b'M', b'A', b'P', b'C', 2, 0, 8, 0]
    {
        return Err(error("MAPC2 header"));
    }
    let rows = &payload[MAP_HEADER_LEN..];
    let states = get32(rows, 8) as usize;
    let provinces = get32(rows, 20) as usize;
    let end = states
        .checked_add(provinces)
        .and_then(|n| n.checked_mul(16))
        .and_then(|n| n.checked_add(32))
        .filter(|n| *n <= rows.len())
        .ok_or_else(|| error("MAPC2 row length"))?;
    Ok((&rows[..end], decode(&rows[end..])?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failed_slots() -> Census {
        let slots = [Some(0), Some(13), None]
            .into_iter()
            .enumerate()
            .map(|(i, class_raw)| Slot {
                physical_ordinal: i as u32,
                value: SlotValue::Failed {
                    cause: Cause::UninitializedReference,
                    class_raw,
                },
            })
            .collect();
        let mut vectors: Vec<_> = Family::ALL
            .into_iter()
            .map(|family| Vector {
                family,
                status: Status::Measured,
                cause: Cause::None,
                declared_count: Some(0),
                covered_prefix: 0,
                slots: Vec::new(),
            })
            .collect();
        vectors[0] = Vector {
            family: Family::Mixed,
            status: Status::Partial,
            cause: Cause::UninitializedReference,
            declared_count: Some(3),
            covered_prefix: 3,
            slots,
        };
        Census {
            game_hour_raw: 7,
            observation_ordinal: 9,
            root_slot_count: Some(2),
            status: Status::Partial,
            cause: Cause::UninitializedReference,
            root_covered_prefix: 2,
            provinces: vec![
                Province {
                    physical_ordinal: 0,
                    province_id: None,
                    status: Status::Vacant,
                    cause: Cause::None,
                    vectors: Vec::new(),
                },
                Province {
                    physical_ordinal: 1,
                    province_id: Some(1),
                    status: Status::Partial,
                    cause: Cause::UninitializedReference,
                    vectors,
                },
            ],
        }
    }

    #[test]
    fn failed_identity_preserves_zero_thirteen_and_unknown_class_before_filtering() {
        let expected = failed_slots();
        let decoded = decode(&encode(&expected).unwrap()).unwrap();
        assert_eq!(decoded, expected);
        let slots = &decoded.provinces[1].vectors[0].slots;
        assert_eq!(
            slots
                .iter()
                .map(|s| s.value.class_witness())
                .collect::<Vec<_>>(),
            vec![Some(0), Some(13), None]
        );
        // Unknown class stays reached/refused; known non-13 alone is skipped.
        let reached: Vec<_> = slots
            .iter()
            .filter(|s| !matches!(s.value.class_witness(), Some(c) if c != 13))
            .map(|s| s.physical_ordinal)
            .collect();
        assert_eq!(reached, vec![1, 2]);
        assert!(reached
            .iter()
            .all(|i| matches!(slots[*i as usize].value, SlotValue::Failed { .. })));
    }

    #[test]
    fn failed_class_presence_filler_cause_and_identity_are_strict() {
        let bytes = encode(&failed_slots()).unwrap();
        let first = HEADER_LEN + 2 * PROVINCE_LEN + 4 * VECTOR_LEN;
        for (offset, value) in [
            (6, 2),
            (7, 1),
            (12, 1),
            (16, 1),
            (20, 1),
            (5, Cause::InvalidHeader as u8),
            (5, Cause::ClockChanged as u8),
            (5, 0),
        ] {
            let mut bad = bytes.clone();
            bad[first + offset] = value;
            assert!(decode(&bad).is_err(), "offset {offset}, value {value}");
        }
        let mut absent_with_class = bytes.clone();
        absent_with_class[first + SLOT_LEN + 6] = 0;
        assert!(decode(&absent_with_class).is_err());
        for status in [1, 2] {
            let mut bad = bytes.clone();
            bad[first + 4] = status;
            bad[first + 5] = 0;
            assert!(decode(&bad).is_err(), "presence bit on status {status}");
        }
        let mut hidden = failed_slots();
        hidden.provinces[1].vectors[0].status = Status::Measured;
        hidden.provinces[1].vectors[0].cause = Cause::None;
        assert!(encode(&hidden).is_err());
    }

    #[test]
    fn nullable_counts_prefixes_ranges_and_reserved_bytes_are_strict() {
        let bytes = encode(&failed_slots()).unwrap();
        let province = HEADER_LEN + PROVINCE_LEN;
        let vector = HEADER_LEN + 2 * PROVINCE_LEN;
        for (offset, value) in [
            (38, 0),
            (39, 1),
            (44, 1),
            (province + 10, 0),
            (province + 11, 1),
            (province + 12, 1),
            (province + 20, 1),
            (vector + 3, 0),
            (vector + 12, 1),
            (vector + 20, 1),
        ] {
            let mut bad = bytes.clone();
            bad[offset] = value;
            assert!(decode(&bad).is_err(), "offset {offset}, value {value}");
        }
        for n in [0, 1, u32::MAX] {
            let mut bad = bytes.clone();
            put32(&mut bad, 12, n);
            assert!(decode(&bad).is_err());
        }
        let mut negative = bytes.clone();
        put32(&mut negative, vector + 4, u32::MAX);
        assert!(decode(&negative).is_err());
        for cut in 0..bytes.len() {
            assert!(decode(&bytes[..cut]).is_err(), "truncated at {cut}");
        }
        let mut trailing = bytes;
        trailing.push(0);
        assert!(decode(&trailing).is_err());
    }

    #[test]
    fn actual_slot_and_province_bounds_are_admitted_exactly_and_never_truncated() {
        let mut c = failed_slots();
        c.status = Status::Measured;
        c.cause = Cause::None;
        c.provinces[1].status = Status::Measured;
        c.provinces[1].cause = Cause::None;
        c.provinces[1].vectors[0].status = Status::Measured;
        c.provinces[1].vectors[0].cause = Cause::None;
        c.provinces[1].vectors[0].slots = (0..MAX_SLOTS)
            .map(|i| Slot {
                physical_ordinal: i as u32,
                value: SlotValue::Null,
            })
            .collect();
        c.provinces[1].vectors[0].declared_count = Some(MAX_SLOTS as i32);
        c.provinces[1].vectors[0].covered_prefix = MAX_SLOTS as u32;
        let bytes = encode(&c).unwrap();
        assert_eq!(decode(&bytes).unwrap(), c);
        c.provinces[1].vectors[0].slots.push(Slot {
            physical_ordinal: MAX_SLOTS as u32,
            value: SlotValue::Null,
        });
        c.provinces[1].vectors[0].declared_count = Some(MAX_SLOTS as i32 + 1);
        c.provinces[1].vectors[0].covered_prefix = MAX_SLOTS as u32 + 1;
        assert!(encode(&c).is_err());
        let mut root = Census {
            game_hour_raw: i32::MIN,
            observation_ordinal: u64::MAX,
            root_slot_count: Some(MAX_PROVINCES as u32),
            status: Status::Measured,
            cause: Cause::None,
            root_covered_prefix: MAX_PROVINCES as u32,
            provinces: (0..MAX_PROVINCES)
                .map(|i| Province {
                    physical_ordinal: i as u32,
                    province_id: None,
                    status: Status::Vacant,
                    cause: Cause::None,
                    vectors: Vec::new(),
                })
                .collect(),
        };
        assert_eq!(decode(&encode(&root).unwrap()).unwrap(), root);
        root.provinces.push(Province {
            physical_ordinal: MAX_PROVINCES as u32,
            province_id: None,
            status: Status::Vacant,
            cause: Cause::None,
            vectors: Vec::new(),
        });
        root.root_slot_count = Some(MAX_PROVINCES as u32 + 1);
        root.root_covered_prefix += 1;
        assert!(encode(&root).is_err());
    }

    #[test]
    fn large_declared_vector_with_bounded_prefix_keeps_count_and_refusal() {
        let mut c = failed_slots();
        c.provinces[1].vectors[0].declared_count = Some(i32::MAX);
        c.provinces[1].vectors[0].cause = Cause::BoundExceeded;
        assert_eq!(decode(&encode(&c).unwrap()).unwrap(), c);
        let v = &mut c.provinces[1].vectors[0];
        v.status = Status::Measured;
        v.cause = Cause::None;
        assert!(
            encode(&c).is_err(),
            "a short prefix never certifies absence or a full vector"
        );
    }

    #[test]
    fn mapc2_requires_version_owned_rows_and_exact_mandatory_leaf() {
        let leaf = encode(&Census::failed(7, 9, Cause::Unreadable)).unwrap();
        let mut encoded = vec![b'M', b'A', b'P', b'C', 2, 0, 8, 0];
        encoded.extend_from_slice(&[0; 32]);
        encoded.extend_from_slice(&leaf);
        let (rows, c) = split_map_control(&encoded).unwrap();
        assert_eq!(rows, &[0; 32]);
        assert_eq!(c.observation_ordinal, 9);
        assert!(
            split_map_control(&encoded[8..]).is_err(),
            "retired unversioned grammar cannot acquire authority"
        );
        let mut old_version = encoded.clone();
        old_version[4] = 1;
        assert!(split_map_control(&old_version).is_err());
        assert!(split_map_control(&encoded[..40]).is_err());
        let mut missing_rows = encoded.clone();
        put32(&mut missing_rows, 8 + 8, 1);
        assert!(split_map_control(&missing_rows).is_err());
        let mut trailing = encoded;
        trailing.push(0);
        assert!(split_map_control(&trailing).is_err());
    }
}
