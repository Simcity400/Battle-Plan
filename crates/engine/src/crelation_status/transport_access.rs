use super::*;

pub(super) struct Candidate<'a> {
    pub country: ccountry::Country,
    pub diplomacy: usize,
    pub matrix: &'a mut Option<Result<Array, &'static str>>,
    pub parties: [u32; 2],
    pub seen: &'a mut [u64],
}

/// Add only the current-subject candidate domain to the original owned pass.
/// A suffix failure invalidates the owner coverage certificate, independently
/// of original RelationLinks/CountryWars availability. No native pointer is
/// retained beyond this immediate borrow. The separate bitmap retires with
/// this owner and preserves first-attempt subject failures on duplicates.
pub(super) fn candidate(
    memory: &impl Memory,
    base: usize,
    countries: &ccountry::Reader<'_, impl Memory>,
    resolver: &mut Resolver,
    candidate: Candidate<'_>,
) -> Result<Option<(u32, usize, u32)>, &'static str> {
    let Candidate {
        country,
        diplomacy,
        matrix,
        parties: [first, second],
        seen,
    } = candidate;
    let mut slot = |raw| {
        if raw == 0 {
            Ok(0)
        } else {
            resolver
                .resolve(countries, raw)
                .map(|country| country.canonical_slot)
        }
    };
    let (a, b) = (slot(first)?, slot(second)?);
    let other = if a == country.canonical_slot {
        b
    } else if b == country.canonical_slot {
        a
    } else {
        return Err("transport subject owned relation excludes its owner");
    };
    // Actual-zero/forward-zero cells have their established independent
    // holders. Self disclosure returns before the subject predicates.
    if other == 0 || other == country.canonical_slot {
        return Ok(None);
    }
    let word = seen
        .get_mut(other as usize / 64)
        .ok_or("transport subject endpoint beyond country slots")?;
    let bit = 1_u64 << (other % 64);
    if *word & bit != 0 {
        return Ok(None);
    }
    *word |= bit;
    let matrix = match *matrix {
        Some(result) => result?,
        None => {
            (*matrix.insert(
                diplomacy
                    .checked_add(cdiplomacy_status::RELATIONS)
                    .and_then(|address| Array::read(memory, address, 8))
                    .ok_or("relation matrix unreadable or invalid"),
            ))?
        }
    };
    let status = matrix
        .element(other, 8)
        .and_then(|address| read::<u64>(memory, address))
        .and_then(|pointer| usize::try_from(pointer).ok())
        .filter(|&pointer| pointer != 0)
        .ok_or("transport subject status missing from matrix")?;
    let at = |offset| {
        status
            .checked_add(offset)
            .ok_or("transport subject status address overflow")
    };
    let expected = base
        .checked_add(VTABLE_RVA)
        .ok_or("transport subject status vtable address overflow")? as u64;
    if read::<u64>(memory, status) != Some(expected) {
        return Err("transport subject status identity unsupported");
    }
    if read::<u64>(memory, at(DIPLOMACY_STATUS)?) != Some(diplomacy as u64) {
        return Err("transport subject status diplomacy owner mismatch");
    }
    let target = read::<u32>(memory, at(TARGET_COUNTRY)?)
        .ok_or("transport subject status target unreadable")?;
    if resolver.resolve(countries, target)?.canonical_slot != other {
        return Err("transport subject status target differs from matrix slot");
    }
    Ok(Some((other, status, target)))
}

/// D11030 then D11080, on the ORIGINAL authenticated status. First raw/alias
/// equality returns before the second endpoint is demanded. No dates or
/// retired filter is substituted for the current +288 pointer.
pub(super) fn capture(
    memory: &impl Memory,
    base: usize,
    status: usize,
    target: u32,
    mut canonical: impl FnMut(u32) -> Result<u32, &'static str>,
) -> Result<ZeroSubjectInputs, &'static str> {
    let at = |pointer: usize, offset: usize| {
        pointer
            .checked_add(offset)
            .ok_or("transport subject address overflow")
    };
    let pointer = read::<u64>(memory, at(status, SUBJECT_RELATION)?)
        .ok_or("transport subject pointer unreadable")?;
    if pointer == 0 {
        return Ok(ZeroSubjectInputs::default());
    }
    let pointer = usize::try_from(pointer).map_err(|_| "transport subject pointer overflow")?;
    let kind = read::<u32>(memory, at(pointer, crelation::TYPE_ID)?)
        .ok_or("transport subject type unreadable")?;
    if kind != csubject_relation::TYPE_VALUE {
        return Ok(ZeroSubjectInputs {
            relation_type: Some(kind),
            parties: None,
        });
    }
    let expected = base
        .checked_add(csubject_relation::VTABLE_RVA)
        .ok_or("transport subject vtable address overflow")? as u64;
    if read::<u64>(memory, pointer) != Some(expected) {
        return Err("transport subject identity unsupported");
    }
    let first = read::<u32>(memory, at(pointer, crelation::FIRST_COUNTRY)?)
        .ok_or("transport subject first endpoint unreadable")?;
    let first_matches =
        first == target || (first != 0 && target != 0 && canonical(first)? == canonical(target)?);
    let second = if first_matches {
        None
    } else {
        Some(
            read::<u32>(memory, at(pointer, crelation::SECOND_COUNTRY)?)
                .ok_or("transport subject second endpoint unreadable")?,
        )
    };
    Ok(ZeroSubjectInputs {
        relation_type: Some(kind),
        parties: Some([Some(first), second]),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::SliceMemory;
    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x400];
        bytes[SUBJECT_RELATION..SUBJECT_RELATION + 8].copy_from_slice(&0x300_u64.to_le_bytes());
        bytes[0x300..0x308].copy_from_slice(&(csubject_relation::VTABLE_RVA as u64).to_le_bytes());
        bytes[0x308..0x30c].copy_from_slice(&csubject_relation::TYPE_VALUE.to_le_bytes());
        bytes
    }
    #[test]
    fn raw_and_alias_first_matches_do_not_read_second_or_dates() {
        let mut bytes = fixture();
        bytes[0x310..0x314].copy_from_slice(&12_u32.to_le_bytes());
        for target in [12, 11] {
            let value = capture(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes[..0x314],
                },
                0,
                0,
                target,
                |_| Ok(1),
            )
            .unwrap();
            assert_eq!(value.parties, Some([Some(12), None]));
        }
    }
    #[test]
    fn null_non_subject_and_second_endpoint_remain_distinct() {
        let mut bytes = fixture();
        bytes[0x310..0x314].copy_from_slice(&7_u32.to_le_bytes());
        bytes[0x314..0x318].copy_from_slice(&11_u32.to_le_bytes());
        bytes[0x349] = 1; // Retired is irrelevant to D11030/D11080.
        assert_eq!(
            capture(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                0,
                11,
                Ok
            )
            .unwrap()
            .parties,
            Some([Some(7), Some(11)])
        );
        bytes[0x308..0x30c].copy_from_slice(&0x1234_u32.to_le_bytes());
        assert_eq!(
            capture(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes[..0x30c]
                },
                0,
                0,
                11,
                Ok
            ),
            Ok(ZeroSubjectInputs {
                relation_type: Some(0x1234),
                parties: None
            })
        );
        bytes[SUBJECT_RELATION..SUBJECT_RELATION + 8].fill(0);
        assert_eq!(
            capture(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                0,
                11,
                Ok
            ),
            Ok(ZeroSubjectInputs::default())
        );
    }
}
