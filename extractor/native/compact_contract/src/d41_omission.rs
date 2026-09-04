//! Domain 41's declaration of WHICH pinned rows a DATA record did not carry.
//!
//! The declaration IS the payload spelling of `COMPACT_STATUS_PARTIAL_DATA` for
//! this domain, in the shape domain 53 uses. The decoder refuses a trailer that
//! names nothing, a declared total that is not the lists' length, an
//! out-of-canonical-order list, and an identity no class could produce; the
//! caller refuses a declaration without the badge. A d41 partial with no
//! declaration is undeclared and stays a refused hour.

pub const D41_OMISSION_TRAILER_MAGIC: &[u8; 4] = b"D41O";
pub const D41_OMISSION_TRAILER_VERSION: u16 = 1;
pub const D41_OMISSION_TRAILER_HEADER_LEN: usize = 16;

/// The pin sets both walks iterate are capped well below this by the collector's
/// own `MAX_PROJECT_FACILITIES` / `MAX_PROJECT_SCIENTISTS`, so a longer list is
/// a payload no producer of this contract can build.
pub const D41_MAX_OMITTED_IDENTITIES: usize = 65_536;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Domain41OmissionClass {
    Facility,
    Scientist,
}

impl Domain41OmissionClass {
    pub const ALL: [Self; 2] = [Self::Facility, Self::Scientist];

    pub const fn ordinal(self) -> usize {
        match self {
            Self::Facility => 0,
            Self::Scientist => 1,
        }
    }

