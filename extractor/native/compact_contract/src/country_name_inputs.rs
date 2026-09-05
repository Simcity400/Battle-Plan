pub const DOMAIN_VERSION_HEADER_LEN: usize = 8;
pub const POLITICS_VERSION: u16 = 1;
pub const IDENTITY_VERSION: u16 = 2;

fn domain_version(domain: u16) -> u16 {
    match domain { 14 => POLITICS_VERSION, 18 => IDENTITY_VERSION, _ => unreachable!("validated naming domain") }
}

pub fn encode_domain_version(domain: u16, out: &mut Vec<u8>) {
    let magic = match domain { 14 => b"CPNI", 18 => b"CIDN", _ => panic!("unsupported naming domain") };
    out.extend_from_slice(magic);
    out.extend_from_slice(&domain_version(domain).to_le_bytes());
    out.extend_from_slice(&(DOMAIN_VERSION_HEADER_LEN as u16).to_le_bytes());
}

pub fn decode_domain_version(domain: u16, bytes: &[u8], cursor: &mut usize) -> Result<(), &'static str> {
    let (magic, version_error) = match domain {
        14 => (b"CPNI", "politics unsupported payload version"),
        18 => (b"CIDN", "country_identity unsupported payload version"),
        _ => return Err("unsupported naming domain"),
    };
    let end = cursor.checked_add(DOMAIN_VERSION_HEADER_LEN).ok_or(version_error)?;
    let header = bytes.get(*cursor..end).ok_or(version_error)?;
    if &header[..4] != magic || header[4..6] != domain_version(domain).to_le_bytes() { return Err(version_error); }
    if header[6..8] != (DOMAIN_VERSION_HEADER_LEN as u16).to_le_bytes() { return Err("country naming invalid version header length"); }
    *cursor = end;
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum NameInputRefusal {
    Unreadable = 1,
    InvalidIdentity = 2,
    InvalidVector = 3,
    Changed = 4,
    ClockChanged = 5,
    InvalidUtf8 = 6,
    OutOfBounds = 7,
}

impl NameInputRefusal {
    pub fn name(self) -> &'static str {
        match self {
            Self::Unreadable => "unreadable",
            Self::InvalidIdentity => "invalid_identity",
            Self::InvalidVector => "invalid_vector",
            Self::Changed => "changed",
            Self::ClockChanged => "clock_changed",
            Self::InvalidUtf8 => "invalid_utf8",
            Self::OutOfBounds => "out_of_bounds",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NameToken {
    Unavailable(NameInputRefusal),
    Absent,
    Present(String),
}

pub const MAX_NAME_TOKEN_BYTES: usize = 4096;

pub fn encode_token(value: &NameToken, out: &mut Vec<u8>) {
    match value {
        NameToken::Unavailable(reason) => out.extend_from_slice(&[0, *reason as u8]),
        NameToken::Absent => out.push(1),
        NameToken::Present(token) => {
            assert!(token.len() <= MAX_NAME_TOKEN_BYTES);
            out.push(2);
            out.extend_from_slice(&(token.len() as u16).to_le_bytes());
            out.extend_from_slice(token.as_bytes());
        }
    }
}

fn take<'a>(bytes: &'a [u8], cursor: &mut usize, len: usize) -> Result<&'a [u8], &'static str> {
    let end = cursor.checked_add(len).ok_or("country name input overflow")?;
    let result = bytes.get(*cursor..end).ok_or("country name input truncated")?;
    *cursor = end;
    Ok(result)
}

pub fn decode_token(bytes: &[u8], cursor: &mut usize) -> Result<NameToken, &'static str> {
    Ok(match take(bytes, cursor, 1)?[0] {
        0 => NameToken::Unavailable(match take(bytes, cursor, 1)?[0] {
            1 => NameInputRefusal::Unreadable,
            2 => NameInputRefusal::InvalidIdentity,
            3 => NameInputRefusal::InvalidVector,
            4 => NameInputRefusal::Changed,
            5 => NameInputRefusal::ClockChanged,
            6 => NameInputRefusal::InvalidUtf8,
            7 => NameInputRefusal::OutOfBounds,
            _ => return Err("country name input invalid refusal"),
        }),
        1 => NameToken::Absent,
        2 => {
            let raw = take(bytes, cursor, 2)?;
            let len = u16::from_le_bytes([raw[0], raw[1]]) as usize;
            if len > MAX_NAME_TOKEN_BYTES { return Err("country name token exceeds bound"); }
            let value = std::str::from_utf8(take(bytes, cursor, len)?)
                .map_err(|_| "country name token invalid UTF-8")?;
            NameToken::Present(value.to_owned())
        }
        _ => return Err("country name input invalid marker"),
    })
}

