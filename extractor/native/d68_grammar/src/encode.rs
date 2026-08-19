//! word, the scalar block's order and the token block's shape are generated from

use alloc::vec::Vec;

use crate::{Field, Slot, HEADER_LEN, MAX_TOKEN_LEN, SCALAR_FIELDS, TOKEN_FIELDS};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionHeader<'a> {
    pub player_tag_db_id: Option<u32>,
    pub dlcs: Option<u32>,
    pub start_date_raw: Option<u32>,
    pub save_version: Option<u32>,
    pub session: Option<u32>,
    pub ideology: Option<&'a str>,
    pub cosmetic_tag: Option<&'a str>,
    pub version: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TokenTooLong {
    pub field: Field,
    pub len: usize,
    pub max: usize,
}

impl<'a> SessionHeader<'a> {
    fn token(&self, field: Field) -> Option<&'a str> {
        match field {
            Field::Ideology => self.ideology,
            Field::CosmeticTag => self.cosmetic_tag,
            Field::Version => self.version,
            Field::Player
            | Field::Dlcs
            | Field::StartDate
            | Field::SaveVersion
            | Field::Session => None,
        }
    }

    fn scalar(&self, field: Field) -> Option<u32> {
        match field {
            Field::Player => self.player_tag_db_id,
            Field::Dlcs => self.dlcs,
            Field::StartDate => self.start_date_raw,
            Field::SaveVersion => self.save_version,
            Field::Session => self.session,
            Field::Ideology | Field::CosmeticTag | Field::Version => None,
        }
    }

    fn is_carried(&self, field: Field) -> bool {
        match field.slot() {
            Slot::Scalar { .. } => self.scalar(field).is_some(),
            Slot::Token { .. } => self.token(field).is_some(),
        }
    }

    pub fn presence(&self) -> u32 {
        let mut presence = 0u32;
        for field in Field::ALL {
            if self.is_carried(field) {
                presence |= field.flag();
            }
        }
        presence
    }
}

pub fn encode_payload(
    header: &SessionHeader<'_>,
    elapsed_wall_ms: u64,
    status_flags: u32,
) -> Result<Vec<u8>, TokenTooLong> {
    for field in TOKEN_FIELDS {
        if let Some(token) = header.token(field) {
            if token.len() > MAX_TOKEN_LEN {
                return Err(TokenTooLong {
                    field,
                    len: token.len(),
                    max: MAX_TOKEN_LEN,
                });
            }
        }
    }

    let mut payload = Vec::with_capacity(HEADER_LEN + SCALAR_FIELDS.len() * 4 + 48);
    payload.extend_from_slice(&elapsed_wall_ms.to_le_bytes());
    payload.extend_from_slice(&header.presence().to_le_bytes());
    payload.extend_from_slice(&status_flags.to_le_bytes());
    for field in SCALAR_FIELDS {
        payload.extend_from_slice(&header.scalar(field).unwrap_or(0).to_le_bytes());
    }
    for field in TOKEN_FIELDS {
        match header.token(field) {
            Some(token) => {
                let len = token.len() as u16;
                payload.extend_from_slice(&len.to_le_bytes());
                payload.extend_from_slice(token.as_bytes());
            }
            None => payload.extend_from_slice(&0u16.to_le_bytes()),
        }
    }
    Ok(payload)
}