    pub const fn unit_name(self) -> &'static str {
        match self {
            Self::Facility => "d41_declared_omission_facility",
            Self::Scientist => "d41_declared_omission_scientist",
        }
    }

    /// Is `identity` a key this class could ever have produced?
    ///
    /// An identity that names no row is not a declaration: the tolerance would
    /// clear `PARTIAL_DATA` while the fold removed nothing.
    pub const fn identity_is_well_formed(self, identity: (u32, u32)) -> bool {
        let (left, right) = identity;
        match self {
            // The pinned facility id and the province the same pin carried.
            // The fold refuses a facility row with id 0, so 0 names no row; a
            // province of 0 is a pin whose province field the walk did read as
            // zero and is not by itself inadmissible.
            Self::Facility => left != 0,
            // The pinned character id. The right half is unused and must be
            // zero so a later reader cannot mistake padding for a second key.
            Self::Scientist => left != 0 && right == 0,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Domain41OmissionDeclaration {
    identities: [Vec<(u32, u32)>; Domain41OmissionClass::ALL.len()],
}

impl Domain41OmissionDeclaration {
    pub fn identities(&self, class: Domain41OmissionClass) -> &[(u32, u32)] {
        &self.identities[class.ordinal()]
    }

    pub fn count(&self, class: Domain41OmissionClass) -> usize {
        self.identities[class.ordinal()].len()
    }

    /// Record one omitted row. Duplicates are kept: two pins that read the same
    /// id are two refusals, and collapsing them would understate the omission.
    pub fn declare(&mut self, class: Domain41OmissionClass, identity: (u32, u32)) {
        self.identities[class.ordinal()].push(identity);
    }

    /// Canonical order for the wire, so the decoder can reject a list an
    /// encoder of this contract could not have assembled.
    pub fn canonicalize(&mut self) {
        for list in &mut self.identities {
            list.sort_unstable();
        }
    }

    pub fn total(&self) -> usize {
        self.identities.iter().map(Vec::len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.total() == 0
    }
}

pub fn d41_omission_trailer_len(declaration: &Domain41OmissionDeclaration) -> usize {
    if declaration.is_empty() {
        return 0;
    }
    D41_OMISSION_TRAILER_HEADER_LEN
        .saturating_add(Domain41OmissionClass::ALL.len().saturating_mul(4))
        .saturating_add(declaration.total().saturating_mul(8))
}

pub fn append_d41_omission_trailer(
    payload: &mut Vec<u8>,
    omissions: Option<&Domain41OmissionDeclaration>,
) {
    let Some(declaration) = omissions.filter(|declaration| !declaration.is_empty()) else {
        return;
    };
    payload.extend_from_slice(D41_OMISSION_TRAILER_MAGIC);
    payload.extend_from_slice(&D41_OMISSION_TRAILER_VERSION.to_le_bytes());
    payload.extend_from_slice(&(D41_OMISSION_TRAILER_HEADER_LEN as u16).to_le_bytes());
    payload.extend_from_slice(&(Domain41OmissionClass::ALL.len() as u32).to_le_bytes());
    payload
        .extend_from_slice(&(declaration.total().min(u32::MAX as usize) as u32).to_le_bytes());
    for class in Domain41OmissionClass::ALL {
        let identities = declaration.identities(class);
        payload.extend_from_slice(&(identities.len().min(u32::MAX as usize) as u32).to_le_bytes());
        for (left, right) in identities {
            payload.extend_from_slice(&left.to_le_bytes());
            payload.extend_from_slice(&right.to_le_bytes());
        }
    }
}

fn read_u16_le(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_u32_le(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

/// Decode the trailer at `trailer_offset`, returning the declaration and the
/// byte length it consumed.
pub fn decode_d41_omission_trailer(
    payload: &[u8],
    trailer_offset: usize,
) -> Result<(Domain41OmissionDeclaration, usize), String> {
    let trailer = payload
        .get(trailer_offset..)
        .ok_or_else(|| "d41 omission trailer offset is beyond the payload".to_string())?;
    if trailer.len() < D41_OMISSION_TRAILER_HEADER_LEN {
        return Err(format!(
            "d41 omission trailer is {} byte(s), expected at least {}",
            trailer.len(),
            D41_OMISSION_TRAILER_HEADER_LEN
        ));
    }
    if &trailer[0..4] != D41_OMISSION_TRAILER_MAGIC {
        return Err("d41 omission trailer magic is invalid".to_string());
    }
    let version = read_u16_le(trailer, 4);
    let header_len = read_u16_le(trailer, 6) as usize;
    if version != D41_OMISSION_TRAILER_VERSION || header_len != D41_OMISSION_TRAILER_HEADER_LEN {
        return Err(format!(
            "d41 omission trailer header shape invalid (version {version}, header_len {header_len})"
        ));
    }
    let class_count = read_u32_le(trailer, 8) as usize;
    if class_count != Domain41OmissionClass::ALL.len() {
        return Err(format!(
            "d41 omission trailer declares {class_count} class(es), expected {}",
            Domain41OmissionClass::ALL.len()
        ));
    }
    let declared_total = read_u32_le(trailer, 12) as usize;
    let mut declaration = Domain41OmissionDeclaration::default();
    let mut cursor = D41_OMISSION_TRAILER_HEADER_LEN;
    let mut total = 0usize;
    for class in Domain41OmissionClass::ALL {
        if trailer.len().saturating_sub(cursor) < 4 {
            return Err(format!(
                "d41 omission trailer class length truncated at byte {cursor}"
            ));
        }
        let len = read_u32_le(trailer, cursor) as usize;
        cursor += 4;
        total = total.saturating_add(len);
        if total > D41_MAX_OMITTED_IDENTITIES {
            return Err(format!(
                "d41 omission trailer declares more than {D41_MAX_OMITTED_IDENTITIES} identities"
            ));
        }
        let Some(body) = len
            .checked_mul(8)
            .filter(|body| *body <= trailer.len() - cursor)
        else {
            return Err(format!(
                "d41 omission trailer body for {} is truncated at byte {cursor}",
                class.unit_name()
            ));
        };
        let mut previous: Option<(u32, u32)> = None;
        for index in 0..len {
            let at = cursor + index * 8;
            let identity = (read_u32_le(trailer, at), read_u32_le(trailer, at + 4));
            if previous.is_some_and(|earlier| earlier > identity) {
                return Err(format!(
                    "d41 omission identities for {} are not sorted",
                    class.unit_name()
                ));
            }
            if !class.identity_is_well_formed(identity) {
                return Err(format!(
                    "d41 omission identity {identity:?} for {} is not a key that class can produce",
                    class.unit_name()
                ));
            }
            previous = Some(identity);
            declaration.declare(class, identity);
        }
        cursor += body;
    }
    if declared_total != total {
        return Err(format!(
            "d41 omission trailer declares {declared_total} omitted row(s) over lists of {total}"
        ));
    }
    if declaration.is_empty() {
        return Err("d41 omission trailer declares no omitted rows".to_string());
    }
    Ok((declaration, cursor))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Domain41OmissionDeclaration {
        let mut declaration = Domain41OmissionDeclaration::default();
        declaration.declare(Domain41OmissionClass::Facility, (91, 4004));
        declaration.declare(Domain41OmissionClass::Facility, (17, 1201));
        declaration.declare(Domain41OmissionClass::Scientist, (5309, 0));
        declaration.canonicalize();
        declaration
    }

    fn encoded(declaration: &Domain41OmissionDeclaration) -> Vec<u8> {
        let mut payload = Vec::new();
        append_d41_omission_trailer(&mut payload, Some(declaration));
        payload
    }

    #[test]
    fn the_trailer_round_trips_every_class_by_identity() {
        let declaration = fixture();
        let payload = encoded(&declaration);
        assert_eq!(payload.len(), d41_omission_trailer_len(&declaration));
        let (decoded, consumed) = decode_d41_omission_trailer(&payload, 0).expect("decode");
        assert_eq!(consumed, payload.len());
        assert_eq!(decoded, declaration);
        assert_eq!(
            decoded.identities(Domain41OmissionClass::Facility),
            &[(17, 1201), (91, 4004)]
        );
        assert_eq!(
            decoded.identities(Domain41OmissionClass::Scientist),
            &[(5309, 0)]
        );
        assert_eq!(decoded.total(), 3);
    }

    #[test]
    fn an_empty_declaration_writes_no_trailer() {
        let declaration = Domain41OmissionDeclaration::default();
        assert_eq!(d41_omission_trailer_len(&declaration), 0);
        assert!(encoded(&declaration).is_empty());
    }

    #[test]
    fn a_trailer_that_names_nothing_is_refused() {
        let mut payload = Vec::new();
        payload.extend_from_slice(D41_OMISSION_TRAILER_MAGIC);
        payload.extend_from_slice(&D41_OMISSION_TRAILER_VERSION.to_le_bytes());
        payload.extend_from_slice(&(D41_OMISSION_TRAILER_HEADER_LEN as u16).to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        let error = decode_d41_omission_trailer(&payload, 0).expect_err("refused");
        assert!(error.contains("declares no omitted rows"), "{error}");
    }

    #[test]
    fn a_declared_total_that_is_not_the_lists_length_is_refused() {
        let declaration = fixture();
        let mut payload = encoded(&declaration);
        payload[12..16].copy_from_slice(&9u32.to_le_bytes());
        let error = decode_d41_omission_trailer(&payload, 0).expect_err("refused");
        assert!(error.contains("declares 9 omitted row(s) over lists of 3"), "{error}");
    }

    #[test]
    fn a_list_out_of_canonical_order_is_refused() {
        let declaration = fixture();
        let mut payload = encoded(&declaration);
        let first = D41_OMISSION_TRAILER_HEADER_LEN + 4;
        let (left, right) = (
            payload[first..first + 8].to_vec(),
            payload[first + 8..first + 16].to_vec(),
        );
        payload[first..first + 8].copy_from_slice(&right);
        payload[first + 8..first + 16].copy_from_slice(&left);
        let error = decode_d41_omission_trailer(&payload, 0).expect_err("refused");
        assert!(error.contains("are not sorted"), "{error}");
    }

    #[test]
    fn an_identity_no_class_could_produce_is_refused() {
        let mut zero_facility = Domain41OmissionDeclaration::default();
        zero_facility.declare(Domain41OmissionClass::Facility, (0, 12));
        let error = decode_d41_omission_trailer(&encoded(&zero_facility), 0).expect_err("refused");
        assert!(error.contains("is not a key that class can produce"), "{error}");

        let mut padded_scientist = Domain41OmissionDeclaration::default();
        padded_scientist.declare(Domain41OmissionClass::Scientist, (44, 7));
        let error =
            decode_d41_omission_trailer(&encoded(&padded_scientist), 0).expect_err("refused");
        assert!(error.contains("is not a key that class can produce"), "{error}");
    }

    #[test]
    fn a_wrong_class_count_and_a_truncated_body_are_refused() {
        let declaration = fixture();
        let mut payload = encoded(&declaration);
        payload[8..12].copy_from_slice(&3u32.to_le_bytes());
        let error = decode_d41_omission_trailer(&payload, 0).expect_err("refused");
        assert!(error.contains("declares 3 class(es)"), "{error}");

        let payload = encoded(&declaration);
        let error = decode_d41_omission_trailer(&payload[..payload.len() - 4], 0)
            .expect_err("refused");
        assert!(error.contains("is truncated"), "{error}");
    }
}