pub fn decode_graphical_culture(bytes: &[u8], cursor: &mut usize) -> Result<NameToken, &'static str> {
    match decode_token(bytes, cursor)? {
        NameToken::Absent => Err("country_identity graphical culture cannot claim absent storage"),
        value => Ok(value),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameTagSource { Base, Override }

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectiveNameTag {
    pub source: NameTagSource,
    pub selected_id: i32,
    pub tag: String,
}

pub fn select_name_tag(base: i32, override_id: i32, mapped_base: i32, dynamic_boundary: i32) -> (NameTagSource, i32) {
    if override_id > 0 && dynamic_boundary <= mapped_base {
        (NameTagSource::Override, override_id)
    } else {
        (NameTagSource::Base, base)
    }
}

pub fn encode_effective(value: &Result<EffectiveNameTag, NameInputRefusal>, out: &mut Vec<u8>) {
    match value {
        Err(reason) => encode_token(&NameToken::Unavailable(*reason), out),
        Ok(value) => {
            out.push(match value.source { NameTagSource::Base => 1, NameTagSource::Override => 2 });
            out.extend_from_slice(&value.selected_id.to_le_bytes());
            encode_token(&NameToken::Present(value.tag.clone()), out);
        }
    }
}

pub fn decode_effective(
    bytes: &[u8],
    cursor: &mut usize,
    country_id: u32,
    country_tag: &str,
) -> Result<Result<EffectiveNameTag, NameInputRefusal>, &'static str> {
    let marker = *bytes.get(*cursor).ok_or("country name selection truncated")?;
    if marker == 0 {
        return match decode_token(bytes, cursor)? {
            NameToken::Unavailable(reason) => Ok(Err(reason)),
            _ => Err("country name selection invalid refusal"),
        };
    }
    *cursor += 1;
    let source = match marker { 1 => NameTagSource::Base, 2 => NameTagSource::Override, _ => return Err("country name selection invalid source") };
    let raw: [u8; 4] = take(bytes, cursor, 4)?.try_into().unwrap();
    let selected_id = i32::from_le_bytes(raw);
    if selected_id < 0 || (source == NameTagSource::Override && selected_id == 0) {
        return Err("country name selection invalid id");
    }
    let NameToken::Present(tag) = decode_token(bytes, cursor)? else { return Err("country name selection missing tag"); };
    if source == NameTagSource::Base
        && (selected_id as u32 != country_id || tag != country_tag)
    {
        return Err("country name base selection identity mismatch");
    }
    Ok(Ok(EffectiveNameTag { source, selected_id, tag }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn domain_versions_refuse_unversioned_and_unknown_payloads_by_name() {
        for (domain, error) in [(14,"politics unsupported payload version"),(18,"country_identity unsupported payload version")] {
            assert_eq!(decode_domain_version(domain, &[1,0,0,0,3,0,b'G',b'E'], &mut 0), Err(error));
            let mut bytes=Vec::new(); encode_domain_version(domain,&mut bytes);
            let mut cursor=0; assert_eq!(decode_domain_version(domain,&bytes,&mut cursor),Ok(())); assert_eq!(cursor,8);
            bytes[4]=0; assert_eq!(decode_domain_version(domain,&bytes,&mut 0),Err(error));
            bytes[4]=domain_version(domain) as u8; bytes[6]=9; assert!(decode_domain_version(domain,&bytes,&mut 0).is_err());
        }
    }

    #[test]
    fn identity_culture_promotes_only_its_domain_grammar() {
        let mut bytes = Vec::new();
        encode_domain_version(18, &mut bytes);
        assert_eq!(&bytes, b"CIDN\x02\0\x08\0");
        bytes[4] = 1;
        assert!(decode_domain_version(18, &bytes, &mut 0).is_err());
        bytes.clear();
        encode_domain_version(14, &mut bytes);
        assert_eq!(&bytes, b"CPNI\x01\0\x08\0");
    }

    #[test]
    fn absence_empty_and_unavailable_have_distinct_wire_values() {
        let cases = [NameToken::Absent, NameToken::Present(String::new()), NameToken::Present("nazism".into()), NameToken::Unavailable(NameInputRefusal::Changed)];
        let mut wires = Vec::new();
        for value in cases {
            let mut wire = Vec::new(); encode_token(&value, &mut wire);
            let mut cursor = 0;
            assert_eq!(decode_token(&wire, &mut cursor), Ok(value));
            assert_eq!(cursor, wire.len());
            for end in 0..wire.len() { assert!(decode_token(&wire[..end], &mut 0).is_err()); }
            assert!(!wires.contains(&wire)); wires.push(wire);
        }
        assert!(decode_token(&[0,0], &mut 0).is_err());
        assert!(decode_token(&[2,1,0,255], &mut 0).is_err());
    }

    #[test]
    fn effective_wire_requires_a_measured_bounded_tag_and_known_source() {
        for value in [Err(NameInputRefusal::ClockChanged), Ok(EffectiveNameTag { source:NameTagSource::Override, selected_id:9, tag:String::new() })] {
            let mut wire = Vec::new(); encode_effective(&value, &mut wire);
            let mut cursor = 0;
            assert_eq!(decode_effective(&wire, &mut cursor, 47, "GER"), Ok(value));
            assert_eq!(cursor, wire.len());
            for end in 0..wire.len() { assert!(decode_effective(&wire[..end], &mut 0, 47, "GER").is_err()); }
        }
        for wire in [vec![3], vec![2,0,0,0,0,2,0,0], vec![1,1,0,0,0,1], vec![1,1,0,0,0,0,1]] {
            assert!(decode_effective(&wire, &mut 0, 47, "GER").is_err());
        }
        let mut oversized = vec![2]; oversized.extend_from_slice(&4097u16.to_le_bytes()); oversized.resize(4100, b'a');
        assert!(decode_token(&oversized, &mut 0).is_err());
    }

    #[test]
    fn effective_base_wire_requires_containing_country_id_and_exact_tag() {
        for (source, selected_id, tag, admitted) in [
            (1u8, 47i32, "GER", true),
            (1, 2, "GER", false),
            (1, 47, "ENG", false),
            (1, 2, "ENG", false),
            (1, 47, "ger", false),
            (1, 47, "", false),
            (2, 2, "ENG", true),
            (2, 2, "", true),
        ] {
            let mut wire = vec![source];
            wire.extend_from_slice(&selected_id.to_le_bytes());
            wire.push(2);
            wire.extend_from_slice(&(tag.len() as u16).to_le_bytes());
            wire.extend_from_slice(tag.as_bytes());
            let mut cursor = 0;
            let result = decode_effective(&wire, &mut cursor, 47, "GER");
            if admitted {
                assert_eq!(result, Ok(Ok(EffectiveNameTag {
                    source: if source == 1 { NameTagSource::Base } else { NameTagSource::Override },
                    selected_id,
                    tag: tag.to_owned(),
                })));
                assert_eq!(cursor, wire.len());
            } else {
                assert_eq!(result, Err("country name base selection identity mismatch"), "{selected_id}/{tag:?}");
            }
        }
    }

    #[test]
    fn selection_uses_signed_mapped_base_boundary_and_positive_override() {
        assert_eq!(select_name_tag(5, 9, 100, 100), (NameTagSource::Override, 9));
        assert_eq!(select_name_tag(5, 9, 99, 100), (NameTagSource::Base, 5));
        assert_eq!(select_name_tag(5, 9, -1, 100), (NameTagSource::Base, 5));
        assert_eq!(select_name_tag(5, 0, 101, 100), (NameTagSource::Base, 5));
        assert_eq!(select_name_tag(5, -1, 101, 100), (NameTagSource::Base, 5));
        assert_eq!(select_name_tag(0, 9, 0, 0), (NameTagSource::Override, 9));
    }
}
