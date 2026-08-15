//! The write side of domain 68's payload.
//!
//! The producer used to build these bytes itself
//! (`attach_log.rs::compact_session_header_data_payload`), which is what made
//! the four transcriptions four INDEPENDENT statements of the layout rather than
//! one statement and three readings. It now goes through here, so the presence
//! word, the scalar block's order and the token block's shape are generated from
//! [`crate::Field::ALL`] on the write side too.
//!
//! Two producer behaviours are preserved deliberately and are called out where
//! they happen: a present field is what sets its presence bit (an absent one
//! packs a zero word or a zero-length token), and the packer never emits a token
//! this grammar cannot express.

use alloc::vec::Vec;

use crate::{Field, Slot, HEADER_LEN, MAX_TOKEN_LEN, SCALAR_FIELDS, TOKEN_FIELDS};

/// One frame's session header, as the producer has it.
///
/// Every field is `Option`: `Some` means "read this frame" and sets the presence
/// bit, `None` means "not read this frame" and packs the absent encoding. There
/// is no third state, so "a value with no presence bit" and "a presence bit with
/// no value" — the two shapes every reader refuses — are shapes the packer
/// cannot construct.
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

/// The packer was handed a token longer than the grammar can express.
///
/// The producer's own reads are already bounded at [`MAX_TOKEN_LEN`] by
/// `compact_contract::build_profile::DOMAIN68_MAX_TOKEN_LEN` /
/// `DOMAIN68_MAX_VERSION_LEN`, so this is unreachable from a shipping read. It
/// is a `Result` rather than a silent truncation because the old packer DID
/// silently truncate — at `u16::MAX`, four hundred times the ceiling every
/// reader enforces — so the one shape it could emit that no reader accepts was
/// a shape it could not report.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TokenTooLong {
    pub field: Field,
    pub len: usize,
    pub max: usize,
}

impl<'a> SessionHeader<'a> {
    /// This field's token, for a token field; `None` for a scalar field.
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

    /// This field's scalar word, for a scalar field; `None` for a token field.
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

    /// Whether this field is carried this frame.
    fn is_carried(&self, field: Field) -> bool {
        match field.slot() {
            Slot::Scalar { .. } => self.scalar(field).is_some(),
            Slot::Token { .. } => self.token(field).is_some(),
        }
    }

    /// The presence word this header implies.
    ///
    /// Built by folding [`Field::ALL`], so a ninth field is carried here the
    /// moment it exists rather than the moment somebody remembers to add a
    /// ninth `if`.
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

/// Encode a whole d68 DATA payload.
pub fn encode_payload(
    header: &SessionHeader<'_>,
    elapsed_wall_ms: u64,
    status_flags: u32,
) -> Result<Vec<u8>, TokenTooLong> {
    // Refuse before writing a byte, so a refusal never leaves a partial payload.
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
    // The scalar block: five words in wire order, an absent field packing zero.
    for field in SCALAR_FIELDS {
        payload.extend_from_slice(&header.scalar(field).unwrap_or(0).to_le_bytes());
    }
    // The token block: three `u16`-prefixed runs in wire order. A token is
    // written whether or not it is carried; an absent one is a zero prefix, and
    // that is why the readers can require exact consumption without a count.
    for field in TOKEN_FIELDS {
        match header.token(field) {
            Some(token) => {
                // In range by the check above, so this cast cannot wrap.
                let len = token.len() as u16;
                payload.extend_from_slice(&len.to_le_bytes());
                payload.extend_from_slice(token.as_bytes());
            }
            None => payload.extend_from_slice(&0u16.to_le_bytes()),
        }
    }
    Ok(payload)
}
